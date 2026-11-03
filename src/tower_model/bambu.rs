//! Column size in BambuStudio (spec 04 §4.3): depth is reserved per tool change from the
//! incoming filament's prime volume (flushes go to the chute); changes into one adhesion
//! category stack in one block whose depth is its deepest layer; a rib wall squares the tower
//! off once and re-plans.
//!
//! Every change is sized with the largest prime volume of the project's filaments. The slicer
//! resolves the per-filament value from the filament presets, which can differ from the
//! project's own list: the A2L goat project stores 45, 45, 30, 30 mm³, yet its CLI slices echo
//! 45 for all four, and only 45 + 45 per two-change layer reproduces the printed 24.5 mm box
//! (the goat's two-change layers go into filaments 3 and 1 or 3 and 2).

use std::collections::BTreeMap;

use super::formulas::{bead_area, min_depth_for_height};
use super::schedule::{ToolLayer, ToolSchedule};
use super::settings::TowerSettings;

/// The planned column: the box the rows fill and, for rib walls, the diagonal ribs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Column {
    /// W × D of the box, the wall line included (D = block depth + one line).
    pub box_size: (f64, f64),
    pub rib: Option<Rib>,
}

/// Diagonal ribs of a rib wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rib {
    /// Full width r of a rib strip.
    pub width: f64,
    /// Length L_r along the diagonal at the first layer; never shorter than the diagonal.
    pub length: f64,
}

pub fn bambu_column(schedule: &ToolSchedule, settings: &TowerSettings, tower_top: f64) -> Column {
    let pw = settings.line_width;
    let layers = schedule.tower_layers();
    let first_plan = block_depth(layers, settings, settings.width);
    let (width, blocks) = if settings.rib_wall {
        let squared = ceil_to_multiple(libm::sqrt((first_plan + pw) * settings.width), pw);
        (squared, block_depth(layers, settings, squared))
    } else {
        (settings.width, first_plan)
    };
    let min_depth = min_depth_for_height(tower_top);
    if !settings.rib_wall {
        // Without ribs a shallow tower spreads its rows to the minimum depth.
        return Column {
            box_size: (width, blocks.max(min_depth) + pw),
            rib: None,
        };
    }
    let depth = blocks + pw;
    let diagonal = libm::hypot(width, depth);
    // A shallow rib tower grows its ribs, not its box, to the minimum depth.
    let shallow = if blocks < min_depth {
        min_depth * std::f64::consts::SQRT_2
    } else {
        0.0
    };
    let length = (diagonal.max(shallow) + settings.extra_rib_length).max(diagonal);
    Column {
        box_size: (width, depth),
        rib: Some(Rib {
            width: settings.rib_width.min(width.min(depth) / 2.0),
            length,
        }),
    }
}

/// Σ over adhesion categories of the category's deepest layer, for a tower of `width`. Which
/// filaments a layer's changes go into depends on the flush order, which is not modelled, so
/// each category takes as many of the layer's changes as it has filaments there.
fn block_depth(layers: &[ToolLayer], settings: &TowerSettings, width: f64) -> f64 {
    let mut deepest: BTreeMap<&str, f64> = BTreeMap::new();
    for layer in layers {
        let mut members: BTreeMap<&str, usize> = BTreeMap::new();
        for filament in &layer.filaments {
            *members.entry(settings.category_of(*filament)).or_default() += 1;
        }
        let rows = rows_for(settings.largest_prime_volume(), layer, settings, width);
        for (category, count) in members {
            let depth =
                count.min(layer.changes) as f64 * rows * settings.infill_gap * settings.line_width;
            let entry = deepest.entry(category).or_default();
            *entry = entry.max(depth);
        }
    }
    deepest.values().sum()
}

/// Purge rows across the tower's inner width (between the two wall lines).
fn rows_for(volume: f64, layer: &ToolLayer, settings: &TowerSettings, width: f64) -> f64 {
    let pw = settings.line_width;
    let inner = width - 2.0 * pw;
    if inner <= 0.0 {
        return 0.0;
    }
    let length = volume / bead_area(pw, layer.z_top - layer.z_bottom);
    (length / inner).ceil()
}

fn ceil_to_multiple(value: f64, step: f64) -> f64 {
    (value / step).ceil() * step
}

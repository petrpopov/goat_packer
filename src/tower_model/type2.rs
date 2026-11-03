//! Column depth in the Type 2 tower of OrcaSlicer 2.4 (O), 2.5 (R) and Snapmaker Orca 2.4 (S)
//! (spec 04 §4.4): one `prime_volume` purge per change, planned as a ramming band plus wipe
//! rows; the layer depth is the sum over its changes and the tower takes its deepest layer.
//! The width stays `prime_tower_width` (rectangular walls only; others are refused).

use super::bambu::Column;
use super::formulas::bead_area;
use super::schedule::{ToolLayer, ToolSchedule};
use super::settings::{TowerSettings, largest_of};
use crate::first_layer::SlicerBuild;

/// O plans the multi-tool ram as a quarter second at the ramming flow.
const O_RAMMING_SECONDS: f64 = 0.25;

pub fn type2_column(
    schedule: &ToolSchedule,
    settings: &TowerSettings,
    build: SlicerBuild,
) -> Column {
    let layers = schedule.tower_layers();
    let deepest = layers
        .iter()
        .enumerate()
        .map(|(index, layer)| layer_depth(layer, index == 0, settings, build))
        .fold(0.0, f64::max);
    Column {
        box_size: (settings.width, deepest + settings.line_width),
        rib: None,
    }
}

/// Σ of the layer's change depths. O and R re-plan the layer's first change with the purge
/// the finishing wall and infill will take anyway ("save on last wipe"); S never does.
fn layer_depth(
    layer: &ToolLayer,
    first_layer: bool,
    settings: &TowerSettings,
    build: SlicerBuild,
) -> f64 {
    (0..layer.changes)
        .map(|change| {
            let saved = change == 0 && build != SlicerBuild::SnapmakerOrca;
            change_depth(layer, first_layer, saved, settings, build)
        })
        .sum()
}

fn change_depth(
    layer: &ToolLayer,
    first_layer: bool,
    saved: bool,
    settings: &TowerSettings,
    build: SlicerBuild,
) -> f64 {
    let pw = settings.line_width;
    let ram_width = 2.0 * pw;
    let height = layer.z_top - layer.z_bottom;
    let draw = draw_width(settings, build);
    let ram = Ram::of(layer, settings, build, height);
    let band = ram.band(draw, ram_width * settings.extra_spacing, settings, build);
    let credit = ram.first_line_credit(draw, settings, build);
    let mut volume = settings.prime_volume;
    if saved {
        volume = saved_volume(layer, settings, height);
    }
    let wiped = (volume - credit * bead_area(pw * settings.extra_flow, height)).max(0.0);
    let rows = (wiped / bead_area(pw, height) / settings.extra_flow / draw).floor() + 1.0;
    rows * row_pitch(first_layer, settings, build) + band
}

/// Width a purge row is planned across: R and O keep one and a half lines off each wall
/// pair; S keeps a line plus two ramming lines.
fn draw_width(settings: &TowerSettings, build: SlicerBuild) -> f64 {
    let pw = settings.line_width;
    match build {
        SlicerBuild::SnapmakerOrca => settings.width - pw - 2.0 * (2.0 * pw),
        _ => settings.width - 3.0 * pw,
    }
}

/// O and R: the first layer is wiped densely, later layers at the extra spacing; S uses the
/// extra spacing everywhere.
fn row_pitch(first_layer: bool, settings: &TowerSettings, build: SlicerBuild) -> f64 {
    let pw = settings.line_width;
    if first_layer && build != SlicerBuild::SnapmakerOrca {
        pw * settings.extra_flow
    } else {
        pw * settings.extra_spacing * settings.extra_flow
    }
}

/// The purge of a saved change: at least the minimal purge, otherwise what is left after the
/// finishing layer. The finishing layer is at least the wall loop of a one-line-deep tower,
/// so this is never less than the slicer's value.
fn saved_volume(layer: &ToolLayer, settings: &TowerSettings, height: f64) -> f64 {
    let pw = settings.line_width;
    let minimal = largest_of(&settings.minimal_purge, &layer.filaments, 0.0);
    let finishing = 2.0 * (settings.width + pw) * bead_area(pw, height);
    minimal.max(settings.prime_volume - finishing)
}

/// The multi-tool ram before a change: its line length at double width.
struct Ram {
    enabled: bool,
    length: f64,
}

impl Ram {
    fn of(layer: &ToolLayer, settings: &TowerSettings, build: SlicerBuild, height: f64) -> Ram {
        let rams: Vec<_> = layer
            .filaments
            .iter()
            .filter_map(|f| settings.ramming.get((*f as usize).checked_sub(1)?))
            .filter(|ram| ram.enabled)
            .collect();
        let volume = rams
            .iter()
            .map(|ram| match build {
                SlicerBuild::Orca24 => O_RAMMING_SECONDS * ram.flow,
                _ => ram.volume,
            })
            .fold(0.0, f64::max);
        Ram {
            enabled: !rams.is_empty(),
            length: volume / bead_area(2.0 * settings.line_width, height),
        }
    }

    /// Depth reserved for the ram: R rounds the lines down plus one, S up (at least one);
    /// O reserves it only with `enable_filament_ramming` (the ram still prints).
    fn band(&self, draw: f64, pitch: f64, settings: &TowerSettings, build: SlicerBuild) -> f64 {
        if !self.enabled {
            return 0.0;
        }
        let lines = match build {
            SlicerBuild::SnapmakerOrca => (self.length / draw).ceil().max(1.0),
            SlicerBuild::Orca24 if !settings.filament_ramming => return 0.0,
            _ => (self.length / draw).floor() + 1.0,
        };
        lines * pitch
    }

    /// The rest of the ram's last line, which the first wipe line completes. R drops it with
    /// skip points or without a ram; O and S always count it.
    fn first_line_credit(&self, draw: f64, settings: &TowerSettings, build: SlicerBuild) -> f64 {
        let counted = match build {
            SlicerBuild::Orca25 => self.enabled && !settings.skip_points,
            _ => true,
        };
        if counted {
            draw - self.length.rem_euclid(draw)
        } else {
            0.0
        }
    }
}

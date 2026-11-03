//! Where each build keeps the stored tower anchor `wipe_tower_x/y` (spec 04 §6, design §6):
//! the packer must place the tower inside this range, or the tower it reserved is not the
//! one that prints.
//!
//! - BambuStudio: the GUI re-clamps on project open to 15 mm from the plate edges, sized by
//!   its own pre-slice estimate (the CLI never moves it).
//! - Snapmaker Orca: the GUI re-clamps on every scene reload, after each slice too, with a
//!   margin of 1 + `prime_tower_brim_width`, sized by the larger of its estimate and the real
//!   depth.
//! - OrcaSlicer 2.5: the CLI and the GUI clamp on every slice with 1 + the estimated real
//!   brim; within 5 mm of the range they snap, farther out they relocate by object positions.
//! - OrcaSlicer 2.4: nothing moves on open or CLI slice; a preset change would clamp with
//!   1 + brim, so the range keeps that margin.

use super::formulas::min_depth_for_height;
use crate::first_layer::{ProjectSettings, SlicerBuild};
use crate::geometry::Rect;

/// BambuStudio's GUI keeps the tower this far from the plate edges (04 Appendix A).
pub const BAMBU_GUI_MARGIN_MM: f64 = 15.0;
/// Orca-family margin before the brim is added.
pub const ORCA_GUI_MARGIN_MM: f64 = 1.0;
/// OrcaSlicer 2.5 snaps an anchor this close to its range and relocates one farther out.
pub const ORCA25_SNAP_MM: f64 = 5.0;
/// Purge per change in BambuStudio's "Saving" prime volume mode.
const SAVING_PRIME_VOLUME: f64 = 15.0;
/// Layer height the GUIs fall back to when the project has none.
const GUI_FALLBACK_LAYER_HEIGHT: f64 = 0.08;
/// Snapmaker Orca sizes its pre-slice depth at this layer height, whatever the project's.
const SNAPMAKER_ESTIMATE_LAYER_HEIGHT: f64 = 0.2;

/// The tower as the anchor rules see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TowerExtent {
    /// Printed column width and depth (box incl. walls).
    pub width: f64,
    pub depth: f64,
    /// Reach of the L0 brim beyond the wall centre line.
    pub brim_reach: f64,
}

/// The size BambuStudio's GUI assumes before slicing (04 §6). `filaments` = the plate's
/// filaments; `None` when the GUI shows no tower.
pub fn bambu_gui_estimate(
    settings: &ProjectSettings,
    filaments: usize,
    object_height: f64,
) -> Option<(f64, f64)> {
    if filaments == 0 {
        return None;
    }
    let layer_height = settings
        .number("layer_height")
        .filter(|h| *h > 0.0)
        .unwrap_or(GUI_FALLBACK_LAYER_HEIGHT);
    let gap = settings.number("prime_tower_infill_gap").unwrap_or(100.0) / 100.0;
    let tower_needed = settings.text("timelapse_type") == Some("1")
        || settings.flag("enable_wrapping_detection") == Some(true);
    let purged = bambu_gui_purge(settings, filaments);
    let min_depth = min_depth_for_height(object_height);
    if settings.flag("prime_tower_rib_wall").unwrap_or(true) {
        if !tower_needed && filaments < 2 {
            return None;
        }
        // A square rib tower: the purge spread over layers sets the side, the height its
        // minimum, and the ribs stick out by half their width per side.
        let volume_side = libm::sqrt(purged / layer_height * gap);
        let side = volume_side.max(min_depth);
        let rib = settings
            .number("prime_tower_rib_width")
            .unwrap_or(8.0)
            .min(side / 2.0);
        let extra = settings
            .number("prime_tower_extra_rib_length")
            .unwrap_or(0.0);
        let size = rib / std::f64::consts::SQRT_2 + (side + extra).max(volume_side);
        return Some((size, size));
    }
    let width = settings.number("prime_tower_width")?;
    let depth = purged / (layer_height * width) * gap;
    if !tower_needed && depth <= 1e-9 {
        return None;
    }
    Some((width, depth.max(min_depth)))
}

/// Volume the Bambu GUI plans to purge per layer: the largest prime volume once per filament
/// change; a two-nozzle printer purges for every filament and adds the filament retracted out
/// of the idle nozzle for every pair.
fn bambu_gui_purge(settings: &ProjectSettings, filaments: usize) -> f64 {
    let largest_prime = if settings.text("prime_volume_mode") == Some("Saving") {
        SAVING_PRIME_VOLUME
    } else {
        settings
            .numbers("filament_prime_volume")
            .into_iter()
            .fold(0.0, f64::max)
    };
    let two_nozzles = settings.texts("nozzle_diameter").len() == 2;
    if !two_nozzles {
        return largest_prime * filaments.saturating_sub(1) as f64;
    }
    let length = settings
        .numbers("filament_change_length")
        .into_iter()
        .fold(0.0, f64::max);
    let diameter = settings
        .numbers("filament_diameter")
        .into_iter()
        .fold(0.0, f64::max);
    let diameter = if diameter > 0.0 { diameter } else { 1.75 };
    let retracted = length * std::f64::consts::PI * diameter * diameter / 4.0;
    largest_prime * filaments as f64 + retracted * (filaments / 2) as f64
}

/// Snapmaker Orca's pre-slice depth estimate (04 §6): one prime volume per extra filament at
/// a fixed 0.2 mm layer, at least the depth by object height.
pub fn snapmaker_gui_depth(
    settings: &ProjectSettings,
    filaments: usize,
    object_height: f64,
) -> f64 {
    let width = settings.number("prime_tower_width").unwrap_or(0.0);
    let volume = settings.number("prime_volume").unwrap_or(0.0);
    let depth = if filaments <= 1 || width <= 0.0 {
        0.0
    } else {
        volume * (filaments - 1) as f64 / (SNAPMAKER_ESTIMATE_LAYER_HEIGHT * width)
    };
    if depth > 0.0 || settings.text("timelapse_type") == Some("1") {
        depth.max(min_depth_for_height(object_height))
    } else {
        depth
    }
}

/// The anchor range of `build` for a tower of `extent` on the plate with `printable` area
/// (plate-local coordinates; Orca-family minima count from the plate origin, maxima from the
/// bed-shape bbox). `filaments` = the plate's filament count. `None` when the range is empty.
pub fn anchor_range(
    build: SlicerBuild,
    settings: &ProjectSettings,
    extent: TowerExtent,
    printable: Rect,
    object_height: f64,
    filaments: usize,
) -> Option<Rect> {
    let range = match build {
        SlicerBuild::BambuStudio => {
            let (size_x, size_y) = bambu_gui_estimate(settings, filaments, object_height)?;
            let m = BAMBU_GUI_MARGIN_MM;
            Rect::new(
                printable.min_x + m,
                printable.min_y + m,
                printable.max_x - size_x - m,
                printable.max_y - size_y - m,
            )
        }
        SlicerBuild::SnapmakerOrca => {
            let m = ORCA_GUI_MARGIN_MM
                + settings
                    .number("prime_tower_brim_width")
                    .unwrap_or(0.0)
                    .max(0.0);
            let width = settings.number("prime_tower_width").unwrap_or(extent.width);
            let depth = snapmaker_gui_depth(settings, filaments, object_height).max(extent.depth);
            Rect::new(
                m,
                m,
                printable.max_x - width - m,
                printable.max_y - depth - m,
            )
        }
        SlicerBuild::Orca25 => {
            let m = ORCA_GUI_MARGIN_MM + extent.brim_reach;
            let depth = extent.depth.max(min_depth_for_height(object_height));
            let (plate_x, plate_y) = (
                printable.max_x - printable.min_x,
                printable.max_y - printable.min_y,
            );
            Rect::new(m, m, plate_x - extent.width - m, plate_y - depth - m)
        }
        SlicerBuild::Orca24 => {
            let brim = settings.number("prime_tower_brim_width").unwrap_or(0.0);
            let m = ORCA_GUI_MARGIN_MM + brim.max(extent.brim_reach);
            Rect::new(
                m,
                m,
                printable.max_x - extent.width - m,
                printable.max_y - extent.depth - m,
            )
        }
    };
    (range.min_x <= range.max_x && range.min_y <= range.max_y).then_some(range)
}

/// Where `build` prints a tower stored at `stored`, given its anchor range: Bambu and
/// Snapmaker clamp into the range, OrcaSlicer 2.4 keeps it, OrcaSlicer 2.5 snaps within
/// 5 mm and otherwise relocates by object positions (`None`: not predictable).
pub fn printed_anchor(build: SlicerBuild, stored: (f64, f64), range: Rect) -> Option<(f64, f64)> {
    let clamped = (
        stored.0.clamp(range.min_x, range.max_x),
        stored.1.clamp(range.min_y, range.max_y),
    );
    match build {
        SlicerBuild::BambuStudio | SlicerBuild::SnapmakerOrca => Some(clamped),
        SlicerBuild::Orca24 => Some(stored),
        SlicerBuild::Orca25 => {
            let off = (stored.0 - clamped.0)
                .abs()
                .max((stored.1 - clamped.1).abs());
            (off <= ORCA25_SNAP_MM).then_some(clamped)
        }
    }
}

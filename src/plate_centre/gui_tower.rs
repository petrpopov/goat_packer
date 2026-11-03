//! What the slicer GUI does to a stored wipe tower position, for callers that know only the
//! profile's `slicer_kind`. The rules themselves are written once, per slicer build, in
//! `tower_model::anchor` from the behaviour description of spec 04 §6; this module picks the
//! build and counts every project filament as used on the plate (more filaments only make
//! the estimate larger and the range stricter).
//!
//! - `"bambu"`: BambuStudio re-clamps `wipe_tower_x/y` to 15 mm from the plate edges, sized
//!   by its own pre-slice estimate, when it opens a 3MF. Evidence: a P1S corner tower stored
//!   at (6.24, 228.52) opened at (15, 223.84), and its sliced tower met the top-left goat's
//!   ear while the CLI slice (which keeps the stored position) was clean.
//! - `"orca"`: the Orca family, read as Snapmaker Orca, the owner's U1 GUI (design §14 Q2): it
//!   re-clamps on every scene reload with a margin of 1 + `prime_tower_brim_width`, the
//!   configured width and its depth estimate (both owner U1 plates sit exactly on
//!   x = 270.5 − W0 − 6). The old 1 mm margin let it move a tower 5 mm onto objects. After a
//!   slice it uses the real depth, which only `tower_model::printed_tower` knows; callers
//!   with a tower model use `TowerPrint::anchor_range` instead.

use crate::first_layer::{ProjectSettings, SlicerBuild};
use crate::geometry::Rect;
use crate::tower_model::{TowerExtent, anchor_range, bambu_gui_estimate};

/// Every filament of the project, the plate's count as far as a caller without the model
/// knows it.
fn project_filaments(settings: &ProjectSettings) -> usize {
    settings.texts("filament_colour").len()
}

/// The tower size BambuStudio's GUI assumes before slicing, which is what its open-time clamp
/// uses (not the sliced size). `None` when the GUI would show no tower.
pub fn gui_tower_estimate(settings: &serde_json::Value, object_height: f64) -> Option<(f64, f64)> {
    let settings = ProjectSettings::from_json(settings);
    bambu_gui_estimate(&settings, project_filaments(&settings), object_height)
}

/// Where the slicer GUI keeps a stored tower position (`wipe_tower_x/y`, the tower's anchor
/// corner): `None` when the range is empty or the GUI shows no tower.
pub fn gui_tower_anchor_range(
    settings: &serde_json::Value,
    slicer_kind: &str,
    plate: Rect,
    object_height: f64,
) -> Option<Rect> {
    let settings = ProjectSettings::from_json(settings);
    let filaments = project_filaments(&settings);
    let build = if slicer_kind == "orca" {
        SlicerBuild::SnapmakerOrca
    } else {
        SlicerBuild::BambuStudio
    };
    if build == SlicerBuild::SnapmakerOrca && filaments < 2 {
        return None;
    }
    // Without a tower model the real size is unknown: the estimates alone size the range.
    let extent = TowerExtent {
        width: settings.number("prime_tower_width").unwrap_or(0.0),
        depth: 0.0,
        brim_reach: 0.0,
    };
    anchor_range(build, &settings, extent, plate, object_height, filaments)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The P1S goat project of job b3970318 (BambuStudio 2.8.2.61): its GUI opened the tower
    /// stored at (6.236, 228.524) with the centre readout (23.58, 232.42) — min corner
    /// (15, 223.84) for the estimated 17.16 mm rib tower.
    fn goat_settings() -> serde_json::Value {
        serde_json::json!({
            "layer_height": "0.28",
            "prime_tower_infill_gap": "150%",
            "prime_tower_rib_wall": "1",
            "prime_tower_rib_width": "8",
            "prime_tower_extra_rib_length": "0",
            "prime_tower_width": "20",
            "filament_colour": ["#FFFFFF", "#000000", "#C12E1F"],
            "nozzle_diameter": ["0.4"],
            "filament_prime_volume": ["45", "45", "30"],
            "prime_volume_mode": "Saving",
            "timelapse_type": "0",
            "enable_wrapping_detection": "0",
            "wipe_tower_x": ["6.236"],
            "wipe_tower_y": ["228.524"],
        })
    }

    /// The owner's U1 project (Snapmaker Orca 2.4.0): rectangular 20 mm tower, 5 mm brim.
    fn u1_settings() -> serde_json::Value {
        serde_json::json!({
            "layer_height": "0.28",
            "prime_tower_width": "20",
            "prime_tower_brim_width": "5",
            "prime_volume": "20",
            "filament_colour": ["#FFFFFF", "#4C373C", "#A3413D", "#FD8008"],
            "wipe_tower_x": ["244.5"],
            "wipe_tower_y": ["245"],
        })
    }

    #[test]
    fn gui_estimate_matches_bambu_studio_readout() {
        let (x, y) = gui_tower_estimate(&goat_settings(), 44.74).unwrap();
        assert!(
            (x - 17.16).abs() < 0.01 && (y - 17.16).abs() < 0.01,
            "{x} {y}"
        );
        let range = gui_tower_anchor_range(
            &goat_settings(),
            "bambu",
            Rect::new(0.0, 0.0, 256.0, 256.0),
            44.74,
        )
        .unwrap();
        assert!((range.min_x - 15.0).abs() < 1e-9 && (range.min_y - 15.0).abs() < 1e-9);
        assert!((range.max_x - 223.84).abs() < 0.01 && (range.max_y - 223.84).abs() < 0.01);
        // The GUI centre readout: clamped min corner plus half the estimate.
        assert!((range.min_x + x / 2.0 - 23.58).abs() < 0.01);
        assert!((range.max_y + y / 2.0 - 232.42).abs() < 0.01);
    }

    #[test]
    fn single_filament_shows_no_gui_tower() {
        let mut settings = goat_settings();
        settings["filament_colour"] = serde_json::json!(["#FFFFFF"]);
        assert_eq!(gui_tower_estimate(&settings, 44.74), None);
        assert_eq!(
            gui_tower_anchor_range(&settings, "orca", Rect::new(0.0, 0.0, 256.0, 256.0), 44.74),
            None
        );
    }

    /// Both owner U1 plates sit on x = 270.5 − W0 − (1 + 5): the stored 244.5 is the bound.
    #[test]
    fn snapmaker_keeps_one_millimetre_plus_the_brim() {
        let printable = Rect::new(0.5, 1.0, 270.5, 271.0);
        let range = gui_tower_anchor_range(&u1_settings(), "orca", printable, 44.743).unwrap();
        assert!((range.min_x - 6.0).abs() < 1e-9 && (range.min_y - 6.0).abs() < 1e-9);
        assert!((range.max_x - 244.5).abs() < 1e-9, "{range:?}");
        // Pre-slice depth: one 20 mm³ purge per extra project filament at 0.2 mm across the
        // 20 mm tower (15 mm) beats the 11.275 mm height minimum.
        assert!(
            (range.max_y - (271.0 - 15.0 - 6.0)).abs() < 1e-9,
            "{range:?}"
        );
    }
}

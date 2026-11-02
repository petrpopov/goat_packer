//! The printer profile (`profiles/*.json`): the only source of printer geometry.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use super::ColoredPackerResult;
use super::bed_rules::{GuiFlavour, ProfileBedRules};
use crate::geometry::{Rect, axis_aligned_line_keepout};
use crate::plate_centre::HeatZone;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrinterProfile {
    pub id: String,
    pub printer_name: String,
    pub slicer_kind: String,
    pub slicer_path: String,
    pub blank_start_gcode_for_cli: bool,
    pub no_check_for_final_tower_probe: bool,
    pub printable_area: [f64; 4],
    /// How far model bodies keep from the printable edge; with `bed_rules` it equals their
    /// body margin (U1: Snapmaker Orca's 3.5 mm part band, equality passes).
    pub edge_margin_mm: f64,
    /// Edge margin for the supports and brim (the printed patch beyond the model) when it
    /// differs from `edge_margin_mm`, which then binds the model only (`PackOptions::body_bed`).
    /// U1: the edge margin keeps the meshes off the edge, but a hand-made 48-goat plate
    /// printed its support bases 0.7 mm from the printable edge (the closest the owner found
    /// Snapmaker Orca to accept), and the then 4 mm margin on the 47.6 mm support patch held
    /// the packer at 45. A set value is calibrated: it replaces the packer's default drift
    /// reserve for supports at the bed edge, which is why U1 keeps 0.7 here while its
    /// `bed_rules` reserve is 0 (the support model's calibrated margin covers the drift
    /// there). Absent = `edge_margin_mm` binds everything: a macro purge (`printer_catalog
    /// annotate`) may run anywhere near the edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support_edge_margin_mm: Option<f64>,
    pub exclusion_rectangles: Vec<[f64; 4]>,
    pub purge_lines: Vec<[f64; 5]>,
    pub tower_marker_style: String,
    pub tower_line_width_mm: f64,
    /// Printer-specific nozzle-clean macro body injected into `layer_change_gcode` by
    /// `nozzle_clean::inject_nozzle_clean`. P1/X1 machines need raw G1 moves to the back
    /// chute (no `G150.x` firmware macros); A2/P2/H2 are expected to wipe via `G150.3`/`G150.1`,
    /// but no A2L sequence is hardware-validated, so the A2L profile omits the field.
    /// Absent (e.g. Snapmaker) means nozzle-clean injection is unsupported for this printer.
    #[serde(default)]
    pub nozzle_clean_gcode: Option<String>,
    /// Real full duration (s) of one filament change with the AMS actually attached; replaces
    /// the slicer's per-change assumption in `print_time::corrected_seconds`. Gen-1 AMS
    /// measured: P1S 57 + 34, A2L 23 + 72 (spec 2026-10-26). Absent = tool changer (U1) or
    /// no correction known.
    #[serde(default)]
    pub filament_change_seconds: Option<f64>,
    /// Bed-heating zones, conventionally smallest first; the smallest zone's centre (by area,
    /// not list position) is where time-window plates gather their objects and the prime tower
    /// (`plate_centre`). A2L source: the system
    /// profile key `bed_heat_soak_area` (two rectangles as corner lists, plus the full bed);
    /// its start G-code heat-soaks only the first-layer bbox
    /// (`G29.30 X{first_layer_print_min[0]} ...`), so a compact plate in the inner zone heats
    /// less bed. Absent = printer heats the whole bed; the centre is the usable bed's.
    #[serde(default)]
    pub heat_zones: Vec<HeatZone>,
    /// The printer's plate zones by class (design 2026-11-02, §7.4): what the slicer-free
    /// core packs against (`bed_rules::plate_rules`). The old bed fields above stay and must
    /// say the same (`bed_rules::old_field_mismatches`): the legacy pipeline, the webapp and
    /// `source_project --edge-margin` read them. Absent = the old fields through the legacy
    /// table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bed_rules: Option<ProfileBedRules>,
    /// The slicer GUI the plate is opened in (`"bambu-02.08"`, `"snapmaker-orca-2.4"`,
    /// `"orca-2.4"`): its own acceptance rules (Snapmaker Orca's 3.5 mm part band) and the
    /// support behaviour to predict. A job prefers the project's own `Application`; this is the
    /// fallback, and the only source an STL job has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gui_flavour: Option<GuiFlavour>,
}

impl PrinterProfile {
    pub fn printable_bed(&self) -> Rect {
        Rect::new(
            self.printable_area[0],
            self.printable_area[1],
            self.printable_area[2],
            self.printable_area[3],
        )
    }

    pub fn usable_bed(&self) -> Result<Rect, String> {
        self.printable_bed().inset(self.edge_margin_mm)
    }

    /// Where the supports and brim may print: the printable area inset by
    /// `support_edge_margin_mm`, or `usable_bed` when the profile has no separate margin.
    pub fn support_bed(&self) -> Result<Rect, String> {
        match self.support_edge_margin_mm {
            Some(margin) => self.printable_bed().inset(margin),
            None => self.usable_bed(),
        }
    }

    /// Fixed keepout rectangles: bed exclusion corners plus purge-line strips. The purge
    /// strips grow by `purge_tip_overshoot_mm` — how far the slicer's real first layer
    /// extends past the mesh outline at thin tips (Arachne/miter, measured up to ~1.7 mm on
    /// a P1S goat's legs) — because the prime line is machine start gcode: nothing routes it
    /// around objects, so a goat whose mesh clears the line can still print its leg tips
    /// straight through it. The measured overshoot from the probe slice comes in here; pass
    /// 0.0 when no probe data exists.
    pub fn fixed_keepouts(&self, purge_tip_overshoot_mm: f64) -> ColoredPackerResult<Vec<Rect>> {
        let mut keepouts: Vec<Rect> = self
            .exclusion_rectangles
            .iter()
            .map(|rect| {
                Rect::new(rect[0], rect[1], rect[2], rect[3]).grown(SLICER_KEEPOUT_SAFETY_MM)
            })
            .collect();
        for line in &self.purge_lines {
            let keepout = axis_aligned_line_keepout(line[0], line[1], line[2], line[3], line[4])?;
            keepouts.push(keepout.grown(PURGE_LINE_KEEPOUT_SAFETY_MM + purge_tip_overshoot_mm));
        }
        Ok(keepouts)
    }
}

/// Empirically measured extra standoff (mm) BambuStudio/OrcaSlicer's CLI require beyond a plain
/// `line_width/2 + clearance` expansion (for the prime tower) or beyond the profile's own bed
/// exclusion/purge-line rectangles before it will slice a dense multi-color plate without
/// reporting a false-positive "gcode path conflicts" / "too close to exclusion area" error. An
/// isolated single-object bisection against the real BambuStudio CLI found a minimum around
/// 0.7-0.8mm extra for the tower alone, but a real dense ~19-22 object Goat 3 P1S plate kept
/// hitting conflicts near both the tower and the bed exclusion corner at smaller margins — even at
/// 2.5mm the full 16000-attempt search occasionally still packed an object right up against the
/// exclusion corner, so this was raised to 4.0mm and applied uniformly to every keepout the packer
/// reserves. Isolated single-object measurements and reduced-attempt tuning runs both understate
/// what a fully packed, maximally-dense real plate needs.
pub(super) const SLICER_KEEPOUT_SAFETY_MM: f64 = 4.0;

/// Purge-line keepout grows by half the line width already; the line's coordinates come
/// straight from the printer profile, so the base margin only needs to absorb raster
/// quantization. A 4 mm blanket here pushed the keepout 0.25 mm into the usable bed and
/// dropped the four cells of the bottom row that sit above the purge line. The real
/// standoff comes from `fixed_keepouts`' tip-overshoot parameter, measured per model.
const PURGE_LINE_KEEPOUT_SAFETY_MM: f64 = 0.5;

pub fn load_printer_profile(path: &Path) -> ColoredPackerResult<PrinterProfile> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read printer profile {}: {error}", path.display()))?;
    let profile: PrinterProfile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid printer profile {}: {error}", path.display()))?;
    if profile.slicer_kind != "bambu" && profile.slicer_kind != "orca" {
        return Err(format!("unsupported slicer kind: {}", profile.slicer_kind).into());
    }
    if profile.tower_line_width_mm <= 0.0 {
        return Err("tower_line_width_mm must be positive".into());
    }
    profile.usable_bed()?;
    profile.support_bed()?;
    if let Some(rules) = &profile.bed_rules {
        rules
            .validate()
            .map_err(|error| format!("printer profile {}: {error}", path.display()))?;
    }
    Ok(profile)
}

const SLICER_KIND_BY_PRINTER_SUBSTRING: &[(&str, &str)] =
    &[("Bambu Lab", "bambu"), ("Snapmaker", "orca")];

/// Maps a 3MF's `printer_model` string to a slicer kind ("bambu"/"orca") using known printer-name
/// substrings. `Application` metadata in the 3MF is not used for this because OrcaSlicer-family
/// tools (including Snapmaker's) are BambuStudio forks and still emit
/// `Application: BambuStudio-x.y.z`, making that field an unreliable signal. Returns `None` for an
/// unrecognized printer so callers can fall back to the printer profile's own `slicer_kind`.
pub fn detect_slicer_kind_from_printer_model(printer_model: &str) -> Option<&'static str> {
    SLICER_KIND_BY_PRINTER_SUBSTRING
        .iter()
        .find(|(needle, _)| printer_model.contains(needle))
        .map(|(_, kind)| *kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Footprint, Point, placement_is_allowed, rasterize_footprint};

    #[test]
    fn support_margin_widens_the_support_bed_only() {
        let mut profile = corner_bed_profile();
        profile.printable_area = [0.5, 1.0, 270.5, 271.0];
        profile.edge_margin_mm = 4.0;
        assert_eq!(profile.support_bed().unwrap(), profile.usable_bed().unwrap());
        profile.support_edge_margin_mm = Some(0.7);
        assert_eq!(profile.support_bed().unwrap(), profile.printable_bed().inset(0.7).unwrap());
        assert_eq!(profile.usable_bed().unwrap(), Rect::new(4.5, 5.0, 266.5, 267.0));
    }

    /// A bed whose purge line hugs the front edge (y=1.0..1.5, x 18..240) with one exclusion
    /// corner front-left (the geometry of the plate in the regression below).
    fn corner_bed_profile() -> PrinterProfile {
        PrinterProfile {
            id: "test_corner_bed".to_owned(),
            printer_name: "Test Corner Bed".to_owned(),
            slicer_kind: "bambu".to_owned(),
            slicer_path: String::new(),
            blank_start_gcode_for_cli: false,
            no_check_for_final_tower_probe: false,
            printable_area: [0.0, 0.0, 256.0, 256.0],
            edge_margin_mm: 0.0,
            support_edge_margin_mm: None,
            exclusion_rectangles: vec![[0.0, 0.0, 18.0, 28.0]],
            purge_lines: vec![
                [18.0, 1.0, 240.0, 1.0, 0.5],
                [240.0, 1.0, 240.0, 11.0, 0.5],
                [239.5, 11.0, 239.5, 1.5, 0.5],
                [239.5, 1.5, 18.0, 1.5, 0.5],
            ],
            tower_marker_style: "wipe_tower".to_owned(),
            tower_line_width_mm: 0.5,
            nozzle_clean_gcode: None,
            filament_change_seconds: None,
            heat_zones: vec![],
            bed_rules: None,
            gui_flavour: None,
        }
    }

    /// Regression (goat3_v4 P1S, 2026-09-14): the bottom-row goats' silhouettes cleared the
    /// bare purge-line keepout (top y=1.75) with the mesh at y≈2.6, but the slicer prints
    /// thin leg tips up to 1.66 mm past the outline — straight through the prime line at
    /// y∈[1.0,1.5]. Only the purge strips may grow by the measured overshoot; the bed
    /// exclusion corner keeps its own validated safety.
    #[test]
    fn purge_keepouts_grow_by_the_measured_tip_overshoot_and_only_them() {
        let profile = corner_bed_profile();
        let bare = profile.fixed_keepouts(0.0).unwrap();
        let widened = profile.fixed_keepouts(1.66).unwrap();
        assert_eq!(bare.len(), 5, "1 corner + 4 purge strips");
        // The exclusion corner is index 0 and must not move with the purge overshoot.
        assert_eq!(bare[0], widened[0]);
        for (bare_rect, widened_rect) in bare.iter().zip(widened.iter()).skip(1) {
            assert!((widened_rect.min_y - (bare_rect.min_y - 1.66)).abs() < 1e-9);
            assert!((widened_rect.max_y - (bare_rect.max_y + 1.66)).abs() < 1e-9);
        }
        // Bare bottom line keepout: y=1 line, width 0.5 -> ±0.25, +0.5 safety -> top 1.75.
        assert!((bare[1].max_y - 1.75).abs() < 1e-9);
    }

    /// The exact position the broken plate used — a 63x53 goat with its mesh bottom at
    /// y≈2.7, printing tips down to ~1.06 — must be rejected once the keepout carries the
    /// measured overshoot, and would still be legal with the bare one (that was the bug).
    #[test]
    fn widened_purge_keepout_rejects_the_position_that_printed_through_the_line() {
        let profile = corner_bed_profile();
        let keepouts = profile.fixed_keepouts(1.66).unwrap();
        let loops = vec![
            Point { x: -31.5, y: -26.5 },
            Point { x: 31.5, y: -26.5 },
            Point { x: 31.5, y: 26.5 },
            Point { x: -31.5, y: 26.5 },
            Point { x: -31.5, y: -26.5 },
        ];
        let footprint = Footprint {
            support_landing_loops: Vec::new(),
            layer_zones: Vec::new(),
            body_loops: vec![loops.clone()],
            bed_loops: None,
            loops: vec![loops.clone()],
            width: 63.0,
            height: 53.0,
            min_z: 0.0,
            max_z: 0.0,
            triangle_count: 0,
        };
        let mask = rasterize_footprint(&footprint, 0.0, 0.25, 0.7);
        let bed = Rect::new(0.0, 0.0, 256.0, 256.0);
        // Mesh bottom at y≈2.75 (center 29.5): legal against the bare keepout, illegal
        // against the widened one — the printed layer would reach y≈1.1.
        assert!(placement_is_allowed(&mask, 500, 118, bed, &keepouts_bare()));
        assert!(!placement_is_allowed(&mask, 500, 118, bed, &keepouts));
    }

    fn keepouts_bare() -> Vec<Rect> {
        corner_bed_profile().fixed_keepouts(0.0).unwrap()
    }
}

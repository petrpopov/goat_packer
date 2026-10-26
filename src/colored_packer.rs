use std::{error::Error, fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::{Rect, axis_aligned_line_keepout, plate_centre::HeatZone};

pub type ColoredPackerResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrinterProfile {
    pub id: String,
    pub printer_name: String,
    pub slicer_kind: String,
    pub slicer_path: String,
    pub blank_start_gcode_for_cli: bool,
    pub no_check_for_final_tower_probe: bool,
    pub printable_area: [f64; 4],
    pub edge_margin_mm: f64,
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
    /// Bed-heating zones, smallest first; the first zone's centre is where time-window plates
    /// gather their objects and the prime tower (`plate_centre`). A2L source: the system
    /// profile key `bed_heat_soak_area` (two rectangles as corner lists, plus the full bed);
    /// its start G-code heat-soaks only the first-layer bbox
    /// (`G29.30 X{first_layer_print_min[0]} ...`), so a compact plate in the inner zone heats
    /// less bed. Absent = printer heats the whole bed; the centre is the usable bed's.
    #[serde(default)]
    pub heat_zones: Vec<HeatZone>,
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
                expand_rect(
                    Rect::new(rect[0], rect[1], rect[2], rect[3]),
                    SLICER_KEEPOUT_SAFETY_MM,
                )
            })
            .collect();
        for line in &self.purge_lines {
            let keepout = axis_aligned_line_keepout(line[0], line[1], line[2], line[3], line[4])?;
            keepouts.push(expand_rect(
                keepout,
                PURGE_LINE_KEEPOUT_SAFETY_MM + purge_tip_overshoot_mm,
            ));
        }
        Ok(keepouts)
    }
}

fn expand_rect(rect: Rect, margin: f64) -> Rect {
    Rect::new(
        rect.min_x - margin,
        rect.min_y - margin,
        rect.max_x + margin,
        rect.max_y + margin,
    )
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
const SLICER_KEEPOUT_SAFETY_MM: f64 = 4.0;

/// Purge-line keepout grows by half the line width already; the line's coordinates come
/// straight from the printer profile, so the base margin only needs to absorb raster
/// quantization. A 4 mm blanket here pushed the keepout 0.25 mm into the usable bed and
/// dropped the four cells of the bottom row that sit above the purge line. The real
/// standoff comes from `fixed_keepouts`' tip-overshoot parameter, measured per model.
const PURGE_LINE_KEEPOUT_SAFETY_MM: f64 = 0.5;

/// The tower bounds are re-measured from the final slice and a repack is forced when they
/// grow, so the reserve just covers probe-vs-final measurement noise — 4 mm here used to
/// throw away grid cells whose patch cleared the actual tower by a wide margin.
const TOWER_KEEPOUT_SAFETY_MM: f64 = 1.0;

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
    Ok(profile)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct PrimeTowerBounds {
    pub centerline: Rect,
    pub keepout: Rect,
    pub extrusion_width_mm: f64,
}

pub fn parse_prime_tower_bounds(
    gcode: &[u8],
    fallback_line_width_mm: f64,
    clearance_mm: f64,
) -> Option<PrimeTowerBounds> {
    let text = String::from_utf8_lossy(gcode);
    let mut absolute_mode = true;
    let mut x = 0.0;
    let mut y = 0.0;
    let mut active = false;
    let mut nozzle_wipe = false;
    let mut line_width = fallback_line_width_mm;
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut max_width = fallback_line_width_mm;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.starts_with("G90") {
            absolute_mode = true;
            continue;
        }
        if line.starts_with("G91") {
            absolute_mode = false;
            continue;
        }
        if let Some(value) = line.strip_prefix("; LINE_WIDTH:") {
            if let Ok(width) = value.trim().parse::<f64>() {
                line_width = width;
            }
            continue;
        }
        if line.contains("WIPE_TOWER_END") || line.contains("WIPE_TOWER_BRIM_END") {
            active = false;
            continue;
        }
        if line.contains("WIPE_TOWER_START") || line.contains("WIPE_TOWER_BRIM_START") {
            active = true;
            continue;
        }
        // OrcaSlicer's IDEX tool change (Snapmaker U1) empties and wipes the nozzle at a
        // garbage point far away from the tower between `;WIPE_START` and `;WIPE_END`, still
        // inside the WIPE_TOWER section. Those moves are not the tower: counting them made
        // the "tower bounds" half the plate wide on a real project.
        if line.contains(";WIPE_START") {
            nozzle_wipe = true;
            continue;
        }
        if line.contains(";WIPE_END") {
            nozzle_wipe = false;
            continue;
        }
        if !(line.starts_with("G0") || line.starts_with("G1")) {
            continue;
        }
        let parsed_x = gcode_axis(line, 'X');
        let parsed_y = gcode_axis(line, 'Y');
        if absolute_mode {
            if let Some(value) = parsed_x {
                x = value;
            }
            if let Some(value) = parsed_y {
                y = value;
            }
        } else {
            if let Some(value) = parsed_x {
                x += value;
            }
            if let Some(value) = parsed_y {
                y += value;
            }
        }
        if active
            && !nozzle_wipe
            && (parsed_x.is_some() || parsed_y.is_some())
            && gcode_axis(line, 'E').is_some()
        {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            max_width = max_width.max(line_width);
        }
    }

    if !min_x.is_finite() {
        return None;
    }
    let centerline = Rect::new(min_x, min_y, max_x, max_y);
    let expansion = max_width * 0.5 + clearance_mm + TOWER_KEEPOUT_SAFETY_MM;
    Some(PrimeTowerBounds {
        centerline,
        keepout: Rect::new(
            min_x - expansion,
            min_y - expansion,
            max_x + expansion,
            max_y + expansion,
        ),
        extrusion_width_mm: max_width,
    })
}

pub fn tower_repack_required(reserved_keepout: Rect, final_tower: PrimeTowerBounds) -> bool {
    final_tower.keepout.min_x < reserved_keepout.min_x
        || final_tower.keepout.min_y < reserved_keepout.min_y
        || final_tower.keepout.max_x > reserved_keepout.max_x
        || final_tower.keepout.max_y > reserved_keepout.max_y
}

fn gcode_axis(line: &str, axis: char) -> Option<f64> {
    line.split_whitespace().find_map(|token| {
        let token = token.trim_start_matches(';');
        token
            .strip_prefix(axis)
            .and_then(|value| value.parse::<f64>().ok())
    })
}

const SLICER_KIND_BY_PRINTER_SUBSTRING: &[(&str, &str)] = &[
    ("Bambu Lab", "bambu"),
    ("Snapmaker", "orca"),
];

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

    /// The real P1S profile geometry: the purge line hugs the front edge (y=1.0..1.5,
    /// x 18..240) and one exclusion corner sits front-left.
    fn p1s_profile() -> PrinterProfile {
        PrinterProfile {
            id: "bambu_p1s".to_owned(),
            printer_name: "Bambu Lab P1S".to_owned(),
            slicer_kind: "bambu".to_owned(),
            slicer_path: String::new(),
            blank_start_gcode_for_cli: false,
            no_check_for_final_tower_probe: false,
            printable_area: [0.0, 0.0, 256.0, 256.0],
            edge_margin_mm: 0.0,
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
        }
    }

    /// Regression (goat3_v4 P1S, 2026-09-14): the bottom-row goats' silhouettes cleared the
    /// bare purge-line keepout (top y=1.75) with the mesh at y≈2.6, but the slicer prints
    /// thin leg tips up to 1.66 mm past the outline — straight through the prime line at
    /// y∈[1.0,1.5]. Only the purge strips may grow by the measured overshoot; the bed
    /// exclusion corner keeps its own validated safety.
    #[test]
    fn purge_keepouts_grow_by_the_measured_tip_overshoot_and_only_them() {
        let profile = p1s_profile();
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
        let profile = p1s_profile();
        let keepouts = profile.fixed_keepouts(1.66).unwrap();
        let mut loops = vec![
            Point { x: -31.5, y: -26.5 },
            Point { x: 31.5, y: -26.5 },
            Point { x: 31.5, y: 26.5 },
            Point { x: -31.5, y: 26.5 },
            Point { x: -31.5, y: -26.5 },
        ];
        let footprint = Footprint {
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
        p1s_profile().fixed_keepouts(0.0).unwrap()
    }
}

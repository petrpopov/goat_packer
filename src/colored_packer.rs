use std::{error::Error, fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::{Rect, axis_aligned_line_keepout};

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

    pub fn fixed_keepouts(&self) -> ColoredPackerResult<Vec<Rect>> {
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
            keepouts.push(expand_rect(keepout, SLICER_KEEPOUT_SAFETY_MM));
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
        if active && (parsed_x.is_some() || parsed_y.is_some()) && gcode_axis(line, 'E').is_some() {
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
    let expansion = max_width * 0.5 + clearance_mm + SLICER_KEEPOUT_SAFETY_MM;
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

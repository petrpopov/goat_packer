//! Centre-out plates, heat zones and prime-tower relocation for time-window plates
//! (spec `docs/superpowers/specs/2026-10-26-centre-out-zones-tower-design.md`).
//!
//! Pure geometry plus the one `project_settings.config` edit the packer is allowed to make:
//! moving the wipe tower of plate 1. Any subset of a valid layout is valid, so "centre-out"
//! needs no new search — the packer orders the full-bed layout by distance from the centre and
//! the time window keeps a prefix of that order.

use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{Point, Rect};

const PROJECT_SETTINGS_ENTRY: &str = "Metadata/project_settings.config";

/// Allowed distance between the measured final tower keepout's min corner and the relocated
/// keepout's min corner (see `tower_corner_error_mm`). A slicer that ignored the relocation leaves the tower at the source position, which is the
/// whole shift away (well over 100 mm on the A2L koza project); measurement noise is ~1 mm.
pub const TOWER_TARGET_TOLERANCE_MM: f64 = 5.0;

/// One bed-heating zone of a printer, `rect` = `[x0, y0, x1, y1]` in bed coordinates.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HeatZone {
    pub name: String,
    pub rect: [f64; 4],
}

impl HeatZone {
    pub fn as_rect(&self) -> Rect {
        Rect::new(self.rect[0], self.rect[1], self.rect[2], self.rect[3])
    }
}

fn rect_centre(rect: Rect) -> (f64, f64) {
    ((rect.min_x + rect.max_x) * 0.5, (rect.min_y + rect.max_y) * 0.5)
}

fn rect_area(rect: Rect) -> f64 {
    (rect.max_x - rect.min_x) * (rect.max_y - rect.min_y)
}

/// Plate centre the objects and the tower gather around: the centre of the smallest heat zone
/// by area, or of the usable bed when the printer has no zones. Picked by area, the same rule
/// as `smallest_containing_zone`, so a mis-ordered profile cannot centre the plate on a large
/// zone while the report names the small one.
pub fn plate_centre(zones: &[HeatZone], usable_bed: Rect) -> (f64, f64) {
    let smallest = zones
        .iter()
        .map(HeatZone::as_rect)
        .min_by(|a, b| rect_area(*a).total_cmp(&rect_area(*b)));
    rect_centre(smallest.unwrap_or(usable_bed))
}

/// A tower keepout moved towards a target centre.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct RelocatedTower {
    pub keepout: Rect,
    /// Applied to `wipe_tower_x[0]`/`wipe_tower_y[0]`: the tower's size does not depend on its
    /// position, so moving the setting by the keepout's shift moves the printed tower with it.
    pub shift: (f64, f64),
}

/// Moves `keepout` so its centre lands on `target`, then pulls it back inside `bed`. The clamp
/// shrinks the shift, never the keepout: the keepout is what the slicer will print, shrinking
/// it would let objects collide with the real tower.
pub fn relocate_keepout(keepout: Rect, target: (f64, f64), bed: Rect) -> RelocatedTower {
    let (cx, cy) = rect_centre(keepout);
    let clamp_shift = |shift: f64, min: f64, max: f64, bed_min: f64, bed_max: f64| {
        // A keepout larger than the bed cannot fit anyway; it then aligns with the bed's
        // minimum edge and the packer's own bed check reports the overhang.
        shift.min(bed_max - max).max(bed_min - min)
    };
    let dx = clamp_shift(target.0 - cx, keepout.min_x, keepout.max_x, bed.min_x, bed.max_x);
    let dy = clamp_shift(target.1 - cy, keepout.min_y, keepout.max_y, bed.min_y, bed.max_y);
    RelocatedTower {
        keepout: Rect::new(
            keepout.min_x + dx,
            keepout.min_y + dy,
            keepout.max_x + dx,
            keepout.max_y + dy,
        ),
        shift: (dx, dy),
    }
}

/// Errors when the relocated tower keepout overlaps any of the profile's fixed keepouts
/// (exclusion rectangles, purge-line strips; `PrinterProfile::fixed_keepouts`). The relocation
/// clamps only against the bed, so on a printer whose plate centre sits near such an area the
/// slicer would print the tower into it — the packer reserves both as obstacles and would never
/// notice. Edge contact is not an overlap.
pub fn ensure_relocated_tower_clear(keepout: Rect, fixed_keepouts: &[Rect]) -> Result<(), String> {
    let overlaps = |other: &Rect| {
        keepout.min_x < other.max_x
            && other.min_x < keepout.max_x
            && keepout.min_y < other.max_y
            && other.min_y < keepout.max_y
    };
    if fixed_keepouts.iter().any(overlaps) {
        return Err(
            "relocated wipe tower would overlap a printer keep-out area; rerun with --no-centre"
                .to_owned(),
        );
    }
    Ok(())
}

/// Formats a millimetre setting the way BambuStudio writes them: up to three decimals, no
/// trailing zeros ("164.898", "160").
fn format_setting_mm(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_owned() } else { text.to_owned() }
}

fn first_entry(settings: &serde_json::Value, key: &str) -> Result<f64, String> {
    let entry = settings
        .get(key)
        .and_then(serde_json::Value::as_array)
        .and_then(|values| values.first())
        .ok_or_else(|| format!("project settings have no {key}[0] to relocate the tower"))?;
    let text = entry
        .as_str()
        .ok_or_else(|| format!("project setting {key}[0] is not a string: {entry}"))?;
    text.trim()
        .parse::<f64>()
        .map_err(|error| format!("project setting {key}[0] = {text:?} is not a number: {error}"))
}

/// Adds `shift` to `wipe_tower_x[0]`/`wipe_tower_y[0]` (arrays of strings, one entry per
/// plate) and returns the new position. Only entry 0 changes: the packer's output is one
/// plate, and every other entry belongs to a plate the packer does not touch (see
/// `scheduler::patch_wipe_tower_per_plate` for what those entries mean). On error the settings
/// are left untouched.
pub fn shift_wipe_tower_settings(
    settings: &mut serde_json::Value,
    shift: (f64, f64),
) -> Result<(f64, f64), String> {
    let x = first_entry(settings, "wipe_tower_x")? + shift.0;
    let y = first_entry(settings, "wipe_tower_y")? + shift.1;
    settings["wipe_tower_x"][0] = serde_json::Value::String(format_setting_mm(x));
    settings["wipe_tower_y"][0] = serde_json::Value::String(format_setting_mm(y));
    Ok((x, y))
}

/// Rewrites `project` in place with the wipe tower of plate 1 shifted by `shift`. Every entry
/// other than `project_settings.config` is copied byte-for-byte; the settings JSON is
/// re-serialized (keys alphabetized, the same as `prepare_cli_copy` and nozzle-clean injection
/// already do — BambuStudio reads a settings map, order carries no meaning).
pub fn relocate_wipe_tower_in_3mf(project: &Path, shift: (f64, f64)) -> Result<(f64, f64), String> {
    let context = |error: &dyn std::fmt::Display| format!("{}: {error}", project.display());
    let mut archive = ZipArchive::new(fs::File::open(project).map_err(|e| context(&e))?)
        .map_err(|e| context(&e))?;
    let mut entries: Vec<(String, bool, Vec<u8>)> = Vec::with_capacity(archive.len());
    let mut moved = None;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| context(&e))?;
        let name = entry.name().to_owned();
        let is_dir = entry.is_dir();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| context(&e))?;
        if name == PROJECT_SETTINGS_ENTRY {
            let mut settings: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| context(&e))?;
            moved = Some(shift_wipe_tower_settings(&mut settings, shift)?);
            bytes = serde_json::to_vec(&settings).map_err(|e| context(&e))?;
        }
        entries.push((name, is_dir, bytes));
    }
    drop(archive);
    let moved = moved.ok_or_else(|| format!("{} has no {PROJECT_SETTINGS_ENTRY}", project.display()))?;
    let mut zip = ZipWriter::new(fs::File::create(project).map_err(|e| context(&e))?);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, is_dir, bytes) in entries {
        if is_dir {
            zip.add_directory(name.trim_end_matches('/'), options)
                .map_err(|e| context(&e))?;
        } else {
            zip.start_file(&name, options).map_err(|e| context(&e))?;
            zip.write_all(&bytes).map_err(|e| context(&e))?;
        }
    }
    zip.finish().map_err(|e| context(&e))?;
    Ok(moved)
}

/// Indices of `centres` sorted by distance from `centre`, ties by original index. The time
/// window keeps a prefix of this order, so it always drops the farthest object first and the
/// kept objects form a compact disc around the centre.
pub fn centre_out_order(centres: &[(f64, f64)], centre: (f64, f64)) -> Vec<usize> {
    let distance = |index: usize| {
        let (x, y) = centres[index];
        (x - centre.0).hypot(y - centre.1)
    };
    let mut order: Vec<usize> = (0..centres.len()).collect();
    order.sort_by(|&a, &b| distance(a).total_cmp(&distance(b)).then(a.cmp(&b)));
    order
}

/// Axis-aligned bounds of every point of `loops`; `None` when there are no points.
pub fn loops_bounds(loops: &[Vec<Point>]) -> Option<Rect> {
    let mut points = loops.iter().flatten();
    let first = points.next()?;
    let mut bounds = Rect::new(first.x, first.y, first.x, first.y);
    for point in points {
        bounds = union_rect(bounds, Rect::new(point.x, point.y, point.x, point.y));
    }
    Some(bounds)
}

pub fn union_rect(a: Rect, b: Rect) -> Rect {
    Rect::new(
        a.min_x.min(b.min_x),
        a.min_y.min(b.min_y),
        a.max_x.max(b.max_x),
        a.max_y.max(b.max_y),
    )
}

/// The smallest (by area) zone fully containing `bbox`, edges inclusive. Picked by area rather
/// than profile order so a mis-ordered profile still reports the tightest zone.
pub fn smallest_containing_zone(zones: &[HeatZone], bbox: Rect) -> Option<&HeatZone> {
    const EPS: f64 = 1e-9;
    zones
        .iter()
        .filter(|zone| {
            let rect = zone.as_rect();
            bbox.min_x >= rect.min_x - EPS
                && bbox.min_y >= rect.min_y - EPS
                && bbox.max_x <= rect.max_x + EPS
                && bbox.max_y <= rect.max_y + EPS
        })
        .min_by(|a, b| rect_area(a.as_rect()).total_cmp(&rect_area(b.as_rect())))
}

/// Distance between the min corners `(min_x, min_y)` of a measured tower keepout and the target
/// keepout. `wipe_tower_x/y` anchors the tower's min corner and a full plate's tower grows away
/// from it (one-sided: more colour changes make it longer/wider, never move the anchor), so the
/// centre would drift by half the growth and read as an ignored relocation.
pub fn tower_corner_error_mm(measured: Rect, target: Rect) -> f64 {
    (measured.min_x - target.min_x).hypot(measured.min_y - target.min_y)
}

/// Distance the slicer GUI keeps between the stored tower position and the plate edges.
/// BambuStudio (2.8.2.61, `WIPE_TOWER_MARGIN` in libslic3r.h) re-clamps `wipe_tower_x/y` when
/// it opens a 3MF (`Plater` → `set_default_wipe_tower_pos_for_plate(keep_existing)`): a P1S
/// corner tower stored at (6.24, 228.52) opened at (15, 223.84) — the GUI's own position
/// readout, centre (23.58, 232.42) — and its sliced tower met the top-left goat's ear, while
/// the CLI slice (which keeps the stored position) was clean. OrcaSlicer's margin is 1 mm.
fn gui_tower_margin_mm(slicer_kind: &str) -> f64 {
    if slicer_kind == "orca" { 1.0 } else { 15.0 }
}

fn setting_value<'a>(settings: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match settings.get(key)? {
        serde_json::Value::Array(items) => items.first(),
        other => Some(other),
    }
}

fn setting_f64(settings: &serde_json::Value, key: &str) -> Option<f64> {
    match setting_value(settings, key)? {
        serde_json::Value::String(text) => text.trim().trim_end_matches('%').trim().parse().ok(),
        serde_json::Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

fn setting_all_f64(settings: &serde_json::Value, key: &str) -> Vec<f64> {
    match settings.get(key) {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().and_then(|text| text.trim().parse().ok()))
            .collect(),
        _ => setting_f64(settings, key).into_iter().collect(),
    }
}

fn setting_str<'a>(settings: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    setting_value(settings, key)?.as_str()
}

/// BambuStudio's `WipeTower::get_limit_depth_by_height`: minimum tower depth by object height,
/// linear between the table points, clamped to the first/last.
fn limit_depth_by_height(height: f64) -> f64 {
    const TABLE: [(f64, f64); 4] = [(5.0, 5.0), (100.0, 20.0), (250.0, 40.0), (350.0, 60.0)];
    if height <= TABLE[0].0 {
        return TABLE[0].1;
    }
    for pair in TABLE.windows(2) {
        let ((h0, d0), (h1, d1)) = (pair[0], pair[1]);
        if height < h1 {
            return d0 + (height - h0) / (h1 - h0) * (d1 - d0);
        }
    }
    TABLE[TABLE.len() - 1].1
}

/// The tower size the slicer GUI assumes before slicing (`PartPlate::estimate_wipe_tower_size`
/// of BambuStudio 2.8.2.61), which is what its open-time clamp uses — not the sliced size.
/// Every filament of the project counts as used on the plate (the GUI counts the plate's real
/// extruders): more filaments only make the estimate larger, the clamp stricter. `None` when
/// the GUI would show no tower or a setting is missing.
pub fn gui_tower_estimate(settings: &serde_json::Value, object_height: f64) -> Option<(f64, f64)> {
    let layer_height = setting_f64(settings, "layer_height")?;
    let extra_spacing = setting_f64(settings, "prime_tower_infill_gap").unwrap_or(100.0) / 100.0;
    let rib_wall = setting_str(settings, "prime_tower_rib_wall").is_none_or(|value| value == "1");
    let width = setting_f64(settings, "prime_tower_width")?;
    let filaments = settings.get("filament_colour")?.as_array()?.len();
    let nozzles = settings.get("nozzle_diameter").and_then(|v| v.as_array()).map_or(1, Vec::len);
    let mut wipe_volume =
        setting_all_f64(settings, "filament_prime_volume").into_iter().fold(0.0, f64::max);
    if setting_str(settings, "prime_volume_mode") == Some("Saving") {
        wipe_volume = 15.0;
    }
    // `timelapse_type` 1 = smooth: the GUI then shows a tower even for one filament.
    let need_tower = setting_str(settings, "timelapse_type") == Some("1")
        || setting_str(settings, "enable_wrapping_detection") == Some("1");
    let mut volume = wipe_volume * if nozzles == 2 { filaments } else { filaments.saturating_sub(1) } as f64;
    if nozzles == 2 {
        let length = setting_all_f64(settings, "filament_change_length").into_iter().fold(0.0, f64::max);
        let diameter = setting_all_f64(settings, "filament_diameter").into_iter().fold(0.0, f64::max);
        let diameter = if diameter > 0.0 { diameter } else { 1.75 };
        volume += length * std::f64::consts::PI * diameter * diameter / 4.0 * (filaments / 2) as f64;
    }
    if layer_height <= 0.0 {
        return None;
    }
    let min_depth = limit_depth_by_height(object_height);
    if rib_wall {
        if !need_tower && filaments < 2 {
            return None;
        }
        let volume_depth = (volume / layer_height * extra_spacing).sqrt();
        let depth = volume_depth.max(min_depth);
        let rib_width = setting_f64(settings, "prime_tower_rib_width").unwrap_or(8.0).min(depth / 2.0);
        let extra_rib = setting_f64(settings, "prime_tower_extra_rib_length").unwrap_or(0.0);
        let side = rib_width / std::f64::consts::SQRT_2 + (depth + extra_rib).max(volume_depth);
        Some((side, side))
    } else {
        let depth = volume / (layer_height * width) * extra_spacing;
        if !need_tower && depth <= 1e-9 {
            return None;
        }
        Some((width, depth.max(min_depth)))
    }
}

/// Where the slicer GUI keeps a stored tower position (`wipe_tower_x/y`, the tower's min
/// corner) on open: `[plate min + margin, plate max − estimated size − margin]` per axis.
/// `None` when the range is empty (the GUI would pin the tower anyway) or no estimate exists.
pub fn gui_tower_anchor_range(
    settings: &serde_json::Value,
    slicer_kind: &str,
    plate: Rect,
    object_height: f64,
) -> Option<Rect> {
    let (size_x, size_y) = gui_tower_estimate(settings, object_height)?;
    let margin = gui_tower_margin_mm(slicer_kind);
    let range = Rect::new(
        plate.min_x + margin,
        plate.min_y + margin,
        plate.max_x - size_x - margin,
        plate.max_y - size_y - margin,
    );
    (range.min_x <= range.max_x && range.min_y <= range.max_y).then_some(range)
}

/// The stored tower position of plate 1 (`wipe_tower_x[0]`, `wipe_tower_y[0]`).
pub fn wipe_tower_anchor(settings: &serde_json::Value) -> Result<(f64, f64), String> {
    Ok((first_entry(settings, "wipe_tower_x")?, first_entry(settings, "wipe_tower_y")?))
}

/// Grid step of the free-spot search and of its occupancy raster.
const FREE_SPOT_STEP_MM: f64 = 0.5;

/// How many of the nearest raster-free spots get the exact check before giving up: the raster
/// is conservative, so the first one almost always passes.
const FREE_SPOT_EXACT_CHECKS: usize = 64;

/// The free place nearest to `current` (by min corner, which `wipe_tower_x/y` anchors) for a
/// tower keepout of `size` inside `area`: it must keep `clearance` from every object's loops
/// (the full patch, supports included) and must not overlap a fixed keepout. `None` when no
/// such place exists.
///
/// The objects are rasterized at `FREE_SPOT_STEP_MM` and dilated by the clearance plus one
/// pixel (pixel-centre sampling misses up to half a diagonal), a summed-area table finds every
/// empty window, and the nearest windows are confirmed against the exact loops.
pub fn free_tower_spot(
    current: Rect,
    size: (f64, f64),
    area: Rect,
    objects: &[Vec<Vec<Point>>],
    fixed_keepouts: &[Rect],
    clearance: f64,
) -> Option<Rect> {
    let step = FREE_SPOT_STEP_MM;
    let columns = ((area.max_x - area.min_x) / step).floor() as usize;
    let rows = ((area.max_y - area.min_y) / step).floor() as usize;
    let (span_x, span_y) = ((size.0 / step).ceil() as usize, (size.1 / step).ceil() as usize);
    if columns < span_x || rows < span_y {
        return None;
    }
    let mut occupied = vec![false; columns * rows];
    for loops in objects {
        for row in 0..rows {
            let y = area.min_y + (row as f64 + 0.5) * step;
            let mut crossings: Vec<f64> = loops
                .iter()
                .flat_map(|loop_| loop_.windows(2))
                .filter(|edge| (edge[0].y > y) != (edge[1].y > y))
                .map(|edge| {
                    let (a, b) = (edge[0], edge[1]);
                    a.x + (b.x - a.x) * (y - a.y) / (b.y - a.y)
                })
                .collect();
            crossings.sort_by(f64::total_cmp);
            for pair in crossings.chunks_exact(2) {
                let first = ((pair[0] - area.min_x) / step - 0.5).ceil().max(0.0) as usize;
                let last = ((pair[1] - area.min_x) / step - 0.5).floor();
                if last < 0.0 {
                    continue;
                }
                for column in first..=(last as usize).min(columns - 1) {
                    occupied[row * columns + column] = true;
                }
            }
        }
    }
    let radius = (clearance / step).ceil() as usize + 1;
    let mut dilated = occupied.clone();
    for row in 0..rows {
        for column in 0..columns {
            if !occupied[row * columns + column] {
                continue;
            }
            for r in row.saturating_sub(radius)..(row + radius + 1).min(rows) {
                for c in column.saturating_sub(radius)..(column + radius + 1).min(columns) {
                    dilated[r * columns + c] = true;
                }
            }
        }
    }
    // Fixed keepouts block every pixel they touch (edge contact is not an overlap).
    for rect in fixed_keepouts {
        let c0 = ((rect.min_x - area.min_x) / step).floor().max(0.0) as usize;
        let c1 = (((rect.max_x - area.min_x) / step).ceil().max(0.0) as usize).min(columns);
        let r0 = ((rect.min_y - area.min_y) / step).floor().max(0.0) as usize;
        let r1 = (((rect.max_y - area.min_y) / step).ceil().max(0.0) as usize).min(rows);
        for r in r0..r1 {
            for c in c0..c1 {
                dilated[r * columns + c] = true;
            }
        }
    }
    let width = columns + 1;
    let mut sums = vec![0u32; width * (rows + 1)];
    for row in 0..rows {
        for column in 0..columns {
            sums[(row + 1) * width + column + 1] = u32::from(dilated[row * columns + column])
                + sums[row * width + column + 1]
                + sums[(row + 1) * width + column]
                - sums[row * width + column];
        }
    }
    let empty = |row: usize, column: usize| {
        let (r1, c1) = (row + span_y, column + span_x);
        sums[r1 * width + c1] + sums[row * width + column]
            == sums[row * width + c1] + sums[r1 * width + column]
    };
    let mut spots: Vec<(f64, Rect)> = Vec::new();
    for row in 0..=rows - span_y {
        for column in 0..=columns - span_x {
            if !empty(row, column) {
                continue;
            }
            let (x, y) = (area.min_x + column as f64 * step, area.min_y + row as f64 * step);
            let spot = Rect::new(x, y, x + size.0, y + size.1);
            if spot.max_x > area.max_x + 1e-9 || spot.max_y > area.max_y + 1e-9 {
                continue;
            }
            spots.push(((x - current.min_x).hypot(y - current.min_y), spot));
        }
    }
    spots.sort_by(|a, b| a.0.total_cmp(&b.0));
    spots
        .into_iter()
        .take(FREE_SPOT_EXACT_CHECKS)
        .map(|(_, spot)| spot)
        .find(|spot| {
            let reach = Rect::new(
                spot.min_x - clearance,
                spot.min_y - clearance,
                spot.max_x + clearance,
                spot.max_y + clearance,
            );
            ensure_relocated_tower_clear(*spot, fixed_keepouts).is_ok()
                && !objects.iter().any(|loops| crate::loops_touch_rect(loops, reach))
        })
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

    #[test]
    fn gui_estimate_matches_bambu_studio_readout() {
        let (x, y) = gui_tower_estimate(&goat_settings(), 44.74).unwrap();
        assert!((x - 17.16).abs() < 0.01 && (y - 17.16).abs() < 0.01, "{x} {y}");
        let range = gui_tower_anchor_range(&goat_settings(), "bambu", Rect::new(0.0, 0.0, 256.0, 256.0), 44.74)
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
    }

    #[test]
    fn orca_keeps_one_millimetre() {
        let range = gui_tower_anchor_range(&goat_settings(), "orca", Rect::new(0.0, 0.0, 256.0, 256.0), 44.74)
            .unwrap();
        assert!((range.min_x - 1.0).abs() < 1e-9 && (range.max_x - (256.0 - 17.16 - 1.0)).abs() < 0.01);
    }
}

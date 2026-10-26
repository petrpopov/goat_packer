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

/// Allowed distance between the measured final tower centre and the relocated keepout centre.
/// A slicer that ignored the relocation leaves the tower at the source position, which is the
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

/// Plate centre the objects and the tower gather around: the centre of the first (smallest)
/// heat zone, or of the usable bed when the printer has no zones.
pub fn plate_centre(zones: &[HeatZone], usable_bed: Rect) -> (f64, f64) {
    rect_centre(zones.first().map_or(usable_bed, HeatZone::as_rect))
}

/// A tower keepout moved towards a target centre.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
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

/// Distance between the centres of a measured tower keepout and the target keepout.
pub fn tower_centre_error_mm(measured: Rect, target: Rect) -> f64 {
    let (mx, my) = rect_centre(measured);
    let (tx, ty) = rect_centre(target);
    (mx - tx).hypot(my - ty)
}

//! Centre-out plates, heat zones and prime-tower relocation for time-window plates
//! (spec `docs/superpowers/specs/2026-10-26-centre-out-zones-tower-design.md`).
//!
//! Pure geometry plus the one `project_settings.config` edit the packer is allowed to make:
//! moving the wipe tower of plate 1. Any subset of a valid layout is valid, so "centre-out"
//! needs no new search — the packer orders the full-bed layout by distance from the centre and
//! the time window keeps a prefix of that order.

mod free_spot;
mod gui_tower;
mod wipe_tower;

use serde::{Deserialize, Serialize};

use crate::geometry::{Point, Rect};

pub use free_spot::free_tower_spot;
pub use gui_tower::{gui_tower_anchor_range, gui_tower_estimate};
pub use wipe_tower::{relocate_wipe_tower_in_3mf, shift_wipe_tower_settings, wipe_tower_anchor};

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
    (
        (rect.min_x + rect.max_x) * 0.5,
        (rect.min_y + rect.max_y) * 0.5,
    )
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
    let dx = clamp_shift(
        target.0 - cx,
        keepout.min_x,
        keepout.max_x,
        bed.min_x,
        bed.max_x,
    );
    let dy = clamp_shift(
        target.1 - cy,
        keepout.min_y,
        keepout.max_y,
        bed.min_y,
        bed.max_y,
    );
    RelocatedTower {
        keepout: keepout.translated(dx, dy),
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
    loops
        .iter()
        .flatten()
        .next()
        .map(|_| Rect::of_points(loops.iter().flatten()))
}

pub fn union_rect(a: Rect, b: Rect) -> Rect {
    a.union(b)
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

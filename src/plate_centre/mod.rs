//! Centre-out plates and heat zones for time-window plates (spec
//! `docs/superpowers/specs/2026-10-26-centre-out-zones-tower-design.md`), and the one
//! `project_settings.config` edit the packer is allowed to make: placing the wipe tower of
//! plate 1. Any subset of a valid layout is valid, so "centre-out"
//! needs no new search — the packer orders the full-bed layout by distance from the centre and
//! the time window keeps a prefix of that order.

mod gui_tower;
mod wipe_tower;

use serde::{Deserialize, Serialize};

use crate::geometry::{Point, Rect};

pub use gui_tower::{gui_tower_anchor_range, gui_tower_estimate};
pub use wipe_tower::{relocate_wipe_tower_in_3mf, shift_wipe_tower_settings, wipe_tower_anchor};

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


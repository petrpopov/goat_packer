//! Tower anchors to try (design 2026-11-02, §9.3): the source anchor, the corners of the
//! range the tower may take, and positions flush with each side of that range every
//! `SCAN_STEP_MM`. The tower body is the largest thing the packer moves, and edges are where
//! a lattice's ragged margin leaves room for it.
//!
//! The range is the spec's `anchor_range` as given (the GUI re-clamps a stored anchor into
//! it when it opens the 3MF, so anchors outside it would not survive), narrowed to where the
//! tower stays inside the printable area. Anchors that put a tower part on a zone that
//! forbids it (body on a hard zone, layer-0 brim on a soft zone) are dropped.

use super::tower::TowerSpec;
use crate::geometry::{Point, Rect};
use crate::kind_masks::{
    FeasibleOrigins, KindMasks, OriginGrid, OriginSet, PlateZone, ZoneArea, forbidden_origins,
};
use crate::span_raster::SpanRaster;
use rayon::prelude::*;

/// Spacing of the scanned anchors along a side of the range, mm.
pub const SCAN_STEP_MM: f64 = 2.0;

/// The anchors in tie order: the source (clamped into the range, where the GUI would put
/// it), the four corners of the fitting range, then each side from its min corner in steps
/// of `SCAN_STEP_MM` (the last step lands on the side's end). No duplicates; `Err` when no
/// anchor is conflict-free.
pub fn scan_anchors(spec: &TowerSpec, plate: &[PlateZone]) -> Result<Vec<Point>, String> {
    spec.check()?;
    let range = spec.anchor_range;
    let source = Point {
        x: spec.source_anchor.x.clamp(range.min_x, range.max_x),
        y: spec.source_anchor.y.clamp(range.min_y, range.max_y),
    };
    let mut anchors = vec![source];
    if let Some(fit) = spec.fit_range(plate) {
        let corners = [
            (fit.min_x, fit.min_y),
            (fit.max_x, fit.min_y),
            (fit.min_x, fit.max_y),
            (fit.max_x, fit.max_y),
        ];
        anchors.extend(corners.map(|(x, y)| Point { x, y }));
        anchors.extend(side_positions(fit));
    }
    let mut unique: Vec<Point> = Vec::new();
    let mut conflicts = Vec::new();
    for anchor in anchors {
        if unique.contains(&anchor) {
            continue;
        }
        match spec.conflict(anchor, plate) {
            None => unique.push(anchor),
            Some(why) => conflicts.push(why),
        }
    }
    if unique.is_empty() {
        return Err(format!(
            "no tower anchor fits the plate: {}",
            conflicts.join("; ")
        ));
    }
    Ok(unique)
}

/// Positions flush with the four sides of `range`: bottom, top, left, right.
fn side_positions(range: Rect) -> Vec<Point> {
    let along = |from: f64, to: f64| -> Vec<f64> {
        let steps = ((to - from) / SCAN_STEP_MM).floor() as usize;
        let mut values: Vec<f64> = (0..=steps)
            .map(|k| from + k as f64 * SCAN_STEP_MM)
            .collect();
        if values.last().is_some_and(|&v| v < to) {
            values.push(to);
        }
        values
    };
    let xs = along(range.min_x, range.max_x);
    let ys = along(range.min_y, range.max_y);
    let mut points = Vec::new();
    for &y in &[range.min_y, range.max_y] {
        points.extend(xs.iter().map(|&x| Point { x, y }));
    }
    for &x in &[range.min_x, range.max_x] {
        points.extend(ys.iter().map(|&y| Point { x, y }));
    }
    points
}

/// Anchors whose pixel phase (the fraction of a pixel their tower sits off the origin grid)
/// agrees to this many pixels share one forbidden set, shifted by whole pixels.
const PHASE_QUANTUM: f64 = 1e-6;

/// The origins each tower position forbids each class, `[anchor][class]`, inside `bases`.
///
/// Building a set costs a conservative raster test of every tower part against every kind
/// of the class, far more than the hundreds of anchors of a scan can afford one by one. A
/// tower moved by a whole number of pixels forbids the same origins moved by the same
/// amount, so one set is built per (pixel phase, class) in a window around the first anchor
/// of the phase and moved to the others, then cut to the class's base map (the printable
/// area bounds the base map, never the tower's set).
pub(crate) fn tower_origin_sets(
    masks: &[KindMasks],
    bases: &[FeasibleOrigins],
    grid: &OriginGrid,
    anchors: &[(Point, Vec<PlateZone>)],
) -> Vec<Vec<OriginSet>> {
    let phase = |a: Point| {
        let frac = |v: f64, origin: f64| {
            let px = (v - origin) / grid.resolution;
            let frac = px - px.floor();
            ((frac / PHASE_QUANTUM).round() as i64) % (1.0 / PHASE_QUANTUM) as i64
        };
        (frac(a.x, grid.origin.x), frac(a.y, grid.origin.y))
    };
    // The first anchor of each phase is its reference, in anchor order.
    let mut references: Vec<usize> = Vec::new();
    let mut reference_of: Vec<usize> = Vec::new();
    for (index, (anchor, _)) in anchors.iter().enumerate() {
        let found = references
            .iter()
            .position(|&r| phase(anchors[r].0) == phase(*anchor));
        reference_of.push(found.unwrap_or_else(|| {
            references.push(index);
            references.len() - 1
        }));
    }
    let units: Vec<(usize, usize)> = (0..references.len())
        .flat_map(|r| (0..masks.len()).map(move |c| (r, c)))
        .collect();
    let wide: Vec<SpanRaster> = units
        .par_iter()
        .map(|&(r, class)| {
            let zones = &anchors[references[r]].1;
            let window = window_around(&masks[class], zones, grid);
            forbidden_origins(&masks[class], zones, grid, &window).spans
        })
        .collect();
    anchors
        .iter()
        .enumerate()
        .map(|(index, (anchor, _))| {
            let r = reference_of[index];
            let reference = anchors[references[r]].0;
            let shift = |from: f64, to: f64| ((to - from) / grid.resolution).round() as i32;
            let (dx, dy) = (shift(reference.x, anchor.x), shift(reference.y, anchor.y));
            (0..masks.len())
                .map(|class| OriginSet {
                    spans: restrict(
                        &moved(&wide[r * masks.len() + class], dx, dy),
                        &bases[class].allowed,
                    ),
                })
                .collect()
        })
        .collect()
}

/// Every origin of `masks` whose copy could meet the tower `zones`: the zones' box in pixels
/// grown by the class's reach, so the set built in it is the whole set.
fn window_around(masks: &KindMasks, zones: &[PlateZone], grid: &OriginGrid) -> FeasibleOrigins {
    let rects = zones.iter().filter_map(|z| match z.area {
        ZoneArea::Rect(r) => Some(r),
        _ => None,
    });
    let bounds = rects.reduce(Rect::union).expect("a tower has a body zone");
    let margin = masks.reach_px() + WINDOW_MARGIN_PX;
    let px = |v: f64, origin: f64| ((v - origin) / grid.resolution).floor() as i32;
    FeasibleOrigins {
        allowed: SpanRaster::rect(
            px(bounds.min_x, grid.origin.x) - margin,
            px(bounds.min_y, grid.origin.y) - margin,
            px(bounds.max_x, grid.origin.x) + margin,
            px(bounds.max_y, grid.origin.y) + margin,
        ),
    }
}

/// Slack around the reach of a class: gaps and a pixel of rounding.
const WINDOW_MARGIN_PX: i32 = 8;

fn moved(set: &SpanRaster, dx: i32, dy: i32) -> SpanRaster {
    SpanRaster {
        min_y: set.min_y + dy,
        rows: set
            .rows
            .iter()
            .map(|row| row.iter().map(|&(a, b)| (a + dx, b + dx)).collect())
            .collect(),
    }
}

/// The pixels of `set` that `base` has.
fn restrict(set: &SpanRaster, base: &SpanRaster) -> SpanRaster {
    let rows = set
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let y = set.min_y + index as i32;
            let mut out = Vec::new();
            for &(a, b) in row {
                for &(c, d) in base.row(y) {
                    let (start, end) = (a.max(c), b.min(d));
                    if start < end {
                        out.push((start, end));
                    }
                }
            }
            out
        })
        .collect();
    SpanRaster {
        min_y: set.min_y,
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keepout::PlateRules;
    use crate::kind_masks::{KindShapes, MaskRules, feasible_origins, plate_zones};
    use crate::object_print::ObjectPrint;

    fn spec() -> TowerSpec {
        TowerSpec {
            body: Rect::new(-0.25, -0.25, 9.0, 7.0),
            brim_l0_mm: 2.0,
            chamfer: Vec::new(),
            top_z_mm: 5.0,
            anchor_range: Rect::new(5.0, 5.0, 61.0, 41.0),
            source_anchor: Point { x: 100.0, y: 3.0 },
        }
    }

    fn zones() -> Vec<PlateZone> {
        let rules = PlateRules {
            printable_area: Rect::new(0.0, 0.0, 80.0, 60.0),
            body_edge_band_mm: 0.0,
            print_edge_reserve_mm: 0.0,
            keepouts: Vec::new(),
            body_rule: Default::default(),
        };
        plate_zones(&rules, None).unwrap()
    }

    #[test]
    fn the_source_comes_first_clamped_then_corners_then_the_sides() {
        let anchors = scan_anchors(&spec(), &zones()).unwrap();
        assert_eq!(anchors[0], Point { x: 61.0, y: 5.0 });
        // The fit range is the spec range here (the plate is roomy): corners follow.
        assert!(anchors[1..4].contains(&Point { x: 5.0, y: 41.0 }));
        assert!(
            anchors.contains(&Point { x: 31.0, y: 5.0 }),
            "bottom side, every 2 mm"
        );
        assert!(
            anchors.contains(&Point { x: 61.0, y: 41.0 }),
            "side ends land exactly"
        );
        assert!(
            anchors
                .iter()
                .all(|a| a.x == 5.0 || a.x == 61.0 || a.y == 5.0 || a.y == 41.0)
        );
        for (i, a) in anchors.iter().enumerate() {
            assert!(!anchors[..i].contains(a), "no duplicates");
        }
    }

    #[test]
    fn anchors_that_leave_the_plate_are_dropped() {
        let mut spec = spec();
        spec.anchor_range = Rect::new(-10.0, 5.0, 100.0, 41.0);
        let anchors = scan_anchors(&spec, &zones()).unwrap();
        for a in &anchors {
            assert!(
                a.x - 0.25 - 2.0 >= -1e-9 && a.x + 9.0 + 2.0 <= 80.0 + 1e-9,
                "{a:?}"
            );
        }
    }

    /// A small print with a body, a landing and a support column.
    fn print() -> ObjectPrint {
        let ring = |x0: f64, y0: f64, x1: f64, y1: f64| {
            [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
                .iter()
                .map(|&(x, y)| Point { x, y })
                .collect::<Vec<_>>()
        };
        ObjectPrint {
            silhouette: vec![ring(-3.0, -2.0, 3.0, 2.0)],
            part_hulls: vec![ring(-3.0, -2.0, 3.0, 2.0)],
            part_boxes: vec![Rect::new(-3.0, -2.0, 3.0, 2.0)],
            landing: vec![ring(-3.0, -2.0, 3.0, 2.0)],
            support_base: vec![ring(-4.0, -3.0, 4.0, 3.0)],
            body: vec![
                vec![ring(-3.0, -2.0, 3.0, 2.0)],
                vec![ring(-3.0, -2.0, 3.0, 2.0)],
            ],
            support: vec![vec![], vec![ring(3.0, -1.0, 5.0, 0.0)]],
            interface: vec![vec![], vec![]],
            ..ObjectPrint::default()
        }
    }

    #[test]
    fn shifted_sets_equal_sets_built_at_each_anchor() {
        let zones_plate = zones();
        let grid = OriginGrid {
            origin: Point { x: 0.0, y: 0.0 },
            resolution: 0.5,
        };
        let rules = MaskRules {
            clearance_mm: 0.5,
            foreign: Default::default(),
            body_rule: Default::default(),
            support_overlap_mm: None,
        };
        let masks = vec![KindShapes::new(&print(), rules).rasterize(0.5)];
        let bases = vec![feasible_origins(&masks[0], &zones_plate, &grid)];
        let spec = spec();
        // Whole-pixel steps (0.5 mm), a different phase (0.2), then another whole step of it.
        let points = [
            (5.0, 5.0),
            (7.5, 9.0),
            (5.2, 5.0),
            (30.2, 17.5),
            (61.0, 41.0),
        ];
        let anchors: Vec<(Point, Vec<PlateZone>)> = points
            .iter()
            .map(|&(x, y)| {
                let a = Point { x, y };
                (a, spec.zones(a))
            })
            .collect();
        let sets = tower_origin_sets(&masks, &bases, &grid, &anchors);
        for (index, (_, zones)) in anchors.iter().enumerate() {
            let direct = forbidden_origins(&masks[0], zones, &grid, &bases[0]);
            let mut got = sets[index][0].spans.clone();
            let mut want = direct.spans;
            // Compare as pixel sets whatever the row range.
            let rows = |s: &mut SpanRaster| {
                let (lo, hi) = (-200, 200);
                (lo..hi).map(|y| s.row(y).to_vec()).collect::<Vec<_>>()
            };
            assert_eq!(
                rows(&mut got),
                rows(&mut want),
                "anchor {:?}",
                points[index]
            );
            assert!(want.pixel_count() > 0);
        }
    }
}

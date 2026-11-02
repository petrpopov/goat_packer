//! Exact integer booleans for the mesh kinds: mm loops onto the 1 nm grid the support model
//! uses (spec 01 §0: upstream scales mm by 10^6), i_overlay on that grid, and back.
//!
//! S2's `polygon_ops` facade is meant to own every boolean; until it lands, the few the mesh
//! stage needs live here, single-threaded and on integers so results never depend on the
//! machine (design §5.2 rules).

use i_overlay::core::{
    fill_rule::FillRule,
    overlay::{IntOverlayOptions, Overlay, ShapeType},
    overlay_rule::OverlayRule,
    solver::{Precision, Solver, Strategy},
};
use i_overlay::i_float::int::point::IntPoint;

use crate::geometry::Point;
use crate::object_print::Loops;

/// Grid steps per mm: 1 nm, the Orca grid (spec 01 §0).
pub const NM_PER_MM: f64 = 1.0e6;

/// One ring on the nm grid, not closed (no repeated first point).
pub type IntRing = Vec<IntPoint>;

/// Rings whose union under one fill rule is an area: shapes as `[outer, holes...]` flattened.
pub type IntRings = Vec<IntRing>;

/// No parallel sorting inside i_overlay: a member of the support ensemble runs on one thread
/// and must give the same bits on every machine (design §5.2).
const SOLVER: Solver = Solver {
    strategy: Strategy::Auto,
    precision: Precision::HIGH,
    multithreading: None,
};

/// mm to nm, rounded half up (spec 01 §1.3 step 1). Coordinates saturate at the i32 range,
/// a metre beyond any bed; i_overlay is exact within ±2^30 nm.
pub fn to_nm(value: f64) -> i32 {
    let scaled = libm::floor(value * NM_PER_MM + 0.5);
    scaled.clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

pub fn int_point(x: f64, y: f64) -> IntPoint {
    IntPoint::new(to_nm(x), to_nm(y))
}

/// Twice the signed area (positive for counter-clockwise), exact.
pub fn doubled_area(ring: &[IntPoint]) -> i128 {
    let count = ring.len();
    (0..count)
        .map(|index| {
            let a = ring[index];
            let b = ring[(index + 1) % count];
            i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y)
        })
        .sum()
}

/// The ring counter-clockwise; `None` when it has no area on the grid.
pub fn counter_clockwise(mut ring: IntRing) -> Option<IntRing> {
    ring.dedup();
    while ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    let area = doubled_area(&ring);
    if ring.len() < 3 || area == 0 {
        return None;
    }
    if area < 0 {
        ring.reverse();
    }
    Some(ring)
}

/// Resolves `rings` under `fill`: outer rings counter-clockwise, holes clockwise.
pub fn resolve(rings: &[IntRing], fill: FillRule) -> IntRings {
    combine(rings, &[], OverlayRule::Subject, fill)
}

/// `subject \ clip`, both under the non-zero rule.
pub fn difference(subject: &[IntRing], clip: &[IntRing]) -> IntRings {
    combine(subject, clip, OverlayRule::Difference, FillRule::NonZero)
}

fn combine(subject: &[IntRing], clip: &[IntRing], rule: OverlayRule, fill: FillRule) -> IntRings {
    let edges: usize = subject.iter().chain(clip).map(Vec::len).sum();
    if edges == 0 {
        return IntRings::new();
    }
    let mut overlay = Overlay::new_custom(edges, IntOverlayOptions::default(), SOLVER);
    for ring in subject.iter().filter(|ring| ring.len() >= 3) {
        overlay.add_contour(ring, ShapeType::Subject);
    }
    for ring in clip.iter().filter(|ring| ring.len() >= 3) {
        overlay.add_contour(ring, ShapeType::Clip);
    }
    overlay.overlay(rule, fill).into_iter().flatten().collect()
}

/// mm loops (closed or not, any orientation) as grid rings, each kept as given.
pub fn rings_from_loops(loops: &[Vec<Point>]) -> IntRings {
    loops
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|point| int_point(point.x, point.y))
                .collect()
        })
        .collect()
}

/// Grid rings as closed mm loops (first point repeated), the `object_print::Loops` form.
pub fn loops_from_rings(rings: &[IntRing]) -> Loops {
    rings
        .iter()
        .filter(|ring| ring.len() >= 3)
        .map(|ring| {
            let mut points: Vec<Point> = ring
                .iter()
                .map(|point| Point {
                    x: f64::from(point.x) / NM_PER_MM,
                    y: f64::from(point.y) / NM_PER_MM,
                })
                .collect();
            points.push(points[0]);
            points
        })
        .collect()
}

/// Union of even-odd loop sets (each set one area), as closed mm loops.
pub fn union_of_areas<'a>(areas: impl Iterator<Item = &'a Loops>) -> Loops {
    let mut rings = IntRings::new();
    for area in areas {
        rings.extend(resolve(&rings_from_loops(area), FillRule::EvenOdd));
    }
    loops_from_rings(&resolve(&rings, FillRule::NonZero))
}

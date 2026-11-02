//! Offsets with the semantics of the Clipper 1 library as the slicers call it (spec 02 §1.1,
//! spec 01 §0). The rules that change discrete decisions downstream are kept:
//!
//! - the delta is an f32 (every slicer offset takes `float delta`);
//! - each ring is offset on its own, after dropping vertices closer than 0.005·|d| to the last
//!   kept one (the slicers' fork of Clipper), and resolved with the positive rule; a clockwise
//!   ring (a hole) is offset with the opposite sign and stays a hole;
//! - mitre limits below 2 act as 2, so a requested 1.2 mitres out to 2·d and squares beyond;
//! - round joins use the arc tolerance τ' = min(τ, |d|/4) and π / acos(1 − τ'/|d|) segments per
//!   half turn, capped at π·|d|;
//! - the per-ring results are united with the non-zero rule (growing) or the positive rule
//!   (shrinking, which is what the slicers' bounding-frame trick computes).
//!
//! The corner construction follows Clipper's published geometry (Boost licence, design §1).

use i_overlay::core::fill_rule::FillRule;

use super::boolean::resolve;
use super::query::ring_doubled_area;
use super::{IntPoint, MAX_COORDINATE, Ring, Rings};

/// The slicers' default mitre limit (`DefaultMiterLimit`).
pub const DEFAULT_MITER_LIMIT: f64 = 3.0;

/// Clipper's arc tolerance when none is given, in grid units.
const DEFAULT_ARC_TOLERANCE: f64 = 0.25;

/// Input decimation of the slicers' Clipper fork: vertices closer than this fraction of |d|
/// to the previously kept one are dropped before offsetting (O `ClipperUtils.hpp:34`).
const SHORTEST_EDGE_FACTOR: f64 = 0.005;

/// How convex corners of the offset are closed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Join {
    /// Mitred out to `limit`·|d| from the vertex, cut square beyond; limits below 2 act as 2.
    Miter(f64),
    /// Cut square at |d| from the vertex (`jtSquare`).
    Square,
    /// Arcs within this tolerance in grid units; ≤ 0 means 0.25 units.
    Round(f64),
}

/// Grows (`delta` > 0) or shrinks every ring of `rings` by `delta` grid units. Holes must be
/// clockwise and outer rings counter-clockwise, as every result of this module is.
pub fn offset(rings: &[Ring], delta: f64, join: Join) -> Rings {
    let delta = f64::from(delta as f32);
    if delta.abs() < 1e-20 {
        return resolve(rings, FillRule::Positive);
    }
    let shortest = (delta * SHORTEST_EDGE_FACTOR).abs();
    let mut parts = Rings::new();
    for ring in rings {
        let Some(mut contour) = decimate(ring, shortest) else {
            continue;
        };
        let counter_clockwise = ring_doubled_area(ring) >= 0;
        let signed = if counter_clockwise { delta } else { -delta };
        if ring_doubled_area(&contour) < 0 {
            contour.reverse();
        }
        let raw = offset_contour(&contour, signed, join);
        let mut resolved = resolve(&[raw], FillRule::Positive);
        if !counter_clockwise {
            resolved.iter_mut().for_each(|ring| ring.reverse());
        }
        parts.extend(resolved);
    }
    let fill = if delta > 0.0 {
        FillRule::NonZero
    } else {
        FillRule::Positive
    };
    resolve(&parts, fill)
}

/// Drops vertices closer than `shortest` to the last kept one, and trailing vertices that
/// close to the first; `None` when fewer than three remain.
fn decimate(ring: &[IntPoint], shortest: f64) -> Option<Ring> {
    let limit = shortest * shortest;
    let same = |a: IntPoint, b: IntPoint| {
        if shortest > 0.0 {
            let (dx, dy) = (f64::from(b.x - a.x), f64::from(b.y - a.y));
            dx * dx + dy * dy < limit
        } else {
            a == b
        }
    };
    let first = *ring.first()?;
    let mut last = ring.len() - 1;
    while last > 0 && same(first, ring[last]) {
        last -= 1;
    }
    let mut kept = vec![first];
    for &point in &ring[1..=last] {
        if !same(*kept.last().expect("starts with one point"), point) {
            kept.push(point);
        }
    }
    (kept.len() >= 3).then_some(kept)
}

/// Corner parameters shared by every vertex of one offset.
struct Corners {
    delta: f64,
    join: Join,
    /// 2 / limit², with the floor of 2 applied.
    miter_bound: f64,
    /// Rotation by one round-join step.
    step_sin: f64,
    step_cos: f64,
    steps_per_radian: f64,
}

impl Corners {
    fn new(delta: f64, join: Join) -> Corners {
        let miter_bound = match join {
            Join::Miter(limit) if limit > 2.0 => 2.0 / (limit * limit),
            _ => 0.5,
        };
        let tolerance = match join {
            Join::Round(tolerance) => tolerance,
            _ => 0.0,
        };
        let tolerance = if tolerance <= 0.0 {
            DEFAULT_ARC_TOLERANCE
        } else if tolerance > delta.abs() * DEFAULT_ARC_TOLERANCE {
            delta.abs() * DEFAULT_ARC_TOLERANCE
        } else {
            tolerance
        };
        let pi = std::f64::consts::PI;
        let mut steps = pi / libm::acos(1.0 - tolerance / delta.abs());
        if steps > delta.abs() * pi {
            steps = delta.abs() * pi;
        }
        let turn = 2.0 * pi / steps;
        let step_sin = libm::sin(turn);
        Corners {
            delta,
            join,
            miter_bound,
            step_sin: if delta < 0.0 { -step_sin } else { step_sin },
            step_cos: libm::cos(turn),
            steps_per_radian: steps / (2.0 * pi),
        }
    }
}

/// The raw offset ring of one counter-clockwise contour (before resolving self-overlaps).
fn offset_contour(contour: &[IntPoint], delta: f64, join: Join) -> Ring {
    let corners = Corners::new(delta, join);
    let count = contour.len();
    let normals: Vec<[f64; 2]> = (0..count)
        .map(|index| unit_normal(contour[index], contour[(index + 1) % count]))
        .collect();
    let mut out = Ring::with_capacity(count * 2);
    let mut incoming = count - 1;
    for vertex in 0..count {
        let point = [f64::from(contour[vertex].x), f64::from(contour[vertex].y)];
        if offset_vertex(
            &corners,
            point,
            normals[incoming],
            normals[vertex],
            &mut out,
        ) {
            incoming = vertex;
        }
    }
    out
}

/// Right-hand unit normal of a → b (the outside of a counter-clockwise ring).
fn unit_normal(a: IntPoint, b: IntPoint) -> [f64; 2] {
    if a == b {
        return [0.0, 0.0];
    }
    let (dx, dy) = (
        f64::from(b.x) - f64::from(a.x),
        f64::from(b.y) - f64::from(a.y),
    );
    let scale = 1.0 / libm::sqrt(dx * dx + dy * dy);
    [dy * scale, -dx * scale]
}

/// Emits the offset points of one vertex between edges with normals `n1` (incoming) and
/// `n2` (outgoing). Returns false for a nearly straight vertex that keeps the incoming
/// normal for the next vertex (Clipper does not advance it there).
fn offset_vertex(
    corners: &Corners,
    point: [f64; 2],
    n1: [f64; 2],
    n2: [f64; 2],
    out: &mut Ring,
) -> bool {
    let delta = corners.delta;
    let along =
        |normal: [f64; 2]| grid_point(point[0] + normal[0] * delta, point[1] + normal[1] * delta);
    let mut sin_a = n1[0] * n2[1] - n2[0] * n1[1];
    let cos_a = n1[0] * n2[0] + n2[1] * n1[1];
    if (sin_a * delta).abs() < 1.0 {
        if cos_a > 0.0 {
            out.push(along(n1));
            return false;
        }
    } else {
        sin_a = sin_a.clamp(-1.0, 1.0);
    }
    if sin_a * delta < 0.0 {
        out.push(along(n1));
        out.push(grid_point(point[0], point[1]));
        out.push(along(n2));
        return true;
    }
    match corners.join {
        Join::Miter(_) => {
            let ratio = 1.0 + (n2[0] * n1[0] + n2[1] * n1[1]);
            if ratio >= corners.miter_bound {
                let scale = delta / ratio;
                out.push(grid_point(
                    point[0] + (n1[0] + n2[0]) * scale,
                    point[1] + (n1[1] + n2[1]) * scale,
                ));
            } else {
                square_corner(point, n1, n2, sin_a, delta, out);
            }
        }
        Join::Square => square_corner(point, n1, n2, sin_a, delta, out),
        Join::Round(_) => round_corner(corners, point, n1, n2, sin_a, out),
    }
    true
}

/// Two points cutting the corner square at |d| from the vertex.
fn square_corner(
    point: [f64; 2],
    n1: [f64; 2],
    n2: [f64; 2],
    sin_a: f64,
    delta: f64,
    out: &mut Ring,
) {
    let cos_a = n1[0] * n2[0] + n1[1] * n2[1];
    let shift = libm::tan(libm::atan2(sin_a, cos_a) / 4.0);
    out.push(grid_point(
        point[0] + delta * (n1[0] - n1[1] * shift),
        point[1] + delta * (n1[1] + n1[0] * shift),
    ));
    out.push(grid_point(
        point[0] + delta * (n2[0] + n2[1] * shift),
        point[1] + delta * (n2[1] - n2[0] * shift),
    ));
}

/// An arc from the incoming to the outgoing normal in fixed angular steps.
fn round_corner(
    corners: &Corners,
    point: [f64; 2],
    n1: [f64; 2],
    n2: [f64; 2],
    sin_a: f64,
    out: &mut Ring,
) {
    let delta = corners.delta;
    let angle = libm::atan2(sin_a, n1[0] * n2[0] + n1[1] * n2[1]);
    let steps = (round_half_up(corners.steps_per_radian * angle.abs()) as i64).max(1);
    let (mut x, mut y) = (n1[0], n1[1]);
    for _ in 0..steps {
        out.push(grid_point(point[0] + x * delta, point[1] + y * delta));
        let previous = x;
        x = x * corners.step_cos - corners.step_sin * y;
        y = previous * corners.step_sin + y * corners.step_cos;
    }
    out.push(grid_point(
        point[0] + n2[0] * delta,
        point[1] + n2[1] * delta,
    ));
}

/// Clipper's rounding: half up, with its guard for the largest double below one half.
fn round_half_up(value: f64) -> f64 {
    if value == 0.499_999_999_999_999_94 {
        0.0
    } else {
        libm::floor(value + 0.5)
    }
}

fn grid_point(x: f64, y: f64) -> IntPoint {
    let bound = f64::from(MAX_COORDINATE);
    IntPoint::new(
        round_half_up(x).clamp(-bound, bound) as i32,
        round_half_up(y).clamp(-bound, bound) as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::polygon_ops::{area, bbox};

    fn square(size: i32) -> Ring {
        vec![
            IntPoint::new(0, 0),
            IntPoint::new(size, 0),
            IntPoint::new(size, size),
            IntPoint::new(0, size),
        ]
    }

    #[test]
    fn a_mitred_square_grows_into_a_square() {
        let grown = offset(&[square(1000)], 100.0, Join::Miter(3.0));
        assert_eq!(area(&grown), 1200.0 * 1200.0);
    }

    #[test]
    fn a_square_join_cuts_corners_at_the_offset_distance() {
        let grown = offset(&[square(1000)], 100.0, Join::Square);
        let found = bbox(&grown).unwrap();
        assert_eq!((found.min.x, found.max.x), (-100, 1100));
        // Each corner loses a triangle with legs 100·(2 − √2).
        let cut = 100.0 * (2.0 - std::f64::consts::SQRT_2);
        let expected = 1200.0 * 1200.0 - 4.0 * 0.5 * cut * cut;
        assert!((area(&grown) - expected).abs() < 200.0);
    }

    #[test]
    fn a_mitre_limit_below_two_acts_as_two() {
        // A 20-degree spike: mitre ratio far above both limits, so both are cut at 2·d.
        let spike = vec![
            IntPoint::new(0, 0),
            IntPoint::new(10_000, 1_763),
            IntPoint::new(0, 3_526),
        ];
        let low = offset(&[spike.clone()], 100.0, Join::Miter(1.2));
        let two = offset(&[spike], 100.0, Join::Miter(2.0));
        assert_eq!(low, two);
    }

    #[test]
    fn round_joins_follow_the_arc_tolerance() {
        let grown = offset(&[square(1000)], 1000.0, Join::Round(5.0));
        let exact = 3000.0 * 3000.0 - (4.0 - std::f64::consts::PI) * 1000.0 * 1000.0;
        // Chords sag at most 5 units inside the arc.
        assert!(area(&grown) <= exact);
        assert!(area(&grown) > exact - 4.0 * 1000.0 * 5.0 * std::f64::consts::FRAC_PI_2);
    }

    #[test]
    fn holes_shrink_when_the_area_grows() {
        let mut hole = vec![
            IntPoint::new(300, 300),
            IntPoint::new(700, 300),
            IntPoint::new(700, 700),
            IntPoint::new(300, 700),
        ];
        hole.reverse();
        let grown = offset(&[square(1000), hole], 100.0, Join::Miter(3.0));
        assert_eq!(area(&grown), 1200.0 * 1200.0 - 200.0 * 200.0);
        let shrunk = offset(&grown, -100.0, Join::Miter(3.0));
        assert_eq!(area(&shrunk), 1000.0 * 1000.0 - 400.0 * 400.0);
    }

    #[test]
    fn decimation_drops_vertices_closer_than_half_a_percent_of_the_delta() {
        let ring = vec![
            IntPoint::new(0, 0),
            IntPoint::new(1000, 0),
            IntPoint::new(1004, 2),
            IntPoint::new(1000, 1000),
            IntPoint::new(0, 1000),
        ];
        assert_eq!(decimate(&ring, 5.0).unwrap().len(), 4);
        assert_eq!(decimate(&ring, 4.0).unwrap().len(), 5);
    }
}

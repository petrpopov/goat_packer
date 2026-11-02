//! Mitred offsets on the nm grid and the morphological closing of a section (spec 01 §1.3
//! step 4: grow by r_c, shrink by r_c, mitred joins with limit 3).
//!
//! Each ring is moved along its edge normals; convex corners get a mitre (or, past the
//! limit, a square cut at the limit distance), reflex corners go through the original vertex
//! so the overlapping wedge cancels. The raw rings are then resolved with the positive rule,
//! which keeps exactly the points the offset covers. S2's `polygon_ops` will replace this
//! with its Clipper-compatible offset; the closing radius (0.049 mm) is far below every
//! margin, so only the topology (gaps narrower than 2·r_c close) matters here.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::i_float::int::point::IntPoint;

use super::area::{IntRing, IntRings, NM_PER_MM, resolve};

/// Mitre limit of the slicer's section closing, in multiples of the offset distance.
pub const CLOSING_MITER_LIMIT: f64 = 3.0;

/// Grows (`delta_mm` > 0) or shrinks a resolved area (outer rings counter-clockwise, holes
/// clockwise).
pub fn offset_area(rings: &IntRings, delta_mm: f64, miter_limit: f64) -> IntRings {
    if delta_mm == 0.0 {
        return rings.clone();
    }
    let delta = delta_mm * NM_PER_MM;
    let raw: IntRings = rings
        .iter()
        .map(|ring| offset_ring(ring, delta, miter_limit))
        .filter(|ring| ring.len() >= 3)
        .collect();
    resolve(&raw, FillRule::Positive)
}

/// Grow then shrink by `radius_mm`: fills gaps and notches narrower than twice the radius.
pub fn close_area(rings: &IntRings, radius_mm: f64) -> IntRings {
    if radius_mm <= 0.0 {
        return rings.clone();
    }
    let grown = offset_area(rings, radius_mm, CLOSING_MITER_LIMIT);
    offset_area(&grown, -radius_mm, CLOSING_MITER_LIMIT)
}

fn offset_ring(ring: &IntRing, delta: f64, miter_limit: f64) -> IntRing {
    let points: Vec<[f64; 2]> = ring
        .iter()
        .map(|point| [f64::from(point.x), f64::from(point.y)])
        .collect();
    let count = points.len();
    let normals: Vec<[f64; 2]> = (0..count)
        .map(|index| right_normal(points[index], points[(index + 1) % count]))
        .collect();
    let mut out = Vec::with_capacity(count * 2);
    for index in 0..count {
        let incoming = normals[(index + count - 1) % count];
        let outgoing = normals[index];
        corner(
            &mut out,
            points[index],
            incoming,
            outgoing,
            delta,
            miter_limit,
        );
    }
    out.dedup();
    out
}

/// Unit normal on the right of a→b: the outside of an area whose interior is on the left.
fn right_normal(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = libm::hypot(dx, dy);
    if length == 0.0 {
        return [0.0, 0.0];
    }
    [dy / length, -dx / length]
}

/// Offset points for one vertex between edges with normals `n1` (in) and `n2` (out).
fn corner(
    out: &mut IntRing,
    vertex: [f64; 2],
    n1: [f64; 2],
    n2: [f64; 2],
    delta: f64,
    miter_limit: f64,
) {
    let at = |n: [f64; 2], scale: f64| grid([vertex[0] + n[0] * scale, vertex[1] + n[1] * scale]);
    // n1 × n2 > 0 turns left (convex for the area); a shrink flips which side is convex.
    let turn = n1[0] * n2[1] - n1[1] * n2[0];
    let cosine = n1[0] * n2[0] + n1[1] * n2[1];
    if turn * delta < 0.0 {
        out.push(at(n1, delta));
        out.push(grid(vertex));
        out.push(at(n2, delta));
        return;
    }
    let one_plus_cos = 1.0 + cosine;
    // The mitre tip lies delta * sqrt(2 / (1 + cos)) from the vertex.
    if one_plus_cos >= 2.0 / (miter_limit * miter_limit) {
        let scale = delta / one_plus_cos;
        out.push(grid([
            vertex[0] + (n1[0] + n2[0]) * scale,
            vertex[1] + (n1[1] + n2[1]) * scale,
        ]));
        return;
    }
    square_cut(out, vertex, n1, n2, delta, miter_limit);
}

/// A mitre longer than the limit is cut square at `miter_limit * |delta|` from the vertex,
/// across the bisector.
fn square_cut(
    out: &mut IntRing,
    vertex: [f64; 2],
    n1: [f64; 2],
    n2: [f64; 2],
    delta: f64,
    miter_limit: f64,
) {
    let bisector = {
        let (x, y) = (n1[0] + n2[0], n1[1] + n2[1]);
        let length = libm::hypot(x, y);
        if length == 0.0 {
            n1
        } else {
            [x / length, y / length]
        }
    };
    let reach = miter_limit * delta;
    // Along each offset edge line, the point whose projection on the bisector is `reach`.
    for (normal, sign) in [(n1, 1.0), (n2, -1.0)] {
        let along = [-normal[1] * sign, normal[0] * sign];
        let base = [vertex[0] + normal[0] * delta, vertex[1] + normal[1] * delta];
        let base_reach = (base[0] - vertex[0]) * bisector[0] + (base[1] - vertex[1]) * bisector[1];
        let along_reach = along[0] * bisector[0] + along[1] * bisector[1];
        let step = if along_reach.abs() < 1e-12 {
            0.0
        } else {
            (reach - base_reach) / along_reach
        };
        out.push(grid([base[0] + along[0] * step, base[1] + along[1] * step]));
    }
}

fn grid(point: [f64; 2]) -> IntPoint {
    IntPoint::new(
        libm::floor(point[0] + 0.5) as i32,
        libm::floor(point[1] + 0.5) as i32,
    )
}

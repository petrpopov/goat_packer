//! Wall centre lines of the tower column (spec 04 §4.5) and their mitred outward offsets (the
//! brim and chamfer loops, 04 §4.6). Rings are closed (first point repeated), counter-clockwise,
//! in tower-local mm with the anchor corner at the origin.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::i_float::int::point::IntPoint;

use super::bambu::Rib;
use crate::geometry::{Point, Rect, polygon_area};
use crate::mesh_section::area::{IntRing, NM_PER_MM, loops_from_rings, resolve, rings_from_loops};

/// Corners turning by more than this and less than its supplement get a fillet (B fillet
/// wall, 04 §4.5 step 4).
const FILLET_MIN_TURN_DEG: f64 = 30.0;
/// Tangent length of a fillet, at most this and never past 1/2.1 of either edge.
const FILLET_TANGENT_MM: f64 = 2.0;
const FILLET_EDGE_FRACTION: f64 = 2.1;
/// Points sampled along each fillet arc before its end point.
const FILLET_SAMPLES: usize = 20;
/// Mitre limit of the brim loop offsets, in multiples of the offset (04 §4.6).
pub const BRIM_MITRE_LIMIT: f64 = 3.0;

/// The axis-aligned wall rectangle `[0, w] × [0, d]`.
pub fn rectangle(width: f64, depth: f64) -> Vec<Point> {
    closed(vec![
        Point { x: 0.0, y: 0.0 },
        Point { x: width, y: 0.0 },
        Point { x: width, y: depth },
        Point { x: 0.0, y: depth },
    ])
}

/// BambuStudio's rib wall on the first layer (04 §4.5): the box united with two strips of
/// width r along its diagonals, each lengthened at both ends to the rib length; optionally
/// filleted and united with the box again; then moved so its bbox minimum is the origin,
/// which is where the anchor puts the layer-0 wall (04 §4.7).
pub fn rib_outline(width: f64, depth: f64, rib: Rib, fillet: bool) -> Vec<Point> {
    let wall = rectangle(width, depth);
    let diagonal = libm::hypot(width, depth);
    let extension = (rib.length - diagonal).max(0.0) / 2.0;
    let strips = [
        strip([0.0, 0.0], [width, depth], extension, rib.width / 2.0),
        strip([width, 0.0], [0.0, depth], extension, rib.width / 2.0),
    ];
    let mut outline = outer_union(&[wall.clone(), strips[0].clone(), strips[1].clone()]);
    if fillet {
        outline = outer_union(&[filleted(&outline), wall]);
    }
    let bounds = Rect::of_points(outline.iter());
    outline
        .iter()
        .map(|p| Point {
            x: p.x - bounds.min_x,
            y: p.y - bounds.min_y,
        })
        .collect()
}

/// A rectangle of half-width `half` around the segment a→b lengthened by `extension` at both
/// ends, square-ended.
fn strip(a: [f64; 2], b: [f64; 2], extension: f64, half: f64) -> Vec<Point> {
    let length = libm::hypot(b[0] - a[0], b[1] - a[1]);
    let (ux, uy) = ((b[0] - a[0]) / length, (b[1] - a[1]) / length);
    let (start, end) = (
        [a[0] - ux * extension, a[1] - uy * extension],
        [b[0] + ux * extension, b[1] + uy * extension],
    );
    let (nx, ny) = (-uy * half, ux * half);
    counter_clockwise(closed(vec![
        Point {
            x: start[0] - nx,
            y: start[1] - ny,
        },
        Point {
            x: end[0] - nx,
            y: end[1] - ny,
        },
        Point {
            x: end[0] + nx,
            y: end[1] + ny,
        },
        Point {
            x: start[0] + nx,
            y: start[1] + ny,
        },
    ]))
}

/// The largest outer ring of the union of `rings`.
fn outer_union(rings: &[Vec<Point>]) -> Vec<Point> {
    let union = loops_from_rings(&resolve(&rings_from_loops(rings), FillRule::NonZero));
    union
        .into_iter()
        .max_by(|a, b| polygon_area(a).total_cmp(&polygon_area(b)))
        .unwrap_or_default()
}

/// Every corner turning between 30° and 150° replaced by a tangent circular arc: tangent
/// length min(2 mm, adjacent edges / 2.1), 20 samples from the first tangent point, then the
/// second. Convex corners lose material, reflex corners gain it.
fn filleted(ring: &[Point]) -> Vec<Point> {
    let points = &ring[..ring.len() - 1];
    let count = points.len();
    let mut out = Vec::with_capacity(count * (FILLET_SAMPLES + 1));
    let limit = libm::cos(FILLET_MIN_TURN_DEG.to_radians());
    for index in 0..count {
        let a = points[(index + count - 1) % count];
        let b = points[index];
        let c = points[(index + 1) % count];
        let (ab, bc) = (
            libm::hypot(b.x - a.x, b.y - a.y),
            libm::hypot(c.x - b.x, c.y - b.y),
        );
        let u = ((b.x - a.x) / ab, (b.y - a.y) / ab);
        let v = ((c.x - b.x) / bc, (c.y - b.y) / bc);
        let cosine = (u.0 * v.0 + u.1 * v.1).clamp(-1.0, 1.0);
        if cosine.abs() >= limit {
            out.push(b);
            continue;
        }
        let tangent = FILLET_TANGENT_MM
            .min(ab / FILLET_EDGE_FRACTION)
            .min(bc / FILLET_EDGE_FRACTION);
        out.extend(fillet_arc(b, u, v, cosine, tangent));
    }
    closed(out)
}

/// The arc tangent to the incoming direction `u` and the outgoing `v` at distance `tangent`
/// from the corner `b`.
fn fillet_arc(b: Point, u: (f64, f64), v: (f64, f64), cosine: f64, tangent: f64) -> Vec<Point> {
    let start = Point {
        x: b.x - u.0 * tangent,
        y: b.y - u.1 * tangent,
    };
    let end = Point {
        x: b.x + v.0 * tangent,
        y: b.y + v.1 * tangent,
    };
    let turn = libm::acos(cosine);
    let left = u.0 * v.1 - u.1 * v.0 > 0.0;
    // The centre lies on the corner's inner bisector, perpendicular to the chord.
    let (cx, cy) = (end.x - start.x, end.y - start.y);
    let chord = libm::hypot(cx, cy);
    let side = if left { 1.0 } else { -1.0 };
    let normal = (-cy / chord * side, cx / chord * side);
    let reach = tangent / libm::sin(turn / 2.0);
    let centre = Point {
        x: b.x + normal.0 * reach,
        y: b.y + normal.1 * reach,
    };
    let radius = libm::hypot(start.x - centre.x, start.y - centre.y);
    let first = libm::atan2(start.y - centre.y, start.x - centre.x);
    let sweep = turn * side;
    let mut arc: Vec<Point> = (0..FILLET_SAMPLES)
        .map(|step| {
            let angle = first + sweep * step as f64 / FILLET_SAMPLES as f64;
            Point {
                x: centre.x + radius * libm::cos(angle),
                y: centre.y + radius * libm::sin(angle),
            }
        })
        .collect();
    arc.push(end);
    arc
}

/// `ring` (closed, counter-clockwise) grown outward by `distance` with mitred joins: a
/// convex corner keeps its mitre up to `BRIM_MITRE_LIMIT`·distance and is cut square beyond;
/// a reflex corner goes through the original vertex and the overlap resolves away.
pub fn grow_mitred(ring: &[Point], distance: f64) -> Vec<Vec<Point>> {
    if distance <= 0.0 {
        return vec![ring.to_vec()];
    }
    let points = &ring[..ring.len() - 1];
    let count = points.len();
    let normals: Vec<(f64, f64)> = (0..count)
        .map(|i| outward_normal(points[i], points[(i + 1) % count]))
        .collect();
    let mut raw: IntRing = Vec::with_capacity(count * 3);
    for index in 0..count {
        let incoming = normals[(index + count - 1) % count];
        let outgoing = normals[index];
        mitre_corner(&mut raw, points[index], incoming, outgoing, distance);
    }
    loops_from_rings(&resolve(&[raw], FillRule::Positive))
}

fn outward_normal(a: Point, b: Point) -> (f64, f64) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = libm::hypot(dx, dy);
    if length == 0.0 {
        return (0.0, 0.0);
    }
    (dy / length, -dx / length)
}

fn mitre_corner(out: &mut IntRing, vertex: Point, n1: (f64, f64), n2: (f64, f64), distance: f64) {
    let at = |x: f64, y: f64| grid(vertex.x + x, vertex.y + y);
    let turn = n1.0 * n2.1 - n1.1 * n2.0;
    let one_plus_cos = 1.0 + n1.0 * n2.0 + n1.1 * n2.1;
    if turn < 0.0 {
        out.push(at(n1.0 * distance, n1.1 * distance));
        out.push(grid(vertex.x, vertex.y));
        out.push(at(n2.0 * distance, n2.1 * distance));
    } else if one_plus_cos >= 2.0 / (BRIM_MITRE_LIMIT * BRIM_MITRE_LIMIT) {
        let scale = distance / one_plus_cos;
        out.push(at((n1.0 + n2.0) * scale, (n1.1 + n2.1) * scale));
    } else {
        // Past the limit: the mitre's tip is cut square at limit·distance along the bisector.
        let (bx, by) = (n1.0 + n2.0, n1.1 + n2.1);
        let length = libm::hypot(bx, by).max(f64::MIN_POSITIVE);
        let (bx, by) = (bx / length, by / length);
        let reach = BRIM_MITRE_LIMIT * distance;
        for (normal, sign) in [(n1, 1.0), (n2, -1.0)] {
            let along = (-normal.1 * sign, normal.0 * sign);
            let base = (normal.0 * distance, normal.1 * distance);
            let base_reach = base.0 * bx + base.1 * by;
            let along_reach = along.0 * bx + along.1 * by;
            let step = if along_reach.abs() < 1e-12 {
                0.0
            } else {
                (reach - base_reach) / along_reach
            };
            out.push(at(base.0 + along.0 * step, base.1 + along.1 * step));
        }
    }
}

fn grid(x: f64, y: f64) -> IntPoint {
    IntPoint::new(
        libm::floor(x * NM_PER_MM + 0.5) as i32,
        libm::floor(y * NM_PER_MM + 0.5) as i32,
    )
}

fn closed(mut points: Vec<Point>) -> Vec<Point> {
    if let Some(first) = points.first().copied() {
        points.push(first);
    }
    points
}

fn counter_clockwise(mut ring: Vec<Point>) -> Vec<Point> {
    if polygon_area(&ring) < 0.0 {
        ring.reverse();
    }
    ring
}

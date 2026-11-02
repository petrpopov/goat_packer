//! Per-part 2D convex hulls and boxes: the slicer tests a part's convex hull against
//! exclusion zones and its bbox against the printable area (design §3.1, §4.2), part by part,
//! so the union of parts is not enough.

use crate::embedded_mesh::{ObjectMesh, Part};
use crate::geometry::{Point, Rect};
use crate::object_print::Loops;

/// One closed counter-clockwise hull ring per normal part, in part order.
pub fn part_hulls(mesh: &ObjectMesh) -> Loops {
    mesh.normal_parts()
        .filter_map(|part| {
            let mut ring = convex_hull(&xy_points(part))?;
            ring.push(ring[0]);
            Some(ring)
        })
        .collect()
}

/// The XY bounding box of every normal part, in part order.
pub fn part_boxes(mesh: &ObjectMesh) -> Vec<Rect> {
    mesh.normal_parts()
        .filter(|part| !part.vertices.is_empty())
        .map(|part| Rect::of_points(xy_points(part).iter()))
        .collect()
}

fn xy_points(part: &Part) -> Vec<Point> {
    part.vertices
        .iter()
        .map(|vertex| Point {
            x: vertex[0],
            y: vertex[1],
        })
        .collect()
}

/// Monotone-chain hull, counter-clockwise, collinear points dropped; `None` below three
/// non-collinear points.
pub fn convex_hull(points: &[Point]) -> Option<Vec<Point>> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    sorted.dedup();
    if sorted.len() < 3 {
        return None;
    }
    let mut hull = half_hull(sorted.iter());
    hull.extend(half_hull(sorted.iter().rev()));
    (hull.len() >= 3).then_some(hull)
}

/// One side of the monotone chain, without its last point (the next side starts there).
fn half_hull<'a>(points: impl Iterator<Item = &'a Point>) -> Vec<Point> {
    let mut chain: Vec<Point> = Vec::new();
    for &point in points {
        while chain.len() >= 2 && turn(chain[chain.len() - 2], chain[chain.len() - 1], point) <= 0.0
        {
            chain.pop();
        }
        chain.push(point);
    }
    chain.pop();
    chain
}

fn turn(a: Point, b: Point, c: Point) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

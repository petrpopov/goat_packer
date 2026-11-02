//! Exact distance and overlap tests between even-odd areas, for the vector check. The rings
//! are used as given: no boolean normalization (i_overlay snaps to an integer grid of up to
//! 2.4e-7 mm on a plate-sized zone, more than the check's slack) and no rasterizing, so a gap
//! is measured up to float rounding. A grid over each area's edges keeps a query to the edges
//! near it, and an area is built once per rotation class and compared at any translation.
//!
//! Every ring edge counts as boundary. That is exact for rings that do not overlap each other
//! (what boolean outputs and the slicer-derived kinds are); edges that cancel under the
//! even-odd rule (a ring repeated) can only make the check stricter.

use super::edge_grid::EdgeGrid;
use crate::geometry::{Point, Rect, polygon_from_loop};
use geo::orient::{Direction, Orient};
use geo::{Area, BooleanOps, MultiPolygon};
use std::ops::Range;

/// Two areas at least this far apart (mm) cannot overlap; closer ones may only be touching,
/// and only then is the exact (and slow) boolean overlap computed.
const TOUCH_MM: f64 = 1e-7;
/// Overlap area (mm²) of touching areas that is still float noise, not an overlap.
const AREA_EPS: f64 = 1e-9;

/// An even-odd area: its rings exactly as given, their bounds and a grid over their edges.
pub(super) struct Region {
    rings: Vec<Vec<Point>>,
    ring_bounds: Vec<Rect>,
    /// Edges of ring `i`: `edges[ring_edges[i].clone()]`.
    ring_edges: Vec<Range<usize>>,
    edges: Vec<[Point; 2]>,
    bounds: Rect,
    grid: EdgeGrid,
}

impl Region {
    /// `loops` as one even-odd area. A ring that is not closed is closed, and a ring of one
    /// point is a point: a degenerate ring is checked as the material it describes rather
    /// than dropped.
    pub(super) fn new(loops: &[Vec<Point>]) -> Region {
        let rings: Vec<Vec<Point>> = loops.iter().filter(|r| !r.is_empty()).cloned().collect();
        let mut edges = Vec::new();
        let mut ring_edges = Vec::with_capacity(rings.len());
        for ring in &rings {
            let start = edges.len();
            edges.extend(ring.windows(2).map(|pair| [pair[0], pair[1]]));
            let (first, last) = (ring[0], ring[ring.len() - 1]);
            if ring.len() == 1 || first != last {
                edges.push([last, first]);
            }
            ring_edges.push(start..edges.len());
        }
        let ring_bounds: Vec<Rect> = rings.iter().map(|r| Rect::of_points(r.iter())).collect();
        let bounds = Rect::of_points(rings.iter().flatten());
        let grid = EdgeGrid::new(&edges, bounds);
        Region {
            rings,
            ring_bounds,
            ring_edges,
            edges,
            bounds,
            grid,
        }
    }

    /// A rectangle as an area.
    pub(super) fn rect(r: Rect) -> Region {
        let corners = [
            (r.min_x, r.min_y),
            (r.max_x, r.min_y),
            (r.max_x, r.max_y),
            (r.min_x, r.max_y),
            (r.min_x, r.min_y),
        ];
        Region::new(&[corners.iter().map(|&(x, y)| Point { x, y }).collect()])
    }

    pub(super) fn is_empty(&self) -> bool {
        self.rings.is_empty()
    }

    /// Box around every ring point; inverted (`min > max`) when empty.
    pub(super) fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Even-odd containment of `p`. Only meaningful for a point off the edges, which is the
    /// only way `nested` asks.
    fn contains(&self, p: Point) -> bool {
        let mut inside = false;
        for (ring, bounds) in self.ring_edges.iter().zip(&self.ring_bounds) {
            if p.y < bounds.min_y || p.y > bounds.max_y || p.x > bounds.max_x {
                continue;
            }
            for &[a, b] in &self.edges[ring.clone()] {
                if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x
                {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

/// Whether `a` and `b` moved by `shift` overlap or come closer than `limit` (> 0, strict).
pub(super) fn closer_than(a: &Region, b: &Region, shift: Point, limit: f64) -> bool {
    debug_assert!(limit > 0.0);
    if a.is_empty() || b.is_empty() || !boxes_meet(a.bounds, moved_box(b.bounds, shift), limit) {
        return false;
    }
    // Walk the area with fewer edges and look the other one up in its grid.
    let near = if b.edges.len() <= a.edges.len() {
        edges_closer(b, a, shift, limit)
    } else {
        edges_closer(a, b, negated(shift), limit)
    };
    near || nested(a, b, shift)
}

/// Whether `a` and `b` moved by `shift` overlap with positive area; touching passes.
pub(super) fn overlaps(a: &Region, b: &Region, shift: Point) -> bool {
    if !closer_than(a, b, shift, TOUCH_MM) {
        return false;
    }
    let origin = Point { x: 0.0, y: 0.0 };
    let common = even_odd_area(a, origin).intersection(&even_odd_area(b, shift));
    common.unsigned_area() > AREA_EPS
}

/// Some edge of `walk` moved by `shift` comes closer than `limit` to an edge of `look`.
fn edges_closer(walk: &Region, look: &Region, shift: Point, limit: f64) -> bool {
    let limit_sq = limit * limit;
    for (ring, bounds) in walk.ring_edges.iter().zip(&walk.ring_bounds) {
        if !boxes_meet(moved_box(*bounds, shift), look.bounds, limit) {
            continue;
        }
        for &[p, q] in &walk.edges[ring.clone()] {
            let (p, q) = (moved(p, shift), moved(q, shift));
            let edge_box = Rect::of_points([p, q].iter());
            if !boxes_meet(edge_box, look.bounds, limit) {
                continue;
            }
            let hit = look
                .grid
                .any_near(&look.edges, edge_box.grown(limit), |&[r, s]| {
                    edge_distance_sq(p, q, r, s) < limit_sq
                });
            if hit {
                return true;
            }
        }
    }
    false
}

/// One area inside the other while no edges come close. Then no ring crosses an edge of the
/// other area, so each ring lies wholly inside or outside it and its first vertex decides
/// (a component of the overlap is bounded by whole rings of one side).
fn nested(a: &Region, b: &Region, shift: Point) -> bool {
    let b_in_a = b.rings.iter().zip(&b.ring_bounds).any(|(ring, bounds)| {
        boxes_meet(moved_box(*bounds, shift), a.bounds, 0.0) && a.contains(moved(ring[0], shift))
    });
    b_in_a
        || a.rings.iter().zip(&a.ring_bounds).any(|(ring, bounds)| {
            boxes_meet(*bounds, moved_box(b.bounds, shift), 0.0)
                && b.contains(moved(ring[0], negated(shift)))
        })
}

/// The area as geo polygons, rings XOR-ed (each oriented first: geo takes the fill rule from
/// the first ring's winding).
fn even_odd_area(region: &Region, shift: Point) -> MultiPolygon {
    region
        .rings
        .iter()
        .fold(MultiPolygon::new(vec![]), |area, ring| {
            let ring: Vec<Point> = ring.iter().map(|&p| moved(p, shift)).collect();
            let polygon = polygon_from_loop(&ring).orient(Direction::Default);
            area.xor(&MultiPolygon::new(vec![polygon]))
        })
}

/// Squared distance between segments `pq` and `rs`: 0 when they cross, otherwise the nearest
/// endpoint-to-segment distance (touching or collinear overlaps put an endpoint on the other).
fn edge_distance_sq(p: Point, q: Point, r: Point, s: Point) -> f64 {
    let (o1, o2) = (cross(p, q, r), cross(p, q, s));
    let (o3, o4) = (cross(r, s, p), cross(r, s, q));
    if o1 * o2 < 0.0 && o3 * o4 < 0.0 {
        return 0.0;
    }
    point_edge_sq(p, r, s)
        .min(point_edge_sq(q, r, s))
        .min(point_edge_sq(r, p, q))
        .min(point_edge_sq(s, p, q))
}

fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn point_edge_sq(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length_sq = dx * dx + dy * dy;
    let t = if length_sq > 0.0 {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / length_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (ex, ey) = (a.x + t * dx - p.x, a.y + t * dy - p.y);
    ex * ex + ey * ey
}

/// Two boxes come closer than `limit` (overlap or touch when `limit` is 0).
fn boxes_meet(a: Rect, b: Rect, limit: f64) -> bool {
    let near = |gap: f64| if limit > 0.0 { gap < limit } else { gap <= 0.0 };
    near(b.min_x - a.max_x)
        && near(a.min_x - b.max_x)
        && near(b.min_y - a.max_y)
        && near(a.min_y - b.max_y)
}

pub(super) fn moved(p: Point, by: Point) -> Point {
    Point {
        x: p.x + by.x,
        y: p.y + by.y,
    }
}

pub(super) fn moved_box(r: Rect, by: Point) -> Rect {
    r.translated(by.x, by.y)
}

pub(super) fn negated(p: Point) -> Point {
    Point { x: -p.x, y: -p.y }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, side: f64) -> Vec<Point> {
        [
            (0.0, 0.0),
            (side, 0.0),
            (side, side),
            (0.0, side),
            (0.0, 0.0),
        ]
        .iter()
        .map(|&(x, y)| Point {
            x: x0 + x,
            y: y0 + y,
        })
        .collect()
    }

    fn at(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn distance_is_exact_at_any_shift() {
        let a = Region::new(&[square(0.0, 0.0, 2.0)]);
        let b = Region::new(&[square(0.0, 0.0, 2.0)]);
        // 3 mm right: 1 mm apart.
        assert!(!closer_than(&a, &b, at(3.0, 0.0), 1.0));
        assert!(closer_than(&a, &b, at(3.0, 0.0), 1.0 + 1e-9));
        // Diagonal: corner to corner sqrt(2).
        assert!(!closer_than(&a, &b, at(3.0, 3.0), 2f64.sqrt() - 1e-12));
        assert!(closer_than(&a, &b, at(3.0, 3.0), 2f64.sqrt() + 1e-12));
    }

    #[test]
    fn nesting_without_close_edges_is_an_overlap() {
        let big = Region::new(&[square(0.0, 0.0, 10.0)]);
        let small = Region::new(&[square(0.0, 0.0, 1.0)]);
        assert!(closer_than(&big, &small, at(4.0, 4.0), 0.5));
        assert!(closer_than(&small, &big, at(-4.0, -4.0), 0.5));
        // Inside the hole of a ring: no material around it.
        let ring = Region::new(&[square(0.0, 0.0, 10.0), square(2.0, 2.0, 6.0)]);
        assert!(!closer_than(&ring, &small, at(4.5, 4.5), 0.5));
        assert!(closer_than(&ring, &small, at(0.5, 4.5), 0.5));
    }

    #[test]
    fn touching_is_not_an_overlap() {
        let a = Region::new(&[square(0.0, 0.0, 2.0)]);
        assert!(!overlaps(&a, &a, at(2.0, 0.0)));
        assert!(!overlaps(&a, &a, at(2.0, 2.0)));
        assert!(overlaps(&a, &a, at(1.9, 0.0)));
        // Sharing three edges, no proper crossing: still an overlap.
        let wide = Region::new(&[square(0.0, 0.0, 2.0), square(2.0, 0.0, 2.0)]);
        assert!(overlaps(&wide, &a, at(0.0, 0.0)));
    }

    #[test]
    fn a_degenerate_ring_still_has_edges() {
        let line = Region::new(&[vec![at(0.0, 0.0), at(5.0, 0.0), at(0.0, 0.0)]]);
        let a = Region::new(&[square(0.0, 0.0, 2.0)]);
        assert!(closer_than(&line, &a, at(1.0, 0.3), 0.5));
        assert!(!closer_than(&line, &a, at(1.0, 0.6), 0.5));
    }

    #[test]
    fn long_edges_are_found_through_the_grid() {
        // A long thin area: its edges span many cells.
        let strip = Region::new(&[vec![
            at(0.0, 0.0),
            at(200.0, 0.0),
            at(200.0, 1.0),
            at(0.0, 1.0),
            at(0.0, 0.0),
        ]]);
        let dot = Region::new(&[square(0.0, 0.0, 0.1)]);
        assert!(closer_than(&strip, &dot, at(150.0, 1.4), 0.5));
        assert!(!closer_than(&strip, &dot, at(150.0, 1.5), 0.5));
    }
}

//! Measurements on grid rings: area, bounding box, point containment, distances and the
//! nearest boundary point of spec 02 §5.5 (`nearest_inside`, `nearest_boundary`).

use super::{IntPoint, Ring};

/// An axis-aligned box in grid units, both corners inclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntBox {
    pub min: IntPoint,
    pub max: IntPoint,
}

impl IntBox {
    /// Width and height in units.
    pub fn size(&self) -> (i64, i64) {
        (
            i64::from(self.max.x) - i64::from(self.min.x),
            i64::from(self.max.y) - i64::from(self.min.y),
        )
    }

    /// The boxes share at least one point (touching counts, as the slicers' `overlap`).
    pub fn overlap(a: &IntBox, b: &IntBox) -> bool {
        a.min.x <= b.max.x && b.min.x <= a.max.x && a.min.y <= b.max.y && b.min.y <= a.max.y
    }

    pub fn contains(&self, point: IntPoint) -> bool {
        self.min.x <= point.x
            && point.x <= self.max.x
            && self.min.y <= point.y
            && point.y <= self.max.y
    }

    pub fn merge(a: Option<IntBox>, b: Option<IntBox>) -> Option<IntBox> {
        match (a, b) {
            (Some(a), Some(b)) => Some(IntBox {
                min: IntPoint::new(a.min.x.min(b.min.x), a.min.y.min(b.min.y)),
                max: IntPoint::new(a.max.x.max(b.max.x), a.max.y.max(b.max.y)),
            }),
            (only, None) | (None, only) => only,
        }
    }
}

/// Twice the signed area of a ring (positive counter-clockwise), exact.
pub fn ring_doubled_area(ring: &[IntPoint]) -> i128 {
    let count = ring.len();
    (0..count)
        .map(|index| {
            let a = ring[index];
            let b = ring[(index + 1) % count];
            i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y)
        })
        .sum()
}

/// Signed area of resolved rings in units² (holes count negative).
pub fn area(rings: &[Ring]) -> f64 {
    let doubled: i128 = rings.iter().map(|ring| ring_doubled_area(ring)).sum();
    doubled as f64 * 0.5
}

/// Bounding box of every vertex; `None` without vertices.
pub fn bbox(rings: &[Ring]) -> Option<IntBox> {
    let mut points = rings.iter().flatten();
    let first = *points.next()?;
    let mut found = IntBox {
        min: first,
        max: first,
    };
    for point in points {
        found.min = IntPoint::new(found.min.x.min(point.x), found.min.y.min(point.y));
        found.max = IntPoint::new(found.max.x.max(point.x), found.max.y.max(point.y));
    }
    Some(found)
}

/// Where a point lies relative to a region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Containment {
    Inside,
    Boundary,
    Outside,
}

/// Point against rings under the non-zero rule; any boundary hit is reported as such.
pub fn containment(rings: &[Ring], point: IntPoint) -> Containment {
    let mut winding = 0i64;
    for ring in rings {
        match ring_winding(ring, point) {
            None => return Containment::Boundary,
            Some(turns) => winding += turns,
        }
    }
    if winding != 0 {
        Containment::Inside
    } else {
        Containment::Outside
    }
}

/// Inside or on the boundary, non-zero rule.
pub fn contains(rings: &[Ring], point: IntPoint) -> bool {
    containment(rings, point) != Containment::Outside
}

/// Winding number of one ring around `point`; `None` when the point is on the ring.
fn ring_winding(ring: &[IntPoint], point: IntPoint) -> Option<i64> {
    let count = ring.len();
    let mut winding = 0;
    for index in 0..count {
        let a = ring[index];
        let b = ring[(index + 1) % count];
        let cross = cross(a, b, point);
        if cross == 0 && on_segment_box(a, b, point) {
            return None;
        }
        if a.y <= point.y {
            if b.y > point.y && cross > 0 {
                winding += 1;
            }
        } else if b.y <= point.y && cross < 0 {
            winding -= 1;
        }
    }
    Some(winding)
}

fn cross(a: IntPoint, b: IntPoint, p: IntPoint) -> i128 {
    (i128::from(b.x) - i128::from(a.x)) * (i128::from(p.y) - i128::from(a.y))
        - (i128::from(b.y) - i128::from(a.y)) * (i128::from(p.x) - i128::from(a.x))
}

fn on_segment_box(a: IntPoint, b: IntPoint, p: IntPoint) -> bool {
    a.x.min(b.x) <= p.x && p.x <= a.x.max(b.x) && a.y.min(b.y) <= p.y && p.y <= a.y.max(b.y)
}

/// Even-odd inside test where the boundary counts as inside (spec 02 §5.5).
fn inside_even_odd(rings: &[Ring], point: IntPoint) -> bool {
    let mut crossings = 0usize;
    for ring in rings {
        match ring_winding(ring, point) {
            None => return true,
            Some(turns) => crossings += turns.unsigned_abs() as usize,
        }
    }
    crossings % 2 == 1
}

/// Smallest distance from `point` to any edge of `rings`, in units; infinite without edges.
pub fn distance_to_boundary(rings: &[Ring], point: IntPoint) -> f64 {
    let mut best = f64::INFINITY;
    for ring in rings {
        let count = ring.len();
        for index in 0..count {
            let distance = segment_distance_squared(ring[index], ring[(index + 1) % count], point);
            best = best.min(distance);
        }
    }
    libm::sqrt(best)
}

fn segment_distance_squared(a: IntPoint, b: IntPoint, p: IntPoint) -> f64 {
    let (ax, ay) = (f64::from(a.x), f64::from(a.y));
    let (dx, dy) = (f64::from(b.x) - ax, f64::from(b.y) - ay);
    let (px, py) = (f64::from(p.x) - ax, f64::from(p.y) - ay);
    let length2 = dx * dx + dy * dy;
    let t = if length2 == 0.0 {
        0.0
    } else {
        ((px * dx + py * dy) / length2).clamp(0.0, 1.0)
    };
    let (ex, ey) = (px - t * dx, py - t * dy);
    ex * ex + ey * ey
}

/// `point` if it lies inside `rings` (even-odd, boundary inside) or `rings` is empty,
/// otherwise `nearest_boundary` (spec 02 §5.5).
pub fn nearest_inside(rings: &[Ring], point: IntPoint) -> IntPoint {
    if rings.is_empty() || inside_even_odd(rings, point) {
        return point;
    }
    nearest_boundary(rings, point).unwrap_or(point)
}

/// The nearest boundary point by the slicers' candidate rule (spec 02 §5.5): edge
/// projections strictly inside an edge, truncated toward zero per component; vertices whose
/// incoming edge projects at or past its end and whose outgoing edge at or before its start.
/// A later candidate wins only if strictly closer; each ring is visited from its closing edge.
pub fn nearest_boundary(rings: &[Ring], point: IntPoint) -> Option<IntPoint> {
    let mut best: Option<(i128, IntPoint)> = None;
    for ring in rings.iter().filter(|ring| ring.len() >= 2) {
        visit_ring(ring, point, &mut best);
    }
    best.map(|(_, found)| found)
}

/// Candidates of one ring in visiting order: closing edge first, then the edges in order;
/// a vertex candidate is examined with its outgoing edge.
fn visit_ring(ring: &[IntPoint], point: IntPoint, best: &mut Option<(i128, IntPoint)>) {
    let count = ring.len();
    let edges: Vec<(IntPoint, IntPoint)> = (0..count)
        .map(|index| (ring[(index + count - 1) % count], ring[index]))
        .filter(|(a, b)| a != b)
        .collect();
    let Some(&last) = edges.last() else {
        return;
    };
    let mut incoming_past_end = projection_parameter(last, point).1;
    for &edge in &edges {
        let (inside, past_end, before_start) = projection_parameter(edge, point);
        if incoming_past_end && before_start {
            offer(best, point, edge.0);
        }
        if let Some(projected) = inside {
            offer(best, point, projected);
        }
        incoming_past_end = past_end;
    }
}

/// For edge a → b: the truncated projection when 0 < t < L, whether t ≥ L (projects at or
/// past the end) and whether t ≤ 0 (at or before the start).
fn projection_parameter(
    edge: (IntPoint, IntPoint),
    point: IntPoint,
) -> (Option<IntPoint>, bool, bool) {
    let (a, b) = edge;
    let (dx, dy) = (
        i64::from(b.x) - i64::from(a.x),
        i64::from(b.y) - i64::from(a.y),
    );
    let (px, py) = (
        i64::from(point.x) - i64::from(a.x),
        i64::from(point.y) - i64::from(a.y),
    );
    let t = i128::from(dx) * i128::from(px) + i128::from(dy) * i128::from(py);
    let length2 = i128::from(dx) * i128::from(dx) + i128::from(dy) * i128::from(dy);
    let inside = (t > 0 && t < length2).then(|| {
        let ratio = t as f64 / length2 as f64;
        IntPoint::new(
            a.x + (dx as f64 * ratio) as i32,
            a.y + (dy as f64 * ratio) as i32,
        )
    });
    (inside, t >= length2, t <= 0)
}

fn offer(best: &mut Option<(i128, IntPoint)>, point: IntPoint, candidate: IntPoint) {
    let dx = i128::from(candidate.x) - i128::from(point.x);
    let dy = i128::from(candidate.y) - i128::from(point.y);
    let distance = dx * dx + dy * dy;
    if best.is_none_or(|(current, _)| distance < current) {
        *best = Some((distance, candidate));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(points: &[(i32, i32)]) -> Ring {
        points.iter().map(|&(x, y)| IntPoint::new(x, y)).collect()
    }

    fn nearest(rings: &[Ring], x: i32, y: i32) -> (i32, i32) {
        let found = nearest_inside(rings, IntPoint::new(x, y));
        (found.x, found.y)
    }

    /// The recorded vectors of spec 02 §8.4.
    #[test]
    fn nearest_boundary_test_vectors() {
        let square = vec![ring(&[(0, 0), (10, 0), (10, 10), (0, 10)])];
        assert_eq!(nearest(&square, 5, 5), (5, 5));
        assert_eq!(nearest(&square, 12, 5), (10, 5));
        assert_eq!(nearest(&square, 13, 13), (10, 10));
        assert_eq!(nearest(&square, -2, -3), (0, 0));
        assert_eq!(nearest(&square, 10, 12), (10, 10));
        let triangle = vec![ring(&[(0, 0), (9, 0), (0, 7)])];
        assert_eq!(nearest(&triangle, 6, 6), (5, 3));
        assert_eq!(nearest(&triangle, 3, 8), (2, 5));
        assert_eq!(nearest(&triangle, 10, -1), (9, 0));
        let two = vec![
            ring(&[(0, 0), (4, 0), (4, 4), (0, 4)]),
            ring(&[(10, 0), (14, 0), (14, 4), (10, 4)]),
        ];
        assert_eq!(nearest(&two, 7, 2), (4, 2));
        assert_eq!(nearest(&two, 7, 5), (4, 4));
        let repeated = vec![ring(&[(0, 0), (10, 0), (10, 0), (10, 10), (0, 10)])];
        assert_eq!(nearest(&repeated, 12, -1), (10, 0));
    }

    #[test]
    fn containment_reports_the_boundary() {
        let square = vec![ring(&[(0, 0), (10, 0), (10, 10), (0, 10)])];
        assert_eq!(
            containment(&square, IntPoint::new(5, 5)),
            Containment::Inside
        );
        assert_eq!(
            containment(&square, IntPoint::new(10, 5)),
            Containment::Boundary
        );
        assert_eq!(
            containment(&square, IntPoint::new(11, 5)),
            Containment::Outside
        );
        assert_eq!(area(&square), 100.0);
        assert_eq!(distance_to_boundary(&square, IntPoint::new(5, 4)), 4.0);
    }
}

//! The slicers' polygon simplification S(τ) (spec 02 §1.1): Douglas-Peucker on each closed
//! ring (run as the open polyline from the first vertex back to it, so the result depends on
//! the start vertex), then a non-zero self-union of that ring, holes turned back clockwise.
//!
//! Bambu asks for strictly simple output, Orca does not (02 §1.1); the difference is only
//! whether touching vertices split a ring, never the area. `i_overlay` has no strictly simple
//! mode, so `strict` is recorded for the swappable backend and the vertex-level difference
//! goes into δ (spec 02 §6 item 2).

use i_overlay::core::fill_rule::FillRule;

use super::boolean::resolve;
use super::query::ring_doubled_area;
use super::{IntPoint, Ring, Rings};

/// Douglas-Peucker on an open polyline: keeps the farthest vertex of a span (the first of
/// equals) while its distance to the span's segment exceeds `tolerance` (grid units).
pub fn douglas_peucker(points: &[IntPoint], tolerance: f64) -> Vec<IntPoint> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let limit = tolerance * tolerance;
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut spans = vec![(0usize, points.len() - 1)];
    while let Some((first, last)) = spans.pop() {
        let mut farthest = (first, 0.0f64);
        for index in first + 1..last {
            let distance = segment_distance_squared(points[first], points[last], points[index]);
            if distance > farthest.1 {
                farthest = (index, distance);
            }
        }
        if farthest.1 > limit {
            keep[farthest.0] = true;
            spans.push((farthest.0, last));
            spans.push((first, farthest.0));
        }
    }
    points
        .iter()
        .zip(&keep)
        .filter_map(|(point, &kept)| kept.then_some(*point))
        .collect()
}

/// S(τ) on every ring of `rings`.
pub fn simplify(rings: &[Ring], tolerance: f64, strict: bool) -> Rings {
    let _ = strict;
    let mut out = Rings::new();
    for ring in rings.iter().filter(|ring| !ring.is_empty()) {
        let mut path = ring.clone();
        path.push(ring[0]);
        let mut simplified = douglas_peucker(&path, tolerance);
        simplified.pop();
        let counter_clockwise = ring_doubled_area(&simplified) > 0;
        for mut resolved in resolve(&[simplified], FillRule::NonZero) {
            if !counter_clockwise {
                resolved.reverse();
            }
            out.push(resolved);
        }
    }
    out
}

fn segment_distance_squared(a: IntPoint, b: IntPoint, p: IntPoint) -> f64 {
    let (ax, ay) = (f64::from(a.x), f64::from(a.y));
    let (dx, dy) = (f64::from(b.x) - ax, f64::from(b.y) - ay);
    let (px, py) = (f64::from(p.x) - ax, f64::from(p.y) - ay);
    let length2 = dx * dx + dy * dy;
    if length2 == 0.0 {
        return px * px + py * py;
    }
    let t = (px * dx + py * dy) / length2;
    let (ex, ey) = if t <= 0.0 {
        (px, py)
    } else if t >= 1.0 {
        (px - dx, py - dy)
    } else {
        (px - t * dx, py - t * dy)
    };
    ex * ex + ey * ey
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::polygon_ops::area;

    #[test]
    fn small_wiggles_are_removed() {
        let ring: Ring = [(0, 0), (500, 3), (1000, 0), (1000, 1000), (0, 1000)]
            .iter()
            .map(|&(x, y)| IntPoint::new(x, y))
            .collect();
        let simplified = simplify(&[ring.clone()], 5.0, false);
        assert_eq!(simplified[0].len(), 4);
        assert_eq!(area(&simplified), 1_000_000.0);
        let kept = simplify(&[ring], 2.0, false);
        assert_eq!(kept[0].len(), 5);
    }

    #[test]
    fn holes_stay_clockwise() {
        let hole: Ring = [(0, 0), (0, 100), (100, 100), (100, 0)]
            .iter()
            .map(|&(x, y)| IntPoint::new(x, y))
            .collect();
        let simplified = simplify(&[hole], 1.0, true);
        assert!(ring_doubled_area(&simplified[0]) < 0);
    }
}

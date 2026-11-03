//! Nearest-boundary queries against a layer's radius-0 collision area, for the smoothing
//! pass: a uniform grid over the boundary segments, so a sphere only looks at the cells its
//! reach touches.

use std::collections::BTreeMap;

use crate::polygon_ops::Ring;

/// Cell edge, grid units.
pub struct SegmentGrid {
    cell: f64,
    cells: BTreeMap<(i64, i64), Vec<usize>>,
    segments: Vec<([f64; 2], [f64; 2])>,
}

impl SegmentGrid {
    pub fn new(rings: &[Ring], cell: f64) -> SegmentGrid {
        let mut grid = SegmentGrid {
            cell,
            cells: BTreeMap::new(),
            segments: Vec::new(),
        };
        for ring in rings {
            for index in 0..ring.len() {
                let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
                if a == b {
                    continue;
                }
                let segment = (
                    [f64::from(a.x), f64::from(a.y)],
                    [f64::from(b.x), f64::from(b.y)],
                );
                grid.insert(segment);
            }
        }
        grid
    }

    fn insert(&mut self, segment: ([f64; 2], [f64; 2])) {
        let id = self.segments.len();
        self.segments.push(segment);
        let (low, high) = (
            self.index(
                segment.0[0].min(segment.1[0]),
                segment.0[1].min(segment.1[1]),
            ),
            self.index(
                segment.0[0].max(segment.1[0]),
                segment.0[1].max(segment.1[1]),
            ),
        );
        for x in low.0..=high.0 {
            for y in low.1..=high.1 {
                self.cells.entry((x, y)).or_default().push(id);
            }
        }
    }

    fn index(&self, x: f64, y: f64) -> (i64, i64) {
        (
            libm::floor(x / self.cell) as i64,
            libm::floor(y / self.cell) as i64,
        )
    }

    /// The boundary point nearest to `at` within `reach`, and its distance.
    pub fn nearest(&self, at: [f64; 2], reach: f64) -> Option<([f64; 2], f64)> {
        let low = self.index(at[0] - reach, at[1] - reach);
        let high = self.index(at[0] + reach, at[1] + reach);
        let mut best: Option<([f64; 2], f64)> = None;
        let mut seen: Vec<usize> = Vec::new();
        for x in low.0..=high.0 {
            for y in low.1..=high.1 {
                let Some(ids) = self.cells.get(&(x, y)) else {
                    continue;
                };
                for &id in ids {
                    if seen.contains(&id) {
                        continue;
                    }
                    seen.push(id);
                    let (point, distance) = closest(self.segments[id], at);
                    if distance <= reach && best.is_none_or(|(_, known)| distance < known) {
                        best = Some((point, distance));
                    }
                }
            }
        }
        best
    }
}

fn closest(segment: ([f64; 2], [f64; 2]), at: [f64; 2]) -> ([f64; 2], f64) {
    let (a, b) = segment;
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length2 = dx * dx + dy * dy;
    let t = (((at[0] - a[0]) * dx + (at[1] - a[1]) * dy) / length2).clamp(0.0, 1.0);
    let point = [a[0] + t * dx, a[1] + t * dy];
    let distance = libm::hypot(at[0] - point[0], at[1] - point[1]);
    (point, distance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::polygon_ops::IntPoint;

    #[test]
    fn the_nearest_point_of_a_square_boundary() {
        let ring: Ring = vec![
            IntPoint::new(0, 0),
            IntPoint::new(1000, 0),
            IntPoint::new(1000, 1000),
            IntPoint::new(0, 1000),
        ];
        let grid = SegmentGrid::new(&[ring], 100.0);
        let (point, distance) = grid.nearest([1100.0, 500.0], 200.0).unwrap();
        assert_eq!((point, distance), ([1000.0, 500.0], 100.0));
        assert!(grid.nearest([2000.0, 500.0], 200.0).is_none());
    }
}

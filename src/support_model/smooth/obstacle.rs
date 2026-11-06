//! Nearest-boundary queries against a layer's radius-0 collision area, for the smoothing
//! pass: a uniform grid over the boundary segments, so a sphere only looks at the cells its
//! reach touches.

use crate::polygon_ops::Ring;

/// Cell edge, grid units.
pub struct SegmentGrid {
    cell: f64,
    /// Lowest cell index and the cell count per axis; cells outside hold no segment.
    origin: (i64, i64),
    size: (i64, i64),
    /// Segment ids per cell, row-major (y major), ascending within a cell.
    cells: Vec<Vec<u32>>,
    segments: Vec<([f64; 2], [f64; 2])>,
}

impl SegmentGrid {
    pub fn new(rings: &[Ring], cell: f64) -> SegmentGrid {
        let mut segments = Vec::new();
        for ring in rings {
            for index in 0..ring.len() {
                let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
                if a == b {
                    continue;
                }
                segments.push((
                    [f64::from(a.x), f64::from(a.y)],
                    [f64::from(b.x), f64::from(b.y)],
                ));
            }
        }
        let index = |x: f64, y: f64| ((x / cell).floor() as i64, (y / cell).floor() as i64);
        let (mut low, mut high) = ((i64::MAX, i64::MAX), (i64::MIN, i64::MIN));
        for (a, b) in &segments {
            let (l, h) = (index(a[0].min(b[0]), a[1].min(b[1])), index(a[0].max(b[0]), a[1].max(b[1])));
            low = (low.0.min(l.0), low.1.min(l.1));
            high = (high.0.max(h.0), high.1.max(h.1));
        }
        let size = if segments.is_empty() {
            (0, 0)
        } else {
            (high.0 - low.0 + 1, high.1 - low.1 + 1)
        };
        let mut grid = SegmentGrid {
            cell,
            origin: low,
            size,
            cells: vec![Vec::new(); (size.0 * size.1) as usize],
            segments: Vec::new(),
        };
        for segment in segments {
            grid.insert(segment);
        }
        grid
    }

    fn insert(&mut self, segment: ([f64; 2], [f64; 2])) {
        let id = self.segments.len() as u32;
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
                let slot = self.slot(x, y).expect("a segment's cells are inside the grid");
                self.cells[slot].push(id);
            }
        }
    }

    /// `f64::floor` is exact (IEEE 754), so it is the same on every platform as `libm`'s,
    /// which is a software routine and was a sixth of the smoothing pass.
    fn index(&self, x: f64, y: f64) -> (i64, i64) {
        ((x / self.cell).floor() as i64, (y / self.cell).floor() as i64)
    }

    fn slot(&self, x: i64, y: i64) -> Option<usize> {
        let (dx, dy) = (x - self.origin.0, y - self.origin.1);
        (dx >= 0 && dy >= 0 && dx < self.size.0 && dy < self.size.1)
            .then(|| (dy * self.size.0 + dx) as usize)
    }

    /// The boundary point nearest to `at` within `reach`, and its distance. The cells are
    /// visited x-major and a segment in several cells is tested again: it gives the same
    /// distance, which never beats itself (`<`), so the answer is the first nearest segment in
    /// that visiting order without the list of seen ids (whose linear search was most of
    /// the smoothing pass).
    pub fn nearest(&self, at: [f64; 2], reach: f64) -> Option<([f64; 2], f64)> {
        let low = self.index(at[0] - reach, at[1] - reach);
        let high = self.index(at[0] + reach, at[1] + reach);
        let (low, high) = (
            (low.0.max(self.origin.0), low.1.max(self.origin.1)),
            (
                high.0.min(self.origin.0 + self.size.0 - 1),
                high.1.min(self.origin.1 + self.size.1 - 1),
            ),
        );
        let mut best: Option<([f64; 2], f64)> = None;
        for x in low.0..=high.0 {
            for y in low.1..=high.1 {
                let Some(slot) = self.slot(x, y) else {
                    continue;
                };
                for &id in &self.cells[slot] {
                    let point = closest(self.segments[id as usize], at);
                    let (dx, dy) = (at[0] - point[0], at[1] - point[1]);
                    // A cheap squared bound with slack first: only a candidate pays for the
                    // exact `hypot`, compared exactly as before.
                    let bound = best.map_or(reach, |(_, known)| known.min(reach));
                    if dx * dx + dy * dy > bound * bound * (1.0 + 1e-9) + 1.0 {
                        continue;
                    }
                    let distance = libm::hypot(dx, dy);
                    if distance <= reach && best.is_none_or(|(_, known)| distance < known) {
                        best = Some((point, distance));
                    }
                }
            }
        }
        best
    }
}

/// The point of `segment` nearest to `at`.
fn closest(segment: ([f64; 2], [f64; 2]), at: [f64; 2]) -> [f64; 2] {
    let (a, b) = segment;
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length2 = dx * dx + dy * dy;
    let t = (((at[0] - a[0]) * dx + (at[1] - a[1]) * dy) / length2).clamp(0.0, 1.0);
    [a[0] + t * dx, a[1] + t * dy]
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

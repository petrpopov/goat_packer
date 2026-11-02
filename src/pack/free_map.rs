//! Where the next copy may go (design 2026-11-02, §9.1): `FreeMap` is the feasible-origin
//! bitmap a greedy member scans and clears, `FreeCounts` the same map with a counter per
//! origin, so a placement (or the tower) can be taken out again by the improvement loop.
//!
//! Both start from a base map (`kind_masks::feasible_origins`: every origin the plate zones
//! allow one rotation class) and keep its bounds, so a removable layer (the tower) never
//! changes the frame a member scans in.

use crate::kind_masks::{FeasibleOrigins, OriginSet};
use crate::pixel_mask::set_bit_range;
use crate::span_raster::SpanRaster;
use std::ops::Range;

/// Free origins of one rotation class as bit rows over the base map's bounding box.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FreeMap {
    min_x: i32,
    min_y: i32,
    width: usize,
    rows: Vec<Vec<u64>>,
}

impl FreeMap {
    /// Every origin of `origins`, over their bounding box (empty when there is none).
    pub fn new(origins: &FeasibleOrigins) -> FreeMap {
        let Some((min_x, min_y, max_x, max_y)) = origins.bounds() else {
            return FreeMap::default();
        };
        let width = (max_x - min_x + 1) as usize;
        let mut rows = vec![vec![0; width.div_ceil(64)]; (max_y - min_y + 1) as usize];
        for (index, row) in rows.iter_mut().enumerate() {
            for &(start, end) in origins.allowed.row(min_y + index as i32) {
                set_bit_range(row, (start - min_x) as usize, (end - min_x) as usize);
            }
        }
        FreeMap {
            min_x,
            min_y,
            width,
            rows,
        }
    }

    /// Columns of the bounding box, the first one at its left edge.
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.cell(x, y)
            .is_some_and(|(row, column)| self.rows[row][column / 64] & (1u64 << (column % 64)) != 0)
    }

    /// Number of free origins.
    pub fn count(&self) -> usize {
        self.rows
            .iter()
            .flatten()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    /// Takes the origins of `set` out (a movable zone placed, e.g. the tower).
    pub fn remove(&mut self, set: &OriginSet) {
        self.clear_spans(&set.spans, 0, 0);
    }

    /// A copy placed at origin `(x, y)`: every origin `(x, y) + nfp` is taken.
    pub(crate) fn exclude(&mut self, nfp: &SpanRaster, x: i32, y: i32) {
        self.clear_spans(nfp, x, y);
    }

    fn clear_spans(&mut self, spans: &SpanRaster, x: i32, y: i32) {
        for (i, row) in spans.rows.iter().enumerate() {
            for &(a, b) in row {
                self.clear(y + spans.min_y + i as i32, x + a, x + b);
            }
        }
    }

    /// Clears plate columns `[start, end)` of row `y`; whatever lies outside the box is ignored.
    fn clear(&mut self, y: i32, start: i32, end: i32) {
        let index = y - self.min_y;
        if index < 0 || index as usize >= self.rows.len() {
            return;
        }
        let Some((start, end)) = self.columns(start, end) else {
            return;
        };
        let row = &mut self.rows[index as usize];
        let first = start / 64;
        let last = (end - 1) / 64;
        let low = !0u64 << (start % 64);
        let high = !0u64 >> (63 - (end - 1) % 64);
        if first == last {
            row[first] &= !(low & high);
        } else {
            row[first] &= !low;
            row[first + 1..last].fill(0);
            row[last] &= !high;
        }
    }

    /// The first free origin in sweep order: rows from the bottom (from the top with
    /// `reverse_y`), and in each row the first free column of `columns` from the left (the
    /// last one with `reverse_x`). Columns count from the box's left edge.
    pub fn first_in(
        &self,
        columns: Range<usize>,
        reverse_x: bool,
        reverse_y: bool,
    ) -> Option<(i32, i32)> {
        let (lo, hi) = (columns.start, columns.end.min(self.width));
        for k in 0..self.rows.len() {
            let y = if reverse_y {
                self.rows.len() - 1 - k
            } else {
                k
            };
            let row = &self.rows[y];
            for j in 0..row.len() {
                let index = if reverse_x { row.len() - 1 - j } else { j };
                let word_lo = lo.saturating_sub(index * 64).min(64);
                let word_hi = hi.saturating_sub(index * 64).min(64);
                let word = if word_lo >= word_hi {
                    0
                } else {
                    row[index] & (!0u64 << word_lo) & (!0u64 >> (64 - word_hi))
                };
                if word != 0 {
                    let bit = if reverse_x {
                        63 - word.leading_zeros()
                    } else {
                        word.trailing_zeros()
                    };
                    return Some((
                        self.min_x + (index * 64 + bit as usize) as i32,
                        self.min_y + y as i32,
                    ));
                }
            }
        }
        None
    }

    /// Row and column of origin `(x, y)` inside the box.
    fn cell(&self, x: i32, y: i32) -> Option<(usize, usize)> {
        let (row, column) = (y - self.min_y, x - self.min_x);
        let inside = row >= 0
            && (row as usize) < self.rows.len()
            && column >= 0
            && (column as usize) < self.width;
        inside.then_some((row as usize, column as usize))
    }

    /// Plate columns `[start, end)` clamped to the box, as box columns.
    fn columns(&self, start: i32, end: i32) -> Option<(usize, usize)> {
        let start = (start - self.min_x).clamp(0, self.width as i32) as usize;
        let end = (end - self.min_x).clamp(0, self.width as i32) as usize;
        (start < end).then_some((start, end))
    }
}

/// A base map with a counter per origin: how many placed copies (and removable zones) forbid
/// it. Unlike `FreeMap`, a placement can be taken out again in O(|no-fit|), which the
/// insertion and ruin-and-recreate moves of the improvement loop need.
#[derive(Clone, Debug)]
pub struct FreeCounts {
    base: FreeMap,
    counts: Vec<u16>,
}

impl FreeCounts {
    pub fn new(base: FreeMap) -> FreeCounts {
        let counts = vec![0; base.width * base.rows.len()];
        FreeCounts { base, counts }
    }

    /// Free in the base map and forbidden by nothing placed.
    pub fn is_free(&self, x: i32, y: i32) -> bool {
        self.base.contains(x, y)
            && self
                .base
                .cell(x, y)
                .is_some_and(|(row, column)| self.counts[row * self.base.width + column] == 0)
    }

    /// A copy placed at `(x, y)`: every origin `(x, y) + nfp` gets one more blocker.
    // The improvement loop (work package P6) is the caller; the tests below exercise it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn add(&mut self, nfp: &SpanRaster, x: i32, y: i32) {
        self.update(nfp, x, y, true);
    }

    /// Takes back an `add` with the same arguments.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn remove(&mut self, nfp: &SpanRaster, x: i32, y: i32) {
        self.update(nfp, x, y, false);
    }

    /// A removable zone (the tower at one anchor) blocks its origins.
    pub fn add_set(&mut self, set: &OriginSet) {
        self.update(&set.spans, 0, 0, true);
    }

    /// Takes back an `add_set` of the same set.
    pub fn remove_set(&mut self, set: &OriginSet) {
        self.update(&set.spans, 0, 0, false);
    }

    /// The origins free right now, as a bitmap for first-fit scans.
    pub fn free_map(&self) -> FreeMap {
        let mut free = self.base.clone();
        let width = self.base.width;
        for (row, bits) in free.rows.iter_mut().enumerate() {
            let counts = &self.counts[row * width..(row + 1) * width];
            for (column, _) in counts.iter().enumerate().filter(|(_, c)| **c > 0) {
                bits[column / 64] &= !(1u64 << (column % 64));
            }
        }
        free
    }

    fn update(&mut self, spans: &SpanRaster, x: i32, y: i32, add: bool) {
        let width = self.base.width;
        for (i, row_spans) in spans.rows.iter().enumerate() {
            let row = y + spans.min_y + i as i32 - self.base.min_y;
            if row < 0 || row as usize >= self.base.rows.len() {
                continue;
            }
            let offset = row as usize * width;
            for &(a, b) in row_spans {
                let Some((start, end)) = self.base.columns(x + a, x + b) else {
                    continue;
                };
                for count in &mut self.counts[offset + start..offset + end] {
                    // A wrapped counter would free an origin that is still blocked; both
                    // cases are bugs (65535 blockers, or a removal that was never added).
                    *count = if add {
                        count.checked_add(1).expect("free-map counter overflow")
                    } else {
                        count
                            .checked_sub(1)
                            .expect("removed a placement that was never added")
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 150 x 20 box with a notch: crosses word boundaries and has a hole.
    fn base() -> FeasibleOrigins {
        let rect = SpanRaster::rect(-5, -3, 144, 16);
        let notch = SpanRaster::rect(60, 2, 70, 9);
        FeasibleOrigins {
            allowed: rect.difference(&notch),
        }
    }

    fn nfp() -> SpanRaster {
        SpanRaster {
            min_y: -2,
            rows: vec![vec![(-3, 4)], vec![(-70, -60), (-1, 2)], vec![(0, 66)]],
        }
    }

    #[test]
    fn first_in_scans_rows_and_columns_in_sweep_order() {
        let map = FreeMap::new(&base());
        assert_eq!(map.first_in(0..map.width(), false, false), Some((-5, -3)));
        assert_eq!(map.first_in(0..map.width(), true, true), Some((144, 16)));
        assert_eq!(map.first_in(70..80, false, false), Some((65, -3)));
        assert_eq!(map.first_in(0..60, true, false), Some((54, -3)));
    }

    #[test]
    fn counts_match_a_recount_after_adds_and_removes() {
        let base = base();
        let nfp = nfp();
        let mut counts = FreeCounts::new(FreeMap::new(&base));
        let mut placed = vec![(0, 0), (40, 5), (100, 12), (40, 5), (-20, -5), (140, 18)];
        for &(x, y) in &placed {
            counts.add(&nfp, x, y);
        }
        for index in [3, 0, 2] {
            let (x, y) = placed.remove(index);
            counts.remove(&nfp, x, y);
        }
        let mut expected = FreeMap::new(&base);
        for &(x, y) in &placed {
            expected.exclude(&nfp, x, y);
        }
        assert_eq!(counts.free_map(), expected);
        for y in -6..20 {
            for x in -8..150 {
                assert_eq!(counts.is_free(x, y), expected.contains(x, y), "{x},{y}");
            }
        }
    }

    #[test]
    fn a_removable_set_comes_and_goes() {
        let base = base();
        let mut counts = FreeCounts::new(FreeMap::new(&base));
        let set = OriginSet {
            spans: SpanRaster::rect(10, 0, 20, 4),
        };
        counts.add_set(&set);
        assert!(!counts.is_free(15, 2));
        counts.add(&nfp(), 15, 2);
        counts.remove_set(&set);
        assert!(!counts.is_free(15, 2), "the placement still blocks it");
        counts.remove(&nfp(), 15, 2);
        assert_eq!(counts.free_map(), FreeMap::new(&base));
        let mut map = FreeMap::new(&base);
        map.remove(&set);
        assert_eq!(map.count(), FreeMap::new(&base).count() - 11 * 5);
    }
}

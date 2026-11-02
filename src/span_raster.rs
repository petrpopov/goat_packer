//! Rasters as sorted pixel spans per row: scanline fill, conservative coverage and the set
//! operations the origin maps need. Clearance is not a raster operation any more: kinds are
//! buffered as vectors first (`kind_masks::buffer`), so the raster only adds its own
//! boundary cells.

use crate::geometry::Point;

/// A raster as sorted, disjoint, half-open column spans per row (`rows[0]` is row `min_y`).
/// Every mask is built through spans: scanline fill and span dilation cost O(rows × edges)
/// instead of the O(pixels × edges) point-in-polygon sampling they replaced — an identical
/// pixel set (the scanline uses the exact crossing formula of `point_in_loops`), without
/// a hash allocation and point-in-polygon evaluation per pixel.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SpanRaster {
    pub(crate) min_y: i32,
    pub(crate) rows: Vec<Vec<(i32, i32)>>,
}

impl SpanRaster {
    /// Pixels whose centres `((ix + 0.5) * res, (iy + 0.5) * res)` fall inside `loops` under
    /// the even-odd rule, restricted to `ix ∈ [min_ix, max_ix]`, `iy ∈ [min_iy, max_iy]` —
    /// exactly the set the per-pixel `point_in_loops` sampling produced.
    pub(crate) fn scanline(
        loops: &[Vec<Point>],
        resolution: f64,
        (min_ix, max_ix, min_iy, max_iy): (i32, i32, i32, i32),
    ) -> Self {
        let mut rows = Vec::with_capacity((max_iy - min_iy + 1).max(0) as usize);
        let mut crossings: Vec<f64> = Vec::new();
        for iy in min_iy..=max_iy {
            let y = (f64::from(iy) + 0.5) * resolution;
            crossings.clear();
            for loop_ in loops {
                for edge in loop_.windows(2) {
                    let (a, b) = (edge[0], edge[1]);
                    if (a.y > y) != (b.y > y) {
                        crossings.push((b.x - a.x) * (y - a.y) / (b.y - a.y) + a.x);
                    }
                }
            }
            crossings.sort_unstable_by(f64::total_cmp);
            // A centre x is inside iff an odd number of crossings c satisfy x < c, i.e. an
            // odd number satisfy c <= x: x ∈ [c[2k], c[2k+1]).
            let mut row: Vec<(i32, i32)> = Vec::new();
            for pair in crossings.chunks_exact(2) {
                let first = first_centre_at_or_after(pair[0], resolution).max(min_ix);
                let end = first_centre_at_or_after(pair[1], resolution).min(max_ix + 1);
                if first < end {
                    push_span(&mut row, first, end);
                }
            }
            rows.push(row);
        }
        Self {
            min_y: min_iy,
            rows,
        }
    }

    /// Conservative coverage: centre fill plus every cell crossed by a boundary.
    /// Centre sampling alone loses thin tips/islands and can accept real intersections.
    /// Grid-aligned edges need no extra cells: touching alone is not positive-area overlap.
    pub(crate) fn cover(
        loops: &[Vec<Point>],
        resolution: f64,
        range: (i32, i32, i32, i32),
    ) -> Self {
        let mut raster = Self::scanline(loops, resolution, range);
        // Suppress floating-point noise at grid lines (not geometric features).
        let snap = |x: f64| {
            let q = x / resolution;
            if (q - q.round()).abs() < 1e-10 {
                q.round()
            } else {
                q
            }
        };
        for ring in loops {
            for edge in ring.windows(2) {
                let (ax, ay) = (snap(edge[0].x), snap(edge[0].y));
                let (bx, by) = (snap(edge[1].x), snap(edge[1].y));
                let mut add = |y: i32, left: f64, right: f64| {
                    let start = (left.floor() as i32).max(range.0);
                    let end = (right.ceil() as i32).min(range.1 + 1);
                    if y >= range.2 && y <= range.3 && start < end {
                        raster.rows[(y - range.2) as usize].push((start, end));
                    }
                };
                if ay == by {
                    if ay != ay.floor() {
                        add(ay.floor() as i32, ax.min(bx), ax.max(bx));
                    }
                    continue;
                }
                let lo = ay.min(by);
                let hi = ay.max(by);
                for y in (lo.floor() as i32).max(range.2)..(hi.ceil() as i32).min(range.3 + 1) {
                    let x_at = |v: f64| ax + (bx - ax) * (v - ay) / (by - ay);
                    let x0 = x_at(lo.max(f64::from(y)));
                    let x1 = x_at(hi.min(f64::from(y + 1)));
                    add(y, x0.min(x1), x0.max(x1));
                }
            }
        }
        for row in &mut raster.rows {
            merge_spans(row);
        }
        raster
    }

    /// No pixel set.
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.iter().all(Vec::is_empty)
    }

    pub(crate) fn row(&self, y: i32) -> &[(i32, i32)] {
        let index = y - self.min_y;
        if index < 0 || index as usize >= self.rows.len() {
            &[]
        } else {
            &self.rows[index as usize]
        }
    }

    pub(crate) fn max_y(&self) -> i32 {
        self.min_y + self.rows.len() as i32
    }

    pub(crate) fn union(&self, other: &SpanRaster) -> SpanRaster {
        if self.rows.is_empty() {
            return other.clone();
        }
        if other.rows.is_empty() {
            return self.clone();
        }
        let min_y = self.min_y.min(other.min_y);
        let max_y = self.max_y().max(other.max_y());
        let rows = (min_y..max_y)
            .map(|y| {
                let mut spans: Vec<(i32, i32)> =
                    self.row(y).iter().chain(other.row(y)).copied().collect();
                merge_spans(&mut spans);
                spans
            })
            .collect();
        SpanRaster { min_y, rows }
    }

    /// Every pixel of the inclusive rectangle `[min_x, max_x] x [min_y, max_y]`; empty when a
    /// min exceeds its max.
    pub(crate) fn rect(min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> SpanRaster {
        if min_x > max_x || min_y > max_y {
            return SpanRaster::default();
        }
        SpanRaster {
            min_y,
            rows: vec![vec![(min_x, max_x + 1)]; (max_y - min_y + 1) as usize],
        }
    }

    /// The pixels of `self` that `other` does not have.
    pub(crate) fn difference(&self, other: &SpanRaster) -> SpanRaster {
        let rows = (self.min_y..self.max_y())
            .map(|y| subtract_spans(self.row(y), other.row(y)))
            .collect();
        SpanRaster {
            min_y: self.min_y,
            rows,
        }
    }

    pub(crate) fn contains(&self, x: i32, y: i32) -> bool {
        let row = self.row(y);
        let after = row.partition_point(|&(start, _)| start <= x);
        after > 0 && x < row[after - 1].1
    }

    pub(crate) fn pixel_count(&self) -> usize {
        self.rows
            .iter()
            .flatten()
            .map(|&(start, end)| (end - start) as usize)
            .sum()
    }
}

/// `a` minus `b`, both sorted and disjoint.
fn subtract_spans(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::with_capacity(a.len());
    let mut cut = b.iter().peekable();
    for &(mut start, end) in a {
        while let Some(&&(cut_start, cut_end)) = cut.peek() {
            if cut_end <= start {
                cut.next();
                continue;
            }
            if cut_start >= end {
                break;
            }
            if cut_start > start {
                out.push((start, cut_start));
            }
            start = start.max(cut_end);
            if cut_end >= end {
                break;
            }
            cut.next();
        }
        if start < end {
            out.push((start, end));
        }
    }
    out
}

/// Smallest `ix` whose pixel centre `(ix + 0.5) * resolution` is `>= c`, evaluated with the
/// same float expression the per-pixel test uses so boundary pixels match bit for bit.
fn first_centre_at_or_after(c: f64, resolution: f64) -> i32 {
    let mut ix = (c / resolution - 0.5).ceil() as i32;
    while (f64::from(ix - 1) + 0.5) * resolution >= c {
        ix -= 1;
    }
    while (f64::from(ix) + 0.5) * resolution < c {
        ix += 1;
    }
    ix
}

/// Appends `[start, end)` to a row built left to right, merging with the previous span.
fn push_span(row: &mut Vec<(i32, i32)>, start: i32, end: i32) {
    if let Some(last) = row.last_mut()
        && start <= last.1
    {
        last.1 = last.1.max(end);
        return;
    }
    row.push((start, end));
}

/// Sorts spans and merges overlapping or adjacent ones in place.
pub(crate) fn merge_spans(spans: &mut Vec<(i32, i32)>) {
    if spans.len() <= 1 {
        return;
    }
    spans.sort_unstable();
    let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for &(start, end) in spans.iter() {
        push_span(&mut merged, start, end);
    }
    *spans = merged;
}

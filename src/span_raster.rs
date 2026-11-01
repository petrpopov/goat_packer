//! Rasters as sorted pixel spans per row: scanline fill, dilation and erosion of loops.

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

    /// Dilation by the discrete disc `dx² + dy² <= radius²` — the structuring element the
    /// per-pixel dilation used: row `y` of the result is the union over `dy` of row `y + dy`
    /// widened by `isqrt(radius² - dy²)` on both sides.
    pub(crate) fn dilate(&self, radius: i32) -> SpanRaster {
        if radius <= 0 || self.rows.is_empty() {
            return self.clone();
        }
        let widths: Vec<i32> = (-radius..=radius)
            .map(|dy| disc_half_width(radius, dy))
            .collect();
        let min_y = self.min_y - radius;
        let max_y = self.max_y() + radius;
        let rows = (min_y..max_y)
            .map(|y| {
                let mut spans: Vec<(i32, i32)> = Vec::new();
                for (offset, &w) in widths.iter().enumerate() {
                    let dy = offset as i32 - radius;
                    spans.extend(self.row(y + dy).iter().map(|&(a, b)| (a - w, b + w)));
                }
                merge_spans(&mut spans);
                spans
            })
            .collect();
        SpanRaster { min_y, rows }
    }

    /// Erosion by the same discrete disc `dilate` uses: a pixel survives iff the whole disc
    /// around it is set. Row `y` is the intersection over `dy` of row `y + dy` narrowed by
    /// `isqrt(radius² - dy²)` on both sides (spans are merged, so a run of set pixels always
    /// lies inside a single span).
    pub(crate) fn erode(&self, radius: i32) -> SpanRaster {
        if radius <= 0 || self.rows.is_empty() {
            return self.clone();
        }
        let min_y = self.min_y + radius;
        let max_y = self.max_y() - radius;
        let rows = (min_y..max_y.max(min_y))
            .map(|y| {
                let mut acc: Option<Vec<(i32, i32)>> = None;
                for dy in -radius..=radius {
                    let w = disc_half_width(radius, dy);
                    let narrowed: Vec<(i32, i32)> = self
                        .row(y + dy)
                        .iter()
                        .map(|&(a, b)| (a + w, b - w))
                        .filter(|&(a, b)| a < b)
                        .collect();
                    acc = Some(match acc {
                        None => narrowed,
                        Some(previous) => intersect_spans(&previous, &narrowed),
                    });
                    if acc.as_ref().is_some_and(Vec::is_empty) {
                        break;
                    }
                }
                acc.unwrap_or_default()
            })
            .collect();
        SpanRaster { min_y, rows }
    }
}

/// `isqrt(radius² - dy²)`: half-width of row `dy` of the discrete disc `dx² + dy² <= radius²`.
fn disc_half_width(radius: i32, dy: i32) -> i32 {
    let mut w = (f64::from(radius * radius - dy * dy)).sqrt() as i32;
    while w * w + dy * dy > radius * radius {
        w -= 1;
    }
    while (w + 1) * (w + 1) + dy * dy <= radius * radius {
        w += 1;
    }
    w
}

/// Intersection of two sorted, disjoint span lists.
fn intersect_spans(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() && j < b.len() {
        let start = a[i].0.max(b[j].0);
        let end = a[i].1.min(b[j].1);
        if start < end {
            out.push((start, end));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
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

//! Span raster transforms the lattice members need and the raster module does not have:
//! shifting, point reflection, 2x subsampling and clipping to a column or row range.
//!
//! The subsampling is exact for lattices, which is why the double lattice's coarse stage can
//! trust it: coarse pixel `(X, Y)` is set iff fine pixel `(2X, 2Y)` is, so a lattice of even
//! vectors is admissible at the fine resolution iff its half is admissible at the coarse one.

use crate::span_raster::SpanRaster;

/// `r` moved by `(dx, dy)`.
pub(crate) fn translate(r: &SpanRaster, dx: i32, dy: i32) -> SpanRaster {
    SpanRaster {
        min_y: r.min_y + dy,
        rows: r
            .rows
            .iter()
            .map(|row| row.iter().map(|&(a, b)| (a + dx, b + dx)).collect())
            .collect(),
    }
}

/// Every pixel `(x, y)` of `r` as `(-x, -y)`.
pub(crate) fn negate(r: &SpanRaster) -> SpanRaster {
    SpanRaster {
        min_y: 1 - r.max_y(),
        rows: r
            .rows
            .iter()
            .rev()
            .map(|row| row.iter().rev().map(|&(a, b)| (1 - b, 1 - a)).collect())
            .collect(),
    }
}

/// The pixels of `r` at even coordinates, halved.
pub(crate) fn subsample2(r: &SpanRaster) -> SpanRaster {
    let first = (r.min_y + 1).div_euclid(2);
    let last = (r.max_y() - 1).div_euclid(2);
    let rows = (first..=last)
        .map(|y| {
            r.row(2 * y)
                .iter()
                .map(|&(a, b)| ((a + 1).div_euclid(2), (b + 1).div_euclid(2)))
                .filter(|&(a, b)| a < b)
                .collect()
        })
        .collect();
    SpanRaster { min_y: first, rows }
}

/// The pixels of `r` with `lo <= x < hi`.
pub(crate) fn clip_x(r: &SpanRaster, lo: i32, hi: i32) -> SpanRaster {
    SpanRaster {
        min_y: r.min_y,
        rows: r
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|&(a, b)| (a.max(lo), b.min(hi)))
                    .filter(|&(a, b)| a < b)
                    .collect()
            })
            .collect(),
    }
}

/// The pixels of `r` with `lo <= y < hi`.
pub(crate) fn clip_y(r: &SpanRaster, lo: i32, hi: i32) -> SpanRaster {
    let first = r.min_y.max(lo);
    let end = r.max_y().min(hi);
    let rows = (first..end.max(first)).map(|y| r.row(y).to_vec()).collect();
    SpanRaster { min_y: first, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(r: &SpanRaster) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for (i, row) in r.rows.iter().enumerate() {
            for &(a, b) in row {
                out.extend((a..b).map(|x| (x, r.min_y + i as i32)));
            }
        }
        out.sort_unstable();
        out
    }

    fn sample() -> SpanRaster {
        let mut r = SpanRaster::rect(-3, -2, 4, 1);
        r = r.union(&SpanRaster::rect(6, 2, 9, 3));
        r.difference(&SpanRaster::rect(0, 0, 1, 0))
    }

    #[test]
    fn translate_and_negate_move_every_pixel() {
        let r = sample();
        let moved: Vec<_> = pixels(&r).iter().map(|&(x, y)| (x + 5, y - 3)).collect();
        assert_eq!(pixels(&translate(&r, 5, -3)), moved);
        let mut flipped: Vec<_> = pixels(&r).iter().map(|&(x, y)| (-x, -y)).collect();
        flipped.sort_unstable();
        assert_eq!(pixels(&negate(&r)), flipped);
    }

    #[test]
    fn subsampling_keeps_the_even_pixels() {
        let r = sample();
        let want: Vec<_> = pixels(&r)
            .into_iter()
            .filter(|&(x, y)| x % 2 == 0 && y % 2 == 0)
            .map(|(x, y)| (x / 2, y / 2))
            .collect();
        assert_eq!(pixels(&subsample2(&r)), want);
    }

    #[test]
    fn clipping_keeps_the_range() {
        let r = sample();
        let kept = |keep: fn((i32, i32)) -> bool| -> Vec<_> {
            pixels(&r).into_iter().filter(|&p| keep(p)).collect()
        };
        assert_eq!(pixels(&clip_x(&r, -1, 7)), kept(|p| (-1..7).contains(&p.0)));
        assert_eq!(pixels(&clip_y(&r, 0, 3)), kept(|p| (0..3).contains(&p.1)));
        assert!(pixels(&clip_y(&r, 20, 30)).is_empty());
    }
}

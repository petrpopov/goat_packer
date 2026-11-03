//! The set the bound's norm ball must stay inside: the integer offsets at which two copies
//! of the run's classes collide, made symmetric (a norm ball is).

use crate::pack::context::Ctx;
use crate::span_raster::SpanRaster;

/// `N_ab ∩ -N_ab` for every pair `(a, b)` of `classes` (a ≤ b): an offset `d` here collides
/// whichever copy is the reference, for every pair of the group.
pub(super) fn symmetric_region(ctx: &Ctx, classes: &[usize]) -> SpanRaster {
    let mut region: Option<SpanRaster> = None;
    for (i, &a) in classes.iter().enumerate() {
        for &b in &classes[i..] {
            let raster = ctx.no_fit(a, b);
            let both = intersection(raster, &mirrored(raster));
            region = Some(match region {
                None => both,
                Some(so_far) => intersection(&so_far, &both),
            });
        }
    }
    region.unwrap_or_default()
}

/// `a ∩ b` through the difference the raster already has.
fn intersection(a: &SpanRaster, b: &SpanRaster) -> SpanRaster {
    a.difference(&a.difference(b))
}

/// The raster turned by a half turn about the origin: pixel `(x, y)` goes to `(-x, -y)`.
fn mirrored(raster: &SpanRaster) -> SpanRaster {
    let rows = raster
        .rows
        .iter()
        .rev()
        .map(|row| row.iter().rev().map(|&(s, e)| (1 - e, 1 - s)).collect())
        .collect();
    SpanRaster {
        min_y: 1 - raster.max_y(),
        rows,
    }
}

/// The first pixel of `[x0, x1]` in row `y` that is not in `region`; `None` when the whole
/// run is inside.
pub(super) fn first_missing(region: &SpanRaster, y: i32, x0: i32, x1: i32) -> Option<i32> {
    let row = region.row(y);
    let mut x = x0;
    while x <= x1 {
        let after = row.partition_point(|&(start, _)| start <= x);
        if after == 0 || x >= row[after - 1].1 {
            return Some(x);
        }
        x = row[after - 1].1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirroring_maps_pixels_to_their_opposites() {
        let raster = SpanRaster {
            min_y: 2,
            rows: vec![vec![(3, 6)], vec![(0, 1), (4, 5)]],
        };
        let flipped = mirrored(&raster);
        for y in -4..6 {
            for x in -8..8 {
                assert_eq!(flipped.contains(x, y), raster.contains(-x, -y), "{x},{y}");
            }
        }
    }

    #[test]
    fn a_run_reports_its_first_missing_pixel() {
        let raster = SpanRaster {
            min_y: 0,
            rows: vec![vec![(0, 4), (5, 9)]],
        };
        assert_eq!(first_missing(&raster, 0, 1, 3), None);
        assert_eq!(first_missing(&raster, 0, 1, 6), Some(4));
        assert_eq!(first_missing(&raster, 1, 0, 0), Some(0));
    }
}

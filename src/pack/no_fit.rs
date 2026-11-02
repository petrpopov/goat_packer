//! The exact discrete no-fit set (design 2026-11-02, §8.3): every integer translation of one
//! rotation class relative to another at which some pair rule breaks. A member scans free
//! origins instead of testing collisions: placing a copy at `p` takes `p + no_fit` out of
//! every class's free map. Span Minkowski difference, moved here unchanged from `collision`.

use crate::collision::zone_pairs;
use crate::kind_masks::KindMasks;
use crate::pixel_mask::PixelMask;
use crate::span_raster::{SpanRaster, merge_spans};

/// Every integer shift of B relative to A at which `a` and `b` collide: the zone pairs plus
/// the support cores, which exist only with `support_overlap_mm` (a side without a core has
/// no support base, and the zone pairs already cover it).
pub(crate) fn no_fit(a: &KindMasks, b: &KindMasks) -> SpanRaster {
    let mut pairs = zone_pairs(a, b);
    if let (Some(core_a), Some(core_b)) = (&a.support_core, &b.support_core) {
        pairs.push((core_a, core_b));
    }
    overlap_shifts(&pairs)
}

/// All integer translations of B relative to A at which some pair `(A, B)` shares a pixel.
/// Span difference: [a,b) - [c,d) = [a-d+1,b-c). Holes remain holes.
pub(crate) fn overlap_shifts(pairs: &[(&PixelMask, &PixelMask)]) -> SpanRaster {
    let pairs: Vec<_> = pairs
        .iter()
        .filter(|(a, b)| a.pixel_count > 0 && b.pixel_count > 0)
        .collect();
    let Some(min_y) = pairs
        .iter()
        .map(|(a, b)| a.min_y - b.min_y - b.height as i32 + 1)
        .min()
    else {
        return SpanRaster::default();
    };
    let max_y = pairs
        .iter()
        .map(|(a, b)| a.min_y + a.height as i32 - b.min_y)
        .max()
        .unwrap_or(min_y);
    let mut rows = vec![Vec::new(); (max_y - min_y) as usize];
    for (a, b) in pairs {
        let ar: Vec<_> = (0..a.height).map(|y| a.row_spans(y)).collect();
        let br: Vec<_> = (0..b.height).map(|y| b.row_spans(y)).collect();
        for (ay, spans_a) in ar.iter().enumerate() {
            for (by, spans_b) in br.iter().enumerate() {
                let row = &mut rows[(a.min_y + ay as i32 - b.min_y - by as i32 - min_y) as usize];
                for &(a0, a1) in spans_a {
                    for &(b0, b1) in spans_b {
                        row.push((a.min_x + a0 - b.min_x - b1 + 1, a.min_x + a1 - b.min_x - b0));
                    }
                }
            }
        }
    }
    for row in &mut rows {
        merge_spans(row);
    }
    SpanRaster { min_y, rows }
}

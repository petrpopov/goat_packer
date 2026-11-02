//! When two placements conflict: two prints' kind masks overlapping under the pair rules of
//! `keepout::pair_rules`, or their support cores overlapping. Masks are already grown by half
//! of their rule's gap (`kind_masks`), so a pixel overlap is a broken rule.

use crate::geometry::{Point, Rect};
use crate::keepout::PlateRules;
use crate::kind_masks::{KindMasks, OriginGrid, plate_zones, zone_conflict};
use crate::pixel_mask::{PixelMask, masks_overlap};
use crate::span_raster::{SpanRaster, merge_spans};

/// The mask pairs whose overlap makes two placements collide (`b` shifted relative to `a`):
/// one per pair rule (and band) and direction, as laid out in `KindMasks::slots`. Pairs with
/// an empty side cannot overlap and are left out. Support cores are a separate pair, see
/// `support_cores_overlap`.
pub(crate) fn zone_pairs<'a>(
    a: &'a KindMasks,
    b: &'a KindMasks,
) -> Vec<(&'a PixelMask, &'a PixelMask)> {
    a.slots
        .iter()
        .filter_map(|&(own, other)| {
            let key = a.keys[other];
            // Prints of one job share the layout; another layout is matched by key.
            let index = if b.keys.get(other) == Some(&key) {
                Some(other)
            } else {
                b.keys.iter().position(|known| *known == key)
            }?;
            let pair = (&a.masks[own], &b.masks[index]);
            (pair.0.pixel_count > 0 && pair.1.pixel_count > 0).then_some(pair)
        })
        .collect()
}

/// `b` at pixel offset `(dx, dy)` from `a` breaks a pair rule (`zone_pairs`).
pub(crate) fn zones_collide(a: &KindMasks, b: &KindMasks, dx: i32, dy: i32) -> bool {
    zone_pairs(a, b)
        .into_iter()
        .any(|(za, zb)| masks_overlap(za, 0, 0, zb, dx, dy))
}

/// `b` at pixel offset `(dx, dy)` from `a` collides: a pair rule or the support cores.
pub fn placements_collide(a: &KindMasks, b: &KindMasks, dx: i32, dy: i32) -> bool {
    zones_collide(a, b, dx, dy) || support_cores_overlap(a, b, dx, dy)
}

/// Support-overlap pair: the support cores of two placements (shifted by `(dx, dy)` pixels
/// of `b` relative to `a`) overlap. Never true when either side carries no core.
pub(crate) fn support_cores_overlap(a: &KindMasks, b: &KindMasks, dx: i32, dy: i32) -> bool {
    match (&a.support_core, &b.support_core) {
        (Some(core_a), Some(core_b)) => masks_overlap(core_a, 0, 0, core_b, dx, dy),
        _ => false,
    }
}

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

/// Whether `mask` may sit with its origin at pixel `(center_x_pixels, center_y_pixels)` of a
/// grid anchored at the plate origin, on `bed` with hard `exclusions`: the production zone
/// predicate (`kind_masks::zone_conflict`) on the legacy fields, so a test of this checks
/// what the search enforces.
pub fn placement_is_allowed(
    mask: &KindMasks,
    center_x_pixels: i32,
    center_y_pixels: i32,
    bed: Rect,
    exclusions: &[Rect],
) -> bool {
    let rules = PlateRules::from_legacy(bed, None, exclusions, &[]);
    let zones = plate_zones(&rules, None).expect("legacy rules are in the plate frame");
    let grid = OriginGrid {
        origin: Point { x: 0.0, y: 0.0 },
        resolution: mask.resolution,
    };
    zone_conflict(mask, &zones, &grid, center_x_pixels, center_y_pixels).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keepout::BodyRule;
    use crate::kind_masks::{KindShapes, MaskRules};
    use crate::object_print::{ObjectPrint, PrintKind};

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
        [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
            .iter()
            .map(|&(x, y)| Point { x, y })
            .collect()
    }

    /// An L-shaped body on a wider support base with a narrow landing and two bands: every
    /// pair rule has something to bite on.
    fn print() -> ObjectPrint {
        ObjectPrint {
            silhouette: vec![rect(-3.0, -2.0, 3.0, -1.0), rect(-3.0, -1.0, -2.0, 2.0)],
            landing: vec![rect(-2.0, -2.0, 2.0, -1.5)],
            support_base: vec![rect(-4.0, -3.0, 4.0, -2.0)],
            part_hulls: vec![rect(-3.0, -2.0, 3.0, 2.0)],
            part_boxes: vec![Rect::new(-3.0, -2.0, 3.0, 2.0)],
            body: vec![vec![rect(-3.0, -2.0, 0.0, 2.0)], vec![]],
            support: vec![vec![], vec![rect(3.5, 0.0, 6.0, 1.0)]],
            interface: vec![vec![], vec![]],
            ..ObjectPrint::default()
        }
    }

    fn rules(body_rule: BodyRule, overlap: Option<f64>) -> MaskRules {
        MaskRules {
            clearance_mm: 0.7,
            body_rule,
            support_overlap_mm: overlap,
        }
    }

    fn masks(body_rule: BodyRule, overlap: Option<f64>, angle: f64) -> KindMasks {
        let (sin, cos) = f64::to_radians(angle).sin_cos();
        KindShapes::new(&print(), rules(body_rule, overlap))
            .mapped(|p| Point {
                x: p.x * cos - p.y * sin,
                y: p.x * sin + p.y * cos,
            })
            .rasterize(0.5)
    }

    #[test]
    fn no_fit_matches_brute_force_collisions_at_every_shift() {
        for body_rule in [BodyRule::Silhouette, BodyRule::Bands] {
            for overlap in [None, Some(0.0), Some(1.0)] {
                for angle in [0.0, 13.0, 90.0] {
                    let a = masks(body_rule, overlap, 0.0);
                    let b = masks(body_rule, overlap, angle);
                    let nfp = no_fit(&a, &b);
                    for y in -30..=30 {
                        for x in -30..=30 {
                            assert_eq!(
                                nfp.contains(x, y),
                                placements_collide(&a, &b, x, y),
                                "{body_rule:?} {overlap:?} {angle} shift {x},{y}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn pairs_follow_the_pair_rules_both_ways() {
        let a = masks(BodyRule::Silhouette, None, 0.0);
        // The silhouette once, base against landing both ways; the bands pair support with
        // body only within one band, and no band has both.
        let pairs = zone_pairs(&a, &a);
        let silhouette = a.mask(PrintKind::Silhouette, 0).unwrap();
        assert!(
            pairs
                .iter()
                .any(|(x, y)| std::ptr::eq(*x, silhouette) && std::ptr::eq(*y, silhouette))
        );
        assert_eq!(pairs.len(), 3, "silhouette, base->landing, landing->base");
        // Body bands replace the silhouette.
        let bands = masks(BodyRule::Bands, None, 0.0);
        assert!(bands.mask(PrintKind::Silhouette, 0).is_none());
        assert_eq!(zone_pairs(&bands, &bands).len(), 3);
    }

    #[test]
    fn a_support_band_meets_a_foreign_body_band_only_in_its_band() {
        let mut other = print();
        other.body[1] = vec![rect(-3.0, -2.0, 0.0, 2.0)];
        let rules = rules(BodyRule::Silhouette, None);
        let a = KindShapes::new(&print(), rules).rasterize(0.5);
        let b = KindShapes::new(&other, rules).rasterize(0.5);
        // B 8 mm right: its band-1 body (x 5..8) meets A's band-1 support (x 3.5..6).
        assert!(placements_collide(&a, &b, 16, 0));
        // The same B without that band body: silhouettes and bases are clear at 8 mm.
        assert!(!placements_collide(&a, &a, 16, 0));
    }
}

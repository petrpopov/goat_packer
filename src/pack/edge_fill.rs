//! Bottom-left fill of a seeded layout (design 2026-11-02, §9.2): a lattice leaves gaps at
//! the plate's edges, and a greedy sweep from each of the four corners puts more copies, of
//! any rotation class, into them. The seed is placed first (its no-fit sets leave every
//! class's free map), then each corner fills its own copy of the maps; the best corner wins.
//!
//! Frame convention: a copy is a pure translation of its class (see `pack`), so the free
//! maps are over frame origins and a copy's far edge is its origin plus the class's extent.

use super::context::Ctx;
use super::free_map::FreeMap;
use crate::layout::{PixelPlacement, cap_reached};

/// Bottom positions are compared as integers at this many units per pixel, so float noise
/// cannot reorder two copies that reach equally far.
const HEIGHT_QUANTUM: f64 = 1e6;

/// The corners a fill starts from: bit 0 = from the right, bit 1 = from the top.
const CORNERS: usize = 4;

/// The seed plus the best corner fill.
pub(crate) struct Filled {
    pub(crate) layout: Vec<PixelPlacement>,
    /// The corner of the winning fill (0 when nothing was added).
    pub(crate) corner: usize,
}

/// `seed` (local classes, mutually legal and free at `anchor`) completed from the four corners
/// with every class of `ctx`, stopping at `cap` copies when set. The most copies win, then the
/// smaller box, then the lower corner.
pub(crate) fn edge_fill(
    ctx: &Ctx,
    anchor: Option<usize>,
    seed: &[PixelPlacement],
    cap: usize,
) -> Filled {
    ctx.prepare_no_fit();
    let mut free: Vec<FreeMap> = (0..ctx.classes())
        .map(|class| ctx.free_map(class, anchor))
        .collect();
    for p in seed {
        place(ctx, &mut free, *p);
    }
    let mut best: Option<(Vec<PixelPlacement>, usize, u64)> = None;
    for corner in 0..CORNERS {
        let layout = fill_from(ctx, free.clone(), seed, corner, cap);
        let area = ctx.bbox_area(&layout);
        let better = best.as_ref().is_none_or(|(b, _, a)| {
            (layout.len(), std::cmp::Reverse(area)) > (b.len(), std::cmp::Reverse(*a))
        });
        if better {
            best = Some((layout, corner, area));
        }
    }
    let (layout, corner, _) = best.expect("four corners ran");
    Filled { layout, corner }
}

fn fill_from(
    ctx: &Ctx,
    mut free: Vec<FreeMap>,
    seed: &[PixelPlacement],
    corner: usize,
    cap: usize,
) -> Vec<PixelPlacement> {
    let mut layout = seed.to_vec();
    while !cap_reached(layout.len(), cap) {
        let Some(next) = next_copy(ctx, &free, corner) else {
            break;
        };
        place(ctx, &mut free, next);
        layout.push(next);
    }
    layout
}

/// Takes a copy's no-fit set out of every class's map.
fn place(ctx: &Ctx, free: &mut [FreeMap], p: PixelPlacement) {
    for (class, map) in free.iter_mut().enumerate() {
        map.exclude(ctx.no_fit(p.class, class), p.x, p.y);
    }
}

/// The free origin, over every class, whose copy reaches least far from `corner`; ties go to
/// the one nearest the corner's side, then to the lower class.
fn next_copy(ctx: &Ctx, free: &[FreeMap], corner: usize) -> Option<PixelPlacement> {
    let (from_right, from_top) = (corner & 1 != 0, corner & 2 != 0);
    let resolution = ctx.grid().resolution;
    free.iter()
        .enumerate()
        .filter_map(|(class, map)| {
            let (x, y) = map.first_in(0..map.width(), from_right, from_top)?;
            let b = ctx.masks(class).extent_bounds;
            let reach = if from_top {
                -f64::from(y) - b.min_y / resolution
            } else {
                f64::from(y) + b.max_y / resolution
            };
            let key = (
                (reach * HEIGHT_QUANTUM).round() as i64,
                if from_right { -x } else { x },
                class,
            );
            Some((key, PixelPlacement { class, x, y }))
        })
        .min_by_key(|(key, _)| *key)
        .map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Point, Rect};
    use crate::keepout::PlateRules;
    use crate::kind_masks::{KindShapes, MaskRules, OriginGrid, plate_zones};
    use crate::object_print::ObjectPrint;

    fn rect_print(w: f64, h: f64) -> ObjectPrint {
        let ring: Vec<Point> = [(-w, -h), (w, -h), (w, h), (-w, h), (-w, -h)]
            .iter()
            .map(|&(x, y)| Point {
                x: x / 2.0,
                y: y / 2.0,
            })
            .collect();
        ObjectPrint {
            silhouette: vec![ring.clone()],
            part_hulls: vec![ring.clone()],
            part_boxes: vec![Rect::of_points(ring.iter())],
            landing: vec![ring.clone()],
            support_base: vec![ring.clone()],
            body: vec![vec![ring]],
            support: vec![vec![]],
            interface: vec![vec![]],
            ..ObjectPrint::default()
        }
    }

    /// Fill a 60 x 30 mm plate with 10 x 6 mm boxes (0.5 mm clearance) from one seed copy.
    #[test]
    fn the_fill_completes_a_seed_and_never_breaks_a_pair_rule() {
        let rules = PlateRules {
            printable_area: Rect::new(0.0, 0.0, 60.0, 30.0),
            body_edge_band_mm: 0.0,
            print_edge_reserve_mm: 0.0,
            support_clip_mm: 0.0,
            support_clip_sides: [true; 4],
            keepouts: Vec::new(),
            body_rule: Default::default(),
        };
        let zones = plate_zones(&rules, None).unwrap();
        let grid = OriginGrid {
            origin: Point { x: 0.0, y: 0.0 },
            resolution: 0.5,
        };
        let mask_rules = MaskRules {
            clearance_mm: 0.5,
            foreign: Default::default(),
            body_rule: Default::default(),
            support_overlap_mm: None,
        };
        let masks = vec![KindShapes::new(&rect_print(10.0, 6.0), mask_rules).rasterize(0.5)];
        let ctx = Ctx::new(masks, vec![0], &zones, grid);
        let seed = [PixelPlacement {
            class: 0,
            x: 60,
            y: 30,
        }];
        let filled = edge_fill(&ctx, None, &seed, 0);
        // Rows of 5 boxes (10.5 mm pitch in 60 mm) and 4 rows (6.5 mm pitch in 30 mm).
        assert!(filled.layout.len() >= 16, "{}", filled.layout.len());
        assert_eq!(filled.layout[0], seed[0], "the seed stays first");
        let no_fit = ctx.no_fit(0, 0);
        for (i, a) in filled.layout.iter().enumerate() {
            assert!(ctx.origins(0).contains(a.x, a.y), "{a:?} on the plate");
            for b in &filled.layout[i + 1..] {
                assert!(!no_fit.contains(b.x - a.x, b.y - a.y), "{a:?} {b:?}");
            }
        }
        let capped = edge_fill(&ctx, None, &seed, 7);
        assert_eq!(capped.layout.len(), 7);
    }
}

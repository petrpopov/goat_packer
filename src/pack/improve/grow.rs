//! Growing one layout: insert a copy where the overlap is least, resolve the overlaps by
//! moving the copies (`separate`; only those that overlap move, the others give way when a
//! neighbour pushes them), keep it when it resolves; repeat. When no insertion origin yields
//! another copy the layout is pulled into each corner in turn (`compact`), which moves its
//! slack elsewhere, and searched again (ruin and recreate with the whole layout as the
//! ruin): a success restarts the corners, four failures end the search.

use super::compact::compact;
use super::insert::CostMap;
use super::separate::{Field, separate};
use super::windows::plate_window;
use crate::layout::{PixelPlacement, cap_reached};
use crate::pack::context::Ctx;

/// Insertion origins are sampled every this many pixels.
const SAMPLE_STEP: usize = 2;
/// Iterations of one try.
const TRY_ITERATIONS: usize = 200;
const CORNERS: usize = 4;

/// `layout` with as many extra copies as the search finds (`cap` limits them, 0 = no cap).
/// Every round tries the `spots` cheapest insertion origins; the layout as it is, then pulled
/// into each corner, gets its own passes.
pub(super) fn grow(
    ctx: &Ctx,
    field: &Field,
    layout: &[PixelPlacement],
    spots: usize,
    cap: usize,
) -> Vec<PixelPlacement> {
    let mut items = insert_pass(ctx, field, layout.to_vec(), spots, cap);
    let mut corner = 0;
    while corner < CORNERS && !cap_reached(items.len(), cap) {
        let packed = compact(field, &items, corner);
        corner += 1;
        if packed == items {
            continue;
        }
        let grown = insert_pass(ctx, field, packed, spots, cap);
        if grown.len() > items.len() {
            items = grown;
            corner = 0;
        }
    }
    items
}

/// Inserts copies one after the other until no origin yields one more.
fn insert_pass(
    ctx: &Ctx,
    field: &Field,
    mut items: Vec<PixelPlacement>,
    spots: usize,
    cap: usize,
) -> Vec<PixelPlacement> {
    while !cap_reached(items.len(), cap) {
        let Some(next) = insert_one(ctx, field, &items, spots) else {
            break;
        };
        items = next;
    }
    items
}

/// One more copy, from the cheapest insertion origins first.
fn insert_one(
    ctx: &Ctx,
    field: &Field,
    items: &[PixelPlacement],
    spots: usize,
) -> Option<Vec<PixelPlacement>> {
    let costs = CostMap::new(field, items, plate_window(ctx)?, SAMPLE_STEP);
    costs
        .spots(spots)
        .into_iter()
        .find_map(|(_, spot)| try_insert(ctx, field, items, spot))
}

/// `items` plus a copy at `spot`, the overlaps resolved; `None` when they do not resolve or
/// the result fails the exact pair check.
fn try_insert(
    ctx: &Ctx,
    field: &Field,
    items: &[PixelPlacement],
    spot: PixelPlacement,
) -> Option<Vec<PixelPlacement>> {
    let mut trial = items.to_vec();
    trial.push(spot);
    let mobile: Vec<usize> = (0..trial.len()).collect();
    let resolved = separate(field, &mut trial, &mobile, TRY_ITERATIONS);
    (resolved && is_legal(ctx, field, &trial)).then_some(trial)
}

/// The exact check on the no-fit sets and free maps, independent of the depth maps: the
/// layout is only ever published through it.
pub(super) fn is_legal(ctx: &Ctx, field: &Field, items: &[PixelPlacement]) -> bool {
    items.iter().enumerate().all(|(i, a)| {
        field.free[a.class].contains(a.x, a.y)
            && items[i + 1..]
                .iter()
                .all(|b| !ctx.no_fit(a.class, b.class).contains(b.x - a.x, b.y - a.y))
    })
}

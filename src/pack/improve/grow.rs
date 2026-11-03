//! Growing one layout: insert a copy where the overlap is least, resolve the overlaps by
//! moving the copies of a window (`separate`), keep it when it resolves; repeat. When no
//! window yields another copy the layout is pulled into each corner in turn (`compact`),
//! which moves its slack elsewhere, and searched again (ruin and recreate with the whole
//! layout as the ruin): a success restarts the corners, four failures end the search.

use super::compact::compact;
use super::insert::CostMap;
use super::separate::{Field, separate};
use super::windows::{Window, window_list};
use crate::layout::{PixelPlacement, cap_reached};
use crate::pack::context::Ctx;
use std::collections::HashSet;

/// Insertion origins are sampled every this many pixels.
const SAMPLE_STEP: usize = 2;
/// Insertion origins tried per window, and iterations of one try.
const SPOTS_PER_WINDOW: usize = 24;
const TRY_ITERATIONS: usize = 200;
const CORNERS: usize = 4;

/// `layout` with as many extra copies as the search finds (`cap` limits them, 0 = no cap).
/// Each pass (the layout as it is, then pulled into each corner) gets `tries` tries; one try is
/// one insertion origin separated.
pub(super) fn grow(
    ctx: &Ctx,
    field: &Field,
    anchor: Option<usize>,
    layout: &[PixelPlacement],
    tries: usize,
    cap: usize,
) -> Vec<PixelPlacement> {
    let windows = window_list(ctx, anchor);
    let mut items = insert_pass(ctx, field, &windows, layout.to_vec(), tries, cap);
    let mut corner = 0;
    while corner < CORNERS && !cap_reached(items.len(), cap) {
        let packed = compact(field, &items, corner);
        corner += 1;
        if packed == items {
            continue;
        }
        let grown = insert_pass(ctx, field, &windows, packed, tries, cap);
        if grown.len() > items.len() {
            items = grown;
            corner = 0;
        }
    }
    items
}

/// Inserts copies one after the other until no window yields one more.
fn insert_pass(
    ctx: &Ctx,
    field: &Field,
    windows: &[Window],
    mut items: Vec<PixelPlacement>,
    mut tries: usize,
    cap: usize,
) -> Vec<PixelPlacement> {
    // Origins that failed on the current layout: overlapping windows share them.
    let mut failed: HashSet<PixelPlacement> = HashSet::new();
    while tries > 0 && !cap_reached(items.len(), cap) {
        let Some(next) = insert_one(ctx, field, windows, &items, &mut failed, &mut tries) else {
            break;
        };
        items = next;
        failed.clear();
    }
    items
}

/// One more copy: the windows in order of room, the origins of each cheapest first.
fn insert_one(
    ctx: &Ctx,
    field: &Field,
    windows: &[Window],
    items: &[PixelPlacement],
    failed: &mut HashSet<PixelPlacement>,
    tries: &mut usize,
) -> Option<Vec<PixelPlacement>> {
    // The last window is the whole plate.
    let costs = CostMap::new(field, items, *windows.last()?, SAMPLE_STEP);
    let mut ranked: Vec<(u32, usize)> = windows
        .iter()
        .enumerate()
        .filter_map(|(w, &window)| costs.best(window).map(|cost| (cost, w)))
        .collect();
    ranked.sort_unstable();
    for (_, w) in ranked {
        for (_, spot) in costs.spots(windows[w], SPOTS_PER_WINDOW) {
            if *tries == 0 {
                return None;
            }
            if !failed.insert(spot) {
                continue;
            }
            *tries -= 1;
            if let Some(next) = try_insert(ctx, field, items, spot, windows[w]) {
                return Some(next);
            }
        }
    }
    None
}

/// `items` plus a copy at `spot`, the overlaps resolved by moving the copies of `window`;
/// `None` when they do not resolve or the result fails the exact pair check.
fn try_insert(
    ctx: &Ctx,
    field: &Field,
    items: &[PixelPlacement],
    spot: PixelPlacement,
    window: Window,
) -> Option<Vec<PixelPlacement>> {
    let mut trial = items.to_vec();
    trial.push(spot);
    let mobile: Vec<usize> = (0..trial.len())
        .filter(|&i| window.contains(trial[i].x, trial[i].y))
        .collect();
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

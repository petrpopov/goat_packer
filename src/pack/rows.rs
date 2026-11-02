//! Brick rows (moved from `grid`): copies of one class on a sheared row lattice whose rows
//! are the tightest horizontal pitch apart, every row shifted by `shift` against the one
//! below, the shift scanned. The general lattice (any first vector, phase search, edge fill)
//! is work package P4's member; this one keeps the old grid baseline as a member.

use super::context::Ctx;
use crate::layout::{PixelPlacement, cap_reached};

/// Row shifts tried, in pixels: every other pixel (0.5 mm at the usual 0.25 mm resolution)
/// keeps the scan well under a second for a goat-sized patch.
const SHIFT_STEP_PX: usize = 2;

/// Rows checked above a row when spacing a sheared lattice: copies further up are far enough
/// for any patch that fits on a bed; the fill still checks every placed copy.
const SPACING_ROWS_CHECKED: i32 = 3;

/// A sheared lattice: rows `pitch_y` apart, each shifted by `shift` against the one below.
#[derive(Clone, Copy)]
struct Lattice {
    pitch_x: i32,
    pitch_y: i32,
    shift: i32,
}

/// The densest brick-row layout of `class` (local id) with the tower at `anchor`, and its
/// shift in pixels. Shaped models nest: a U1 goat patch (47.6 x 43 mm) tiles a 270 mm bed
/// with 48 in brick rows shifted by half a pitch (a hand-made plate printed so), while the
/// rectangular grid fits 37. Shift 0 (the plain grid) runs first, so a tie keeps it.
pub(crate) fn brick_rows(
    ctx: &Ctx,
    class: usize,
    anchor: Option<usize>,
    cap: usize,
) -> (Vec<PixelPlacement>, i32) {
    let Some(range) = ctx.origins(class).bounds() else {
        return (Vec::new(), 0);
    };
    let pitch_x = row_pitch(ctx, class);
    let mut best: (Vec<PixelPlacement>, i32) = (Vec::new(), 0);
    for shift in (0..pitch_x).step_by(SHIFT_STEP_PX) {
        let lattice = Lattice {
            pitch_x,
            pitch_y: row_spacing(ctx, class, pitch_x, shift),
            shift,
        };
        let placed = fill_rows(ctx, class, anchor, range, lattice, cap);
        if placed.len() > best.0.len() {
            best = (placed, shift);
        }
    }
    best
}

/// Row `r` starts `r * shift mod pitch_x` right of the first feasible origin; a cell is
/// taken when it is still free (plate zones, the tower, every copy placed so far).
fn fill_rows(
    ctx: &Ctx,
    class: usize,
    anchor: Option<usize>,
    (min_x, min_y, max_x, max_y): (i32, i32, i32, i32),
    Lattice {
        pitch_x,
        pitch_y,
        shift,
    }: Lattice,
    cap: usize,
) -> Vec<PixelPlacement> {
    let mut free = ctx.free_map(class, anchor);
    let no_fit = ctx.no_fit(class, class);
    let mut placed: Vec<PixelPlacement> = Vec::new();
    let mut y = min_y;
    let mut row = 0;
    while y <= max_y {
        let mut x = min_x + (row * shift).rem_euclid(pitch_x);
        while x <= max_x {
            if free.contains(x, y) {
                placed.push(PixelPlacement { class, x, y });
                free.exclude(no_fit, x, y);
                if cap_reached(placed.len(), cap) {
                    return placed;
                }
            }
            x += pitch_x;
        }
        y += pitch_y;
        row += 1;
    }
    placed
}

/// The SMALLEST whole-pixel x shift where two copies satisfy the pair rules — not the full
/// bounding box.
fn row_pitch(ctx: &Ctx, class: usize) -> i32 {
    let reach = ctx.masks(class).reach_px();
    let no_fit = ctx.no_fit(class, class);
    (1..=reach)
        .find(|&s| !no_fit.contains(s, 0))
        .unwrap_or(reach)
}

/// The smallest row spacing of a lattice with copies `pitch_x` apart in a row and rows
/// shifted by `shift`: every copy of the next few rows clears the copy at the origin.
/// Distant lattice neighbours can still conflict for disconnected shapes; `fill_rows`
/// checks every placed copy.
fn row_spacing(ctx: &Ctx, class: usize, pitch_x: i32, shift: i32) -> i32 {
    let no_fit = ctx.no_fit(class, class);
    let reach = ctx.masks(class).reach_px() / pitch_x + 1;
    let clears = |dy: i32| {
        (1..=SPACING_ROWS_CHECKED).all(|row| {
            let offset = (row * shift).rem_euclid(pitch_x);
            (-reach - 1..=reach).all(|k| !no_fit.contains(offset + k * pitch_x, row * dy))
        })
    };
    (1..)
        .find(|&dy| clears(dy))
        .expect("rows clear once they no longer overlap")
}

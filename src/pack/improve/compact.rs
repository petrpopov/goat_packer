//! Gravity toward a corner: every copy slides along x, then along y, as far as it can go
//! without overlapping a neighbour or leaving its free origins, and the sweep repeats until
//! nothing moves. The layout keeps its count and gathers its slack on the far side, where the
//! insertion search finds pockets that the lattice's own phase had spread thin.

use super::separate::Field;
use crate::layout::PixelPlacement;

/// Sweeps at most; the slack of a clipped lattice is gone after a few.
const MAX_SWEEPS: usize = 12;
/// A copy slides at most this many pixels per axis and sweep.
const MAX_SLIDE: i32 = 400;

/// `items` pulled toward `corner` (bit 0 = right, bit 1 = top). Order of the copies: nearest
/// to the corner first, ties by index, so it is deterministic and the front copies settle
/// against the wall before the rest follow.
pub(super) fn compact(
    field: &Field,
    items: &[PixelPlacement],
    corner: usize,
) -> Vec<PixelPlacement> {
    let (sx, sy) = (
        if corner & 1 == 0 { -1 } else { 1 },
        if corner & 2 == 0 { -1 } else { 1 },
    );
    let mut items = items.to_vec();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| (-(sx * items[i].x + sy * items[i].y), i));
    for _ in 0..MAX_SWEEPS {
        let mut moved = false;
        for &i in &order {
            moved |= slide(field, &mut items, i, sx, 0);
            moved |= slide(field, &mut items, i, 0, sy);
        }
        if !moved {
            break;
        }
    }
    items
}

/// Moves copy `i` along `(dx, dy)` while the next origin is free and overlap-free.
fn slide(field: &Field, items: &mut [PixelPlacement], i: usize, dx: i32, dy: i32) -> bool {
    let start = items[i];
    let near = field.neighbours(items, start.x, start.y, MAX_SLIDE, Some(i));
    let mut at = start;
    for _ in 0..MAX_SLIDE {
        let next = PixelPlacement {
            x: at.x + dx,
            y: at.y + dy,
            ..at
        };
        let legal = field.free[next.class].contains(next.x, next.y)
            && field.overlap(items, &near, next.class, next.x, next.y) == 0;
        if !legal {
            break;
        }
        at = next;
    }
    items[i] = at;
    at != start
}

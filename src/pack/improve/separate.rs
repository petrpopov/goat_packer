//! Guided local search that removes overlaps (after Umetani et al. 2009 and Umetani and
//! Murakami 2022): the copies in `mobile` move by coordinate descent on the weighted sum of
//! penetration depths, every copy stays on a free origin of its class, and when a whole sweep
//! finds no better position the weights of the overlapping pairs grow in proportion to their
//! depth, so the pairs that resist get more attention. No random numbers: the order of the
//! copies rotates with the iteration and ties keep the first position found. (The prototype
//! also jumped to a strided grid of the window every few iterations; that cost a hundred
//! times a line search and found nothing the tries from other insertion origins do not.)

use super::penetration::DepthTable;
use crate::layout::PixelPlacement;
use crate::pack::free_map::FreeMap;

/// A move shifts one copy along one axis by at most this many pixels (or turns its class).
const MOVE_RADIUS: i32 = 10;
/// A try ends when the total overlap has not reached a new low for this many iterations.
const STALL_ITERATIONS: usize = 80;
/// A move must lower the weighted cost by more than this.
const EPSILON: f32 = 1e-6;

/// What the search needs to know about the run.
pub(super) struct Field<'a> {
    pub(super) depth: &'a DepthTable,
    /// Free origins per class (plate zones and the tower at the layout's anchor).
    pub(super) free: &'a [FreeMap],
}

impl Field<'_> {
    /// The copies of `items` close enough to a copy near `(x, y)` to overlap it, `skip`
    /// excluded.
    pub(super) fn neighbours(
        &self,
        items: &[PixelPlacement],
        x: i32,
        y: i32,
        slack: i32,
        skip: Option<usize>,
    ) -> Vec<usize> {
        let reach = self.depth.reach() + slack;
        (0..items.len())
            .filter(|&j| Some(j) != skip)
            .filter(|&j| (items[j].x - x).abs() <= reach && (items[j].y - y).abs() <= reach)
            .collect()
    }

    /// Sum of depths of a copy of `class` at `(x, y)` against `near`.
    #[inline]
    pub(super) fn overlap(
        &self,
        items: &[PixelPlacement],
        near: &[usize],
        class: usize,
        x: i32,
        y: i32,
    ) -> u32 {
        near.iter()
            .map(|&j| {
                let o = items[j];
                self.depth.at(o.class, class, x - o.x, y - o.y)
            })
            .sum()
    }

    fn weighted(
        &self,
        items: &[PixelPlacement],
        weights: &Weights,
        i: usize,
        near: &[usize],
        at: PixelPlacement,
    ) -> f32 {
        near.iter()
            .map(|&j| {
                let o = items[j];
                let d = self.depth.at(o.class, at.class, at.x - o.x, at.y - o.y);
                if d == 0 {
                    0.0
                } else {
                    d as f32 * weights.get(i, j)
                }
            })
            .sum()
    }
}

/// Pair weights, symmetric, all 1 at the start.
struct Weights {
    n: usize,
    w: Vec<f32>,
}

impl Weights {
    fn new(n: usize) -> Weights {
        Weights {
            n,
            w: vec![1.0; n * n],
        }
    }

    fn get(&self, i: usize, j: usize) -> f32 {
        self.w[i * self.n + j]
    }

    fn grow(&mut self, i: usize, j: usize, by: f32) {
        self.w[i * self.n + j] += by;
        self.w[j * self.n + i] += by;
    }
}

/// Tries to resolve every overlap of `items` by moving only `mobile` (indices into `items`)
/// within `max_iters` iterations. True when no pair overlaps any more; `items` then holds the
/// legal layout (whatever the answer, it holds the last state reached).
pub(super) fn separate(
    field: &Field,
    items: &mut [PixelPlacement],
    mobile: &[usize],
    max_iters: usize,
) -> bool {
    let mut weights = Weights::new(items.len());
    // Total overlap (each pair counted from both sides) of the best iteration so far.
    let (mut best_total, mut best_iteration) = (u32::MAX, 0);
    for iteration in 0..max_iters {
        let overlaps: Vec<(usize, u32)> = mobile
            .iter()
            .map(|&i| (i, overlap_of(field, items, i)))
            .filter(|&(_, o)| o > 0)
            .collect();
        if overlaps.is_empty() {
            return true;
        }
        let total: u32 = overlaps.iter().map(|&(_, o)| o).sum();
        if total < best_total {
            (best_total, best_iteration) = (total, iteration);
        } else if iteration - best_iteration > STALL_ITERATIONS {
            return false;
        }
        let colliding: Vec<usize> = overlaps.iter().map(|&(i, _)| i).collect();
        let mut improved = false;
        for k in 0..colliding.len() {
            let i = colliding[(k + iteration) % colliding.len()];
            improved |= move_item(field, items, &weights, i);
        }
        if !improved {
            grow_weights(field, items, mobile, &mut weights);
        }
    }
    mobile.iter().all(|&i| overlap_of(field, items, i) == 0)
}

fn overlap_of(field: &Field, items: &[PixelPlacement], i: usize) -> u32 {
    let near = field.neighbours(items, items[i].x, items[i].y, 0, Some(i));
    field.overlap(items, &near, items[i].class, items[i].x, items[i].y)
}

/// Coordinate descent step of copy `i`: the best position on the x line through it (every
/// class), then on the y line. True when the copy moved.
fn move_item(field: &Field, items: &mut [PixelPlacement], weights: &Weights, i: usize) -> bool {
    let start = items[i];
    let near = field.neighbours(items, start.x, start.y, 2 * MOVE_RADIUS, Some(i));
    let mut best = (field.weighted(items, weights, i, &near, start), start);
    for axis in 0..2 {
        let at = best.1;
        let line = (-MOVE_RADIUS..=MOVE_RADIUS).flat_map(|d| {
            (0..field.depth.classes()).map(move |class| {
                let (dx, dy) = if axis == 0 { (d, 0) } else { (0, d) };
                PixelPlacement {
                    class,
                    x: at.x + dx,
                    y: at.y + dy,
                }
            })
        });
        consider(field, items, weights, i, &near, line, &mut best);
    }
    items[i] = best.1;
    best.1 != start
}

/// Lowers `best` to the first strictly cheaper free position of `positions`.
fn consider(
    field: &Field,
    items: &[PixelPlacement],
    weights: &Weights,
    i: usize,
    near: &[usize],
    positions: impl Iterator<Item = PixelPlacement>,
    best: &mut (f32, PixelPlacement),
) {
    for p in positions {
        if p == best.1 || !field.free[p.class].contains(p.x, p.y) {
            continue;
        }
        let cost = field.weighted(items, weights, i, near, p);
        if cost < best.0 - EPSILON {
            *best = (cost, p);
        }
    }
}

/// The penalty step: every overlapping pair with a mobile member grows by its depth over the
/// deepest depth.
fn grow_weights(field: &Field, items: &[PixelPlacement], mobile: &[usize], weights: &mut Weights) {
    let mut pairs = Vec::new();
    for &i in mobile {
        let near = field.neighbours(items, items[i].x, items[i].y, 0, Some(i));
        for j in near {
            let both_mobile = mobile.contains(&j);
            if both_mobile && j < i {
                continue;
            }
            let (a, b) = (items[j], items[i]);
            let d = field.depth.at(a.class, b.class, b.x - a.x, b.y - a.y);
            if d > 0 {
                pairs.push((i, j, d));
            }
        }
    }
    let Some(deepest) = pairs.iter().map(|p| p.2).max() else {
        return;
    };
    for (i, j, d) in pairs {
        weights.grow(i, j, d as f32 / deepest as f32);
    }
}

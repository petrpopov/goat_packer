//! Member C, two blocks (design 2026-11-02, §9.2): one guillotine cut of the plate, on a
//! 2 mm grid in x or y, and a lattice of its own on each side. A plate whose size is not a
//! multiple of the best lattice's cell wastes a strip at the far edge; two lattices (the
//! same one with another phase, another class, another shear) trade that strip for a seam.
//!
//! The seam is exact, not estimated: the first block's copies are placed, their no-fit sets
//! leave the second block's origins, and only then is the second lattice's best coset taken
//! from what remains. Both blocks' own pairs are legal by lattice admissibility, so the whole
//! layout is legal by construction; `pack` checks the winner again anyway.
//!
//! Cuts and lattice pairs are ranked by the sum of the blocks' plain best counts (an upper
//! bound), a fixed ordered list: the effort only reads further down it.

use super::hnf::Hnf;
use super::residue::histogram;
use super::single::{Budget, Scored};
use super::span_ops::{clip_x, clip_y, translate};
use crate::layout::PixelPlacement;
use crate::pack::context::Ctx;
use crate::pack::edge_fill::edge_fill;
use crate::pack::select::{Candidate, Member};
use crate::span_raster::SpanRaster;
use rayon::prelude::*;
use std::cmp::Reverse;

/// Spacing of the cut positions, mm (design: 2 mm).
pub const CUT_GRID_MM: f64 = 2.0;
/// Lattices per class a block may use: the best of member A's ranking.
pub const SIDE_LATTICES: usize = 3;
/// Left-block phases tried per combination (the best, then the runner-up).
const LEFT_PHASES: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Axis {
    X,
    Y,
}

/// A lattice a block may use.
#[derive(Clone, Copy)]
struct Side {
    class: usize,
    lattice: Hnf,
}

/// A cut with the lattices of its two sides; the first side holds origins below `cut`.
#[derive(Clone, Copy)]
struct Combo {
    axis: Axis,
    cut: i32,
    first: usize,
    second: usize,
    bound: u32,
}

/// Two-block candidates at each of `anchors` (tower slots; the lattice members' best ones),
/// for the best `budget.lattices * 2` combinations.
pub(crate) fn two_block(
    ctx: &Ctx,
    ranked: &[Vec<Scored>],
    anchors: &[Option<usize>],
    budget: Budget,
    cap: usize,
) -> Vec<Candidate> {
    ctx.prepare_no_fit();
    let sides = side_lattices(ranked);
    let combos = ranked_combos(ctx, &sides, budget.lattices * 2);
    let phases = budget.phases.min(LEFT_PHASES);
    let jobs: Vec<(Combo, Option<usize>, usize)> = combos
        .iter()
        .flat_map(|&c| {
            anchors
                .iter()
                .flat_map(move |&a| (0..phases).map(move |p| (c, a, p)))
        })
        .collect();
    jobs.par_iter()
        .filter_map(|&(combo, anchor, rank)| build(ctx, &sides, combo, anchor, rank, cap))
        .collect()
}

/// The best `SIDE_LATTICES` of every class's ranking, class order then rank.
fn side_lattices(ranked: &[Vec<Scored>]) -> Vec<Side> {
    ranked
        .iter()
        .enumerate()
        .flat_map(|(class, scored)| {
            scored.iter().take(SIDE_LATTICES).map(move |s| Side {
                class,
                lattice: s.lattice(),
            })
        })
        .collect()
}

/// The cut positions of an axis: every `CUT_GRID_MM` strictly inside the origins' box.
fn cut_positions(ctx: &Ctx, axis: Axis) -> Vec<i32> {
    let boxes = (0..ctx.classes()).filter_map(|c| ctx.origins(c).bounds());
    let range = boxes.fold(None, |acc: Option<(i32, i32)>, b| {
        let (lo, hi) = match axis {
            Axis::X => (b.0, b.2),
            Axis::Y => (b.1, b.3),
        };
        Some(acc.map_or((lo, hi), |(l, h)| (l.min(lo), h.max(hi))))
    });
    let step = ((CUT_GRID_MM / ctx.grid().resolution).round() as i32).max(1);
    let Some((lo, hi)) = range else {
        return Vec::new();
    };
    (1..)
        .map(|k| lo + k * step)
        .take_while(|&c| c <= hi)
        .collect()
}

/// `base` below the cut (`below`) or from it, along `axis`.
fn region(base: &SpanRaster, axis: Axis, cut: i32, below: bool) -> SpanRaster {
    let (lo, hi) = if below {
        (i32::MIN, cut)
    } else {
        (cut, i32::MAX)
    };
    match axis {
        Axis::X => clip_x(base, lo, hi),
        Axis::Y => clip_y(base, lo, hi),
    }
}

/// The most origins any coset of `lattice` has in `spans`.
fn best_count(spans: &SpanRaster, lattice: &Hnf) -> u32 {
    histogram(spans, lattice).into_iter().max().unwrap_or(0)
}

/// Every (cut, first lattice, second lattice) by falling bound, the best `limit`.
fn ranked_combos(ctx: &Ctx, sides: &[Side], limit: usize) -> Vec<Combo> {
    let cuts: Vec<(Axis, i32)> = [Axis::X, Axis::Y]
        .into_iter()
        .flat_map(|axis| cut_positions(ctx, axis).into_iter().map(move |c| (axis, c)))
        .collect();
    let per_cut: Vec<Vec<Combo>> = cuts
        .par_iter()
        .map(|&(axis, cut)| {
            let counts = |below: bool| -> Vec<u32> {
                sides
                    .iter()
                    .map(|s| {
                        let base = &ctx.origins(s.class).allowed;
                        best_count(&region(base, axis, cut, below), &s.lattice)
                    })
                    .collect()
            };
            let (first, second) = (counts(true), counts(false));
            let mut combos = Vec::new();
            for (i, &a) in first.iter().enumerate() {
                for (j, &b) in second.iter().enumerate() {
                    combos.push(Combo {
                        axis,
                        cut,
                        first: i,
                        second: j,
                        bound: a + b,
                    });
                }
            }
            combos
        })
        .collect();
    let mut combos: Vec<Combo> = per_cut.into_iter().flatten().collect();
    // The sort is stable: ties keep the generation order (axis, cut, first, second).
    combos.sort_by_key(|c| Reverse(c.bound));
    combos.truncate(limit);
    combos
}

/// The origins of `class` on one side of the cut, the tower at `anchor` removed.
fn side_origins(
    ctx: &Ctx,
    class: usize,
    anchor: Option<usize>,
    combo: &Combo,
    below: bool,
) -> SpanRaster {
    let mut spans = region(&ctx.origins(class).allowed, combo.axis, combo.cut, below);
    if let Some(anchor) = anchor {
        spans = spans.difference(&ctx.towers()[anchor].forbidden(class).spans);
    }
    spans
}

/// The coset of `lattice` with the `rank`-th most origins in `free` (count down, coset up).
fn nth_coset(free: &SpanRaster, lattice: &Hnf, rank: usize) -> Option<usize> {
    let counts = histogram(free, lattice);
    let mut order: Vec<usize> = (0..counts.len()).filter(|&c| counts[c] > 0).collect();
    order.sort_by_key(|&c| (Reverse(counts[c]), c));
    order.get(rank).copied()
}

/// The origins of `free` on the coset, rows from the bottom.
fn block_points(
    ctx: &Ctx,
    class: usize,
    side: &Side,
    coset: usize,
    free: &SpanRaster,
) -> Vec<PixelPlacement> {
    let Some(bounds) = ctx.origins(class).bounds() else {
        return Vec::new();
    };
    side.lattice
        .points_in(coset, bounds)
        .into_iter()
        .filter(|&(x, y)| free.contains(x, y))
        .map(|(x, y)| PixelPlacement { class, x, y })
        .collect()
}

/// One layout: the first block's `rank`-th phase, the second block's best coset among the
/// origins the first block leaves, completed from the corners. `None` for an empty block.
fn build(
    ctx: &Ctx,
    sides: &[Side],
    combo: Combo,
    anchor: Option<usize>,
    rank: usize,
    cap: usize,
) -> Option<Candidate> {
    let (first, second) = (&sides[combo.first], &sides[combo.second]);
    let free_first = side_origins(ctx, first.class, anchor, &combo, true);
    let coset_first = nth_coset(&free_first, &first.lattice, rank)?;
    let block_first = block_points(ctx, first.class, first, coset_first, &free_first);
    let mut free_second = side_origins(ctx, second.class, anchor, &combo, false);
    for p in &block_first {
        let no_fit = translate(ctx.no_fit(p.class, second.class), p.x, p.y);
        free_second = free_second.difference(&no_fit);
    }
    let coset_second = nth_coset(&free_second, &second.lattice, 0)?;
    let mut seed = block_first;
    seed.extend(block_points(
        ctx,
        second.class,
        second,
        coset_second,
        &free_second,
    ));
    seed.sort_by_key(|p| (p.y, p.x, p.class));
    if cap > 0 {
        seed.truncate(cap);
    }
    let filled = edge_fill(ctx, anchor, &seed, cap);
    let detail = [
        i64::from(combo.axis == Axis::Y),
        i64::from(combo.cut),
        combo.first as i64,
        combo.second as i64,
        coset_first as i64,
        coset_second as i64,
        filled.corner as i64,
    ];
    Some(ctx.candidate(Member::TwoBlock, anchor, detail.to_vec(), &filled.layout))
}

#[cfg(test)]
mod tests {
    use super::super::single::rank_class;
    use super::super::test_shapes::{assert_legal, triangle, with_ctx};
    use super::*;

    fn ranked(ctx: &Ctx) -> Vec<Vec<Scored>> {
        (0..ctx.classes())
            .map(|class| rank_class(ctx, class, Budget::for_effort(64)))
            .collect()
    }

    #[test]
    fn the_seam_is_legal_against_a_pair_by_pair_check() {
        with_ctx(&[triangle(0), triangle(1)], |ctx| {
            let found = two_block(ctx, &ranked(ctx), &[None], Budget::for_effort(64), 0);
            assert!(!found.is_empty());
            for candidate in &found {
                assert_legal(ctx, &candidate.placements);
            }
        });
    }

    #[test]
    fn a_cut_exists_on_both_axes_at_the_grid() {
        with_ctx(&[triangle(0)], |ctx| {
            let step = (CUT_GRID_MM / ctx.grid().resolution) as i32;
            for axis in [Axis::X, Axis::Y] {
                let cuts = cut_positions(ctx, axis);
                assert!(cuts.len() > 4);
                assert!(cuts.windows(2).all(|w| w[1] - w[0] == step));
            }
        });
    }

    #[test]
    fn the_effort_only_reads_further_down_the_list() {
        with_ctx(&[triangle(0), triangle(1)], |ctx| {
            let ranked = ranked(ctx);
            let best = |effort| {
                two_block(ctx, &ranked, &[None], Budget::for_effort(effort), 0)
                    .iter()
                    .map(|c| c.key.count)
                    .max()
                    .unwrap_or(0)
            };
            let counts: Vec<usize> = [1, 4, 16, 64].into_iter().map(best).collect();
            assert!(counts.windows(2).all(|w| w[0] <= w[1]), "{counts:?}");
        });
    }
}

//! Member B, the double lattice (design 2026-11-02, §9.2): copies of two rotation classes
//! A and B (typically theta and theta + 180) on two cosets of one lattice, which interlock
//! where one class alone cannot (a shape's 180-degree copy fills its concavities).
//!
//! `double_search` finds the lattices, `double_phase` their best coset pairs; this file
//! ranks them, completes the best ones with the edge fill and hands them over as candidates.
//! As in member A the ranking never depends on the effort: a larger effort reads further
//! down fixed ordered lists, so an earlier winner is never lost.

use super::double_phase::{AnchorLosses, Maps, Pair, Scored, best_pairs, rescore};
use super::double_search::{PairSets, seed_lattices};
use super::hnf::Hnf;
use super::residue::TowerOrigins;
use super::single::{Budget, lattice_points};
use crate::layout::PixelPlacement;
use crate::pack::context::Ctx;
use crate::pack::edge_fill::edge_fill;
use crate::pack::select::{Candidate, Member};
use rayon::prelude::*;
use std::cmp::Reverse;

/// Coset pairs per lattice kept before the tower rescoring (a constant, so the effort only
/// decides how many of them are completed).
const PAIR_POOL: usize = 48;
/// Lattices scored per completed lattice (the determinant ordering is only a proxy).
const SCORING_FACTOR: usize = 24;

/// A lattice with its best rescored pairs.
struct Ranked {
    lattice: Hnf,
    pairs: Vec<Scored>,
}

/// Double-lattice candidates of the class pair `(a, b)` (local ids, `a != b`) over every
/// tower anchor of `ctx`: one completed layout per (lattice, pair), in rank order.
pub(crate) fn double_lattice(
    ctx: &Ctx,
    (a, b): (usize, usize),
    budget: Budget,
    cap: usize,
) -> Vec<Candidate> {
    ctx.prepare_no_fit();
    let ranked = ranked_lattices(ctx, (a, b), budget);
    let jobs: Vec<(Hnf, Scored)> = ranked
        .into_iter()
        .flat_map(|r| {
            r.pairs
                .into_iter()
                .take(budget.phases)
                .map(move |s| (r.lattice, s))
        })
        .collect();
    jobs.par_iter()
        .map(|(lattice, scored)| complete(ctx, (a, b), lattice, scored, cap))
        .collect()
}

/// The best `budget.lattices` lattices by their best pair score (ties: determinant order).
fn ranked_lattices(ctx: &Ctx, (a, b): (usize, usize), budget: Budget) -> Vec<Ranked> {
    let sets = PairSets {
        aa: ctx.no_fit(a, a),
        bb: ctx.no_fit(b, b),
        ab: ctx.no_fit(a, b),
    };
    let seeds = seed_lattices(&sets, budget.lattices * SCORING_FACTOR);
    let maps = Maps {
        base_a: &ctx.origins(a).allowed,
        base_b: &ctx.origins(b).allowed,
        ab: ctx.no_fit(a, b),
        ba: ctx.no_fit(b, a),
    };
    let towers = AnchorLosses {
        a: anchor_origins(ctx, a),
        b: anchor_origins(ctx, b),
    };
    let mut ranked: Vec<(usize, Ranked)> = seeds
        .par_iter()
        .enumerate()
        .map(|(index, lattice)| {
            let pairs = best_pairs(&maps, lattice, PAIR_POOL);
            let pairs = rescore(lattice, &pairs, &towers);
            let ranked = Ranked {
                lattice: *lattice,
                pairs,
            };
            (index, ranked)
        })
        .filter(|(_, r)| !r.pairs.is_empty())
        .collect();
    ranked.sort_by_key(|(index, r)| (Reverse(r.pairs[0].score), *index));
    ranked.truncate(budget.lattices);
    ranked.into_iter().map(|(_, r)| r).collect()
}

fn anchor_origins<'c>(ctx: &'c Ctx, class: usize) -> Vec<TowerOrigins<'c>> {
    ctx.towers()
        .iter()
        .map(|tower| TowerOrigins::new(tower.forbidden(class)))
        .collect()
}

/// The layout of one coset pair at its anchor, completed from the corners.
fn complete(
    ctx: &Ctx,
    (a, b): (usize, usize),
    lattice: &Hnf,
    scored: &Scored,
    cap: usize,
) -> Candidate {
    let Pair { a: ca, b: cb, .. } = scored.pair;
    let mut seed = lattice_points(ctx, a, lattice, ca, scored.anchor, 0);
    seed.extend(lattice_points(ctx, b, lattice, cb, scored.anchor, 0));
    seed.sort_by_key(|p: &PixelPlacement| (p.y, p.x, p.class));
    if cap > 0 {
        seed.truncate(cap);
    }
    let filled = edge_fill(ctx, scored.anchor, &seed, cap);
    let detail = [
        a as i32,
        b as i32,
        lattice.s,
        lattice.t,
        lattice.h,
        ca as i32,
        cb as i32,
        filled.corner as i32,
    ];
    ctx.candidate(
        Member::DoubleLattice,
        scored.anchor,
        detail.iter().map(|&v| i64::from(v)).collect(),
        &filled.layout,
    )
}

#[cfg(test)]
mod tests {
    use super::super::test_shapes::{assert_legal, triangle, with_ctx};
    use super::*;

    fn candidates(effort: usize) -> Vec<(usize, Vec<PixelPlacement>)> {
        with_ctx(&[triangle(0), triangle(1)], |ctx| {
            let found = double_lattice(ctx, (0, 1), Budget::for_effort(effort), 0);
            found
                .into_iter()
                .map(|c| (c.key.count, c.placements))
                .collect()
        })
    }

    #[test]
    fn every_candidate_is_legal_pair_by_pair() {
        with_ctx(&[triangle(0), triangle(1)], |ctx| {
            let found = double_lattice(ctx, (0, 1), Budget::for_effort(64), 0);
            assert!(!found.is_empty());
            for candidate in &found {
                assert_legal(ctx, &candidate.placements);
            }
        });
    }

    #[test]
    fn interlocking_halves_beat_the_single_class_lattice() {
        let double = candidates(64).iter().map(|c| c.0).max().unwrap();
        let single = with_ctx(&[triangle(0)], |ctx| {
            let budget = Budget::for_effort(64);
            let ranked = super::super::single::rank_class(ctx, 0, budget);
            super::super::single::complete_ranked(ctx, 0, &ranked, budget, 0)
                .iter()
                .map(|c| c.key.count)
                .max()
                .unwrap()
        });
        assert!(
            double * 10 >= single * 13,
            "double {double} against single {single}"
        );
    }

    #[test]
    fn more_effort_never_loses_the_best_count() {
        let best = |effort| candidates(effort).iter().map(|c| c.0).max().unwrap();
        let counts: Vec<usize> = [1, 4, 16, 64].into_iter().map(best).collect();
        assert!(counts.windows(2).all(|w| w[0] <= w[1]), "{counts:?}");
    }
}

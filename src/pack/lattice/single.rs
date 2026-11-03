//! Member A, the single lattice (design 2026-11-02, §9.2): every admissible lattice of one
//! rotation class with its exact best phases under the best tower anchor, the top lattices
//! and their top phases completed by the edge fill.
//!
//! The ranking never depends on the effort: lattices are ordered by their best score (then
//! enumeration order), a lattice's phases by score (then the histogram order), and a larger
//! effort only reads further down both lists. A candidate found at a low effort is therefore
//! among the candidates of every higher one, and the best candidate never gets worse.

use super::enumerate::enumerate;
use super::hnf::Hnf;
use super::residue::{Phase, TowerOrigins, best_phases, histogram};
use crate::layout::PixelPlacement;
use crate::pack::context::Ctx;
use crate::pack::edge_fill::edge_fill;
use crate::pack::select::{Candidate, Member};
use rayon::prelude::*;
use std::cmp::Reverse;

/// Lattices completed at full effort (design K).
pub const TOP_LATTICES: usize = 24;
/// Phases per lattice completed at full effort (design R).
pub const TOP_PHASES: usize = 6;
/// The effort at which the budget is full.
pub const FULL_EFFORT: usize = 64;
/// Tower anchors of the best candidates handed to the members that cannot scan them all.
pub const SHORTLIST_ANCHORS: usize = 3;

/// How far a run reads down the two ranked lists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Budget {
    pub lattices: usize,
    pub phases: usize,
}

impl Budget {
    /// Grows with `effort` (1 = one phase of the best lattice, `FULL_EFFORT` and above = the
    /// design's K x R); never shrinks as effort grows.
    pub fn for_effort(effort: usize) -> Budget {
        let effort = effort.clamp(1, FULL_EFFORT);
        Budget {
            lattices: (TOP_LATTICES * effort).div_ceil(FULL_EFFORT),
            phases: (TOP_PHASES * effort).div_ceil(FULL_EFFORT),
        }
    }
}

/// A lattice with its best phases.
struct Scored {
    lattice: Hnf,
    phases: Vec<Phase>,
}

/// Single-lattice candidates of `class` (local id) over every tower anchor of `ctx`: one
/// completed layout per (lattice, phase), in rank order.
pub(crate) fn single_lattice(
    ctx: &Ctx,
    class: usize,
    budget: Budget,
    cap: usize,
) -> Vec<Candidate> {
    ctx.prepare_no_fit();
    let scored = ranked_lattices(ctx, class, budget);
    let triples: Vec<(Hnf, Phase)> = scored
        .into_iter()
        .take(budget.lattices)
        .flat_map(|s| s.phases.into_iter().map(move |p| (s.lattice, p)))
        .collect();
    triples
        .par_iter()
        .map(|(lattice, phase)| complete(ctx, class, lattice, phase, cap))
        .collect()
}

/// Lattices scored per parallel step. A constant, not the thread count, so the set of
/// lattices scored never depends on the machine.
const SCORING_CHUNK: usize = 16;

/// The best `budget.lattices` lattices with their `budget.phases` best phases, best lattice
/// first (ties: enumeration order).
///
/// Two passes keep the tower rescoring cheap. First every lattice gets its best score alone
/// (the scan stops at the first coset that cannot be beaten); a lattice scores at most its
/// plain best count, so lattices are taken in falling order of that count and the pass stops
/// once the next one cannot reach the current top. Then only the top lattices pay for their
/// full phase list. The histograms fold in parallel and are collected in enumeration order,
/// so the ranking is the sequential one for any thread count.
fn ranked_lattices(ctx: &Ctx, class: usize, budget: Budget) -> Vec<Scored> {
    let lattices = enumerate(ctx.no_fit(class, class));
    let base = &ctx.origins(class).allowed;
    let towers: Vec<TowerOrigins> = ctx
        .towers()
        .iter()
        .map(|tower| TowerOrigins::new(tower.forbidden(class)))
        .collect();
    let phases_of = |index: usize, limit: usize| {
        let counts = histogram(base, &lattices[index]);
        best_phases(&lattices[index], &counts, &towers, limit)
    };
    let plain_best: Vec<u32> = lattices
        .par_iter()
        .map(|lattice| histogram(base, lattice).into_iter().max().unwrap_or(0))
        .collect();
    let mut order: Vec<usize> = (0..lattices.len()).filter(|&i| plain_best[i] > 0).collect();
    order.sort_by_key(|&i| (Reverse(plain_best[i]), i));
    let mut scores: Vec<(usize, u32)> = Vec::new();
    for chunk in order.chunks(SCORING_CHUNK) {
        if kth_score(&scores, budget.lattices).is_some_and(|kth| plain_best[chunk[0]] < kth) {
            break;
        }
        let batch: Vec<(usize, u32)> = chunk
            .par_iter()
            .filter_map(|&index| Some((index, phases_of(index, 1).first()?.score)))
            .collect();
        scores.extend(batch);
    }
    scores.sort_by_key(|&(index, score)| (Reverse(score), index));
    scores.truncate(budget.lattices);
    scores
        .par_iter()
        .map(|&(index, _)| Scored {
            lattice: lattices[index],
            phases: phases_of(index, budget.phases),
        })
        .collect()
}

/// The score of the `k`-th best lattice so far.
fn kth_score(scores: &[(usize, u32)], k: usize) -> Option<u32> {
    let mut sorted: Vec<u32> = scores.iter().map(|&(_, score)| score).collect();
    sorted.sort_unstable_by_key(|&score| Reverse(score));
    sorted.get(k.checked_sub(1)?).copied()
}

/// The layout of one lattice phase at its anchor, completed from the corners.
fn complete(ctx: &Ctx, class: usize, lattice: &Hnf, phase: &Phase, cap: usize) -> Candidate {
    let seed = lattice_points(ctx, class, lattice, phase, cap);
    let filled = edge_fill(ctx, phase.anchor, &seed, cap);
    let detail = [
        class as i32,
        lattice.s,
        lattice.t,
        lattice.h,
        phase.coset as i32,
        filled.corner as i32,
    ];
    ctx.candidate(
        Member::Lattice,
        phase.anchor,
        detail.iter().map(|&v| i64::from(v)).collect(),
        &filled.layout,
    )
}

/// The phase's points the class may take (base map minus the tower), rows from the bottom,
/// at most `cap` of them when set.
fn lattice_points(
    ctx: &Ctx,
    class: usize,
    lattice: &Hnf,
    phase: &Phase,
    cap: usize,
) -> Vec<PixelPlacement> {
    let Some(bounds) = ctx.origins(class).bounds() else {
        return Vec::new();
    };
    let tower = phase.anchor.map(|a| ctx.towers()[a].forbidden(class));
    let free = lattice
        .points_in(phase.coset, bounds)
        .into_iter()
        .filter(|&(x, y)| ctx.origins(class).contains(x, y))
        .filter(|&(x, y)| tower.is_none_or(|set| !set.contains(x, y)))
        .map(|(x, y)| PixelPlacement { class, x, y });
    if cap == 0 {
        free.collect()
    } else {
        free.take(cap).collect()
    }
}

/// The anchors of the best `SHORTLIST_ANCHORS` candidates, best first, without repeats: the
/// tower positions worth running the members that cannot scan every anchor at.
pub(crate) fn best_anchors(candidates: &[Candidate]) -> Vec<usize> {
    let mut ranked: Vec<&Candidate> = candidates.iter().collect();
    ranked.sort_by(|a, b| a.key.cmp(&b.key));
    let mut anchors: Vec<usize> = Vec::new();
    for anchor in ranked.iter().filter_map(|c| c.anchor) {
        if !anchors.contains(&anchor) && anchors.len() < SHORTLIST_ANCHORS {
            anchors.push(anchor);
        }
    }
    anchors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_only_grows_with_effort() {
        let budgets: Vec<Budget> = (0..=80).map(Budget::for_effort).collect();
        assert!(
            budgets
                .windows(2)
                .all(|w| { w[0].lattices <= w[1].lattices && w[0].phases <= w[1].phases })
        );
        assert_eq!(
            budgets[0],
            Budget {
                lattices: 1,
                phases: 1
            }
        );
        assert_eq!(
            budgets[64],
            Budget {
                lattices: TOP_LATTICES,
                phases: TOP_PHASES
            }
        );
        assert_eq!(budgets[80], budgets[64]);
    }
}

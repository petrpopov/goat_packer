//! The members' candidates of a fixed-class run (design 2026-11-02, section 9): which member
//! runs at which tower anchor and class, and how their work is split over threads.

use super::context::Ctx;
use super::lattice::{double, single, two_block};
use super::select::{Candidate, Member};
use super::{rows, sweep};
use crate::options::PackOptions;
use rayon::prelude::*;

/// Tower anchors the greedy members (sweeps, rows) run at when no lattice ranked them: the
/// source and the corners of the range, which `scan_anchors` lists first.
const GREEDY_ANCHORS: usize = 5;

/// One unit of greedy member work: a member at one tower anchor, for one class where the
/// member runs per class.
type Unit = (Member, Option<usize>, Option<usize>);

/// Every member's candidates. The lattice runs first, per class, over every scanned anchor;
/// the greedy members cost a full run per anchor, so they run at the source anchor and the
/// anchors of the best lattice candidates.
pub(super) fn member_candidates(
    ctx: &Ctx,
    members: &[Member],
    options: &PackOptions,
) -> Vec<Candidate> {
    let effort = options.attempts.clamp(1, sweep::MAX_SWEEP_VARIANTS);
    let cap = options.max_objects;
    let mut candidates: Vec<Candidate> = Vec::new();
    let budget = single::Budget::for_effort(effort);
    // Member A's ranking also feeds the two-block member, so it runs once for both.
    let ranked: Vec<Vec<single::Scored>> =
        if members.contains(&Member::Lattice) || members.contains(&Member::TwoBlock) {
            (0..ctx.classes())
                .map(|class| single::rank_class(ctx, class, budget))
                .collect()
        } else {
            Vec::new()
        };
    if members.contains(&Member::Lattice) {
        for (class, scored) in ranked.iter().enumerate() {
            candidates.extend(single::complete_ranked(ctx, class, scored, budget, cap));
        }
    }
    if members.contains(&Member::DoubleLattice) {
        for a in 0..ctx.classes() {
            for b in a + 1..ctx.classes() {
                candidates.extend(double::double_lattice(ctx, (a, b), budget, cap));
            }
        }
    }
    let slots = greedy_slots(ctx, &single::best_anchors(&candidates));
    if members.contains(&Member::TwoBlock) {
        candidates.extend(two_block::two_block(ctx, &ranked, &slots, budget, cap));
    }
    candidates.extend(greedy_candidates(ctx, members, &slots, effort, cap));
    candidates
}

/// The anchor slots of the greedy members: the source (slot 0), the lattice's best anchors,
/// and without a lattice result the corners too; ascending, no repeats.
fn greedy_slots(ctx: &Ctx, ranked: &[usize]) -> Vec<Option<usize>> {
    let slots = ctx.anchor_slots();
    if slots[0].is_none() {
        return slots;
    }
    let mut chosen: Vec<usize> = vec![0];
    if ranked.is_empty() {
        chosen.extend(1..GREEDY_ANCHORS.min(slots.len()));
    }
    chosen.extend_from_slice(ranked);
    chosen.sort_unstable();
    chosen.dedup();
    chosen.into_iter().map(Some).collect()
}

fn greedy_candidates(
    ctx: &Ctx,
    members: &[Member],
    slots: &[Option<usize>],
    variants: usize,
    cap: usize,
) -> Vec<Candidate> {
    let mut units: Vec<Unit> = Vec::new();
    for &member in members {
        for &anchor in slots {
            match member {
                Member::Rows => units.extend((0..ctx.classes()).map(|c| (member, anchor, Some(c)))),
                Member::Sweep => units.push((member, anchor, None)),
                _ => {}
            }
        }
    }
    units
        .par_iter()
        .map(|&(member, anchor, class)| match (member, class) {
            (Member::Rows, Some(class)) => {
                let (layout, shift) = rows::brick_rows(ctx, class, anchor, cap);
                ctx.candidate(
                    member,
                    anchor,
                    vec![class as i64, i64::from(shift)],
                    &layout,
                )
            }
            _ => {
                let run = sweep::sweeps(ctx, anchor, variants, cap);
                ctx.candidate(member, anchor, vec![run.variant as i64], &run.layout)
            }
        })
        .collect()
}

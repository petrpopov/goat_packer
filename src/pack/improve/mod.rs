//! Member E: improvement of the best layouts (design 2026-11-02, §9.2, work package P6).
//! A lattice clipped to the plate leaves slack at the edges and around the tower that no
//! sweep recovers. For each top layout the pass tries to insert one more copy where the
//! overlap is least, then moves the copies around it (guided local search on penetration
//! depth, `separate`) until no pair overlaps; every success is searched again for the next
//! copy, and a stalled layout is pulled into each corner and searched again (`grow`).
//!
//! Deterministic: insertion origins, tries and moves follow fixed orders and there are no random
//! numbers; the budget counts insertion origins per round, so more effort extends the sequence
//! of tries and never loses an earlier result. Layouts are independent units; rayon reduces them in input order.

mod compact;
mod grow;
mod insert;
mod penetration;
mod polish;
mod separate;
#[cfg(test)]
mod tests;
mod windows;

use super::MemberCount;
use super::context::Ctx;
use super::select::{Candidate, Member};
use crate::layout::PixelPlacement;
use penetration::DepthTable;
use rayon::prelude::*;
use separate::Field;

pub(super) use polish::polish_candidates;

/// Layouts improved per run.
const TOP_LAYOUTS: usize = 20;
/// A layout this many copies below the best count is not worth improving.
const SOURCE_SLACK: usize = 3;
/// Insertion origins tried per round at full effort (`FULL_EFFORT` and above); one at effort 1.
const FULL_SPOTS: usize = 8;
const FULL_EFFORT: usize = 64;

/// Insertion origins per round for `effort`: grows with it, never shrinks. The origins are
/// the cheapest first, so a higher effort tries the same ones and then more.
fn spots_for(effort: usize) -> usize {
    (FULL_SPOTS * effort.clamp(1, FULL_EFFORT)).div_ceil(FULL_EFFORT)
}

/// The improved layouts of the best of `candidates`, as candidates of member `Improve`
/// (`detail` = the source member, the source's count, then the source's own detail). Only
/// layouts with more copies than their source are returned.
pub(super) fn improve_candidates(
    ctx: &Ctx,
    candidates: &[Candidate],
    effort: usize,
    cap: usize,
) -> Vec<Candidate> {
    let sources = top_sources(candidates);
    if sources.is_empty() || ctx.classes() == 0 {
        return Vec::new();
    }
    let depth = DepthTable::new(ctx);
    let spots = spots_for(effort);
    sources
        .par_iter()
        .filter_map(|source| {
            let free: Vec<_> = (0..ctx.classes())
                .map(|class| ctx.free_map(class, source.anchor))
                .collect();
            let field = Field {
                depth: &depth,
                free: &free,
            };
            let layout = local_layout(ctx, source);
            let grown = grow::grow(ctx, &field, &layout, spots, cap);
            if grown.len() <= layout.len() {
                return None;
            }
            let mut detail = vec![source.key.tie.member as i64, layout.len() as i64];
            detail.extend_from_slice(&source.key.tie.detail);
            Some(ctx.candidate(Member::Improve, source.anchor, detail, &grown))
        })
        .collect()
}

/// Per source member, the best count its improved layouts reached.
pub(super) fn improved_counts(candidates: &[Candidate]) -> Vec<MemberCount> {
    let mut counts: Vec<MemberCount> = Vec::new();
    for c in candidates
        .iter()
        .filter(|c| c.key.tie.member == Member::Improve)
    {
        let (source, from) = (c.key.tie.detail[0], c.key.tie.detail[1]);
        if (c.key.count as i64) <= from {
            continue;
        }
        let Some(member) = Member::AVAILABLE
            .iter()
            .copied()
            .find(|&m| m as i64 == source)
        else {
            continue;
        };
        match counts.iter_mut().find(|m| m.member == member) {
            Some(known) => known.count = known.count.max(c.key.count),
            None => counts.push(MemberCount {
                member,
                count: c.key.count,
            }),
        }
    }
    counts.sort_by_key(|m| m.member);
    counts
}

/// The layouts to improve: the best of each member in turn (round robin over the members'
/// own rankings), so one member's near-identical phases do not crowd the others out. A
/// layout further than `SOURCE_SLACK` copies below the best count never gets there, and
/// no more than `TOP_LAYOUTS` are taken.
fn top_sources(candidates: &[Candidate]) -> Vec<&Candidate> {
    let mut order: Vec<&Candidate> = candidates.iter().collect();
    order.sort_by(|a, b| a.key.cmp(&b.key));
    let Some(best) = order.first().map(|c| c.key.count) else {
        return Vec::new();
    };
    let mut members: Vec<Member> = Vec::new();
    for c in &order {
        if !members.contains(&c.key.tie.member) {
            members.push(c.key.tie.member);
        }
    }
    let mut ranked: Vec<std::vec::IntoIter<&Candidate>> = members
        .iter()
        .map(|&m| {
            let own: Vec<&Candidate> = order
                .iter()
                .copied()
                .filter(|c| c.key.tie.member == m && c.key.count + SOURCE_SLACK >= best)
                .collect();
            own.into_iter()
        })
        .collect();
    let mut taken: Vec<&Candidate> = Vec::new();
    while taken.len() < TOP_LAYOUTS {
        let before = taken.len();
        for list in &mut ranked {
            taken.extend(list.next());
        }
        if taken.len() == before {
            break;
        }
    }
    taken.truncate(TOP_LAYOUTS);
    taken
}

/// A candidate's copies in the context's local class numbering. A fixed-class run numbers
/// its classes 0..n, so the run ids are the local ones.
fn local_layout(ctx: &Ctx, candidate: &Candidate) -> Vec<PixelPlacement> {
    debug_assert!(candidate.placements.iter().all(|p| p.class < ctx.classes()));
    candidate.placements.clone()
}

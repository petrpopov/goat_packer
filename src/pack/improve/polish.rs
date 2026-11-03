//! Spare room spent on safety (work package P6, robustness): among layouts with the same
//! count the one whose pairs keep the most room wins (`robustness`), but the members pack
//! pairs touching. The best layouts are pushed apart here: the no-fit sets and the tower's
//! origin sets are grown by `m` pixels and the same guided search (`separate`) is run with
//! every copy mobile; when it resolves, every pair keeps `m` pixels of slack. `m` grows one
//! pixel at a time until the search fails or the cap of `robustness` is reached.

use super::grow::is_legal;
use super::penetration::DepthTable;
use super::separate::{Field, separate};
use crate::kind_masks::OriginSet;
use crate::layout::PixelPlacement;
use crate::pack::context::Ctx;
use crate::pack::free_map::FreeMap;
use crate::pack::robustness::{cap_px, layout_slack, quarter_mm};
use crate::pack::select::{Candidate, Member};
use crate::span_raster::{SpanRaster, merge_spans};
use rayon::prelude::*;

/// Layouts polished per run: the best by rank key.
const POLISH_LAYOUTS: usize = 3;
/// Iterations of one separation; every copy is mobile, so a margin step may take many.
const POLISH_ITERATIONS: usize = 300;

/// The best candidates by rank key pushed apart as far as `separate` manages, as candidates of
/// member `Improve` whose `detail` starts with the source member, the count and `-1` (the
/// count is the source's). Only layouts with more slack than their source are returned.
pub(in crate::pack) fn polish_candidates(ctx: &Ctx, candidates: &[Candidate]) -> Vec<Candidate> {
    let mut order: Vec<&Candidate> = candidates.iter().collect();
    order.sort_by(|a, b| a.key.cmp(&b.key));
    order.truncate(POLISH_LAYOUTS);
    let resolution = ctx.grid().resolution;
    order
        .par_iter()
        .filter_map(|source| {
            let start = layout_slack(ctx, source.anchor, &source.placements);
            let pushed = polish(ctx, source.anchor, &source.placements, start);
            let slack = layout_slack(ctx, source.anchor, &pushed);
            if slack <= start {
                return None;
            }
            let mut detail = vec![source.key.tie.member as i64, source.key.count as i64, -1];
            detail.extend_from_slice(&source.key.tie.detail);
            let mut candidate = ctx.candidate(Member::Improve, source.anchor, detail, &pushed);
            candidate.key.robustness_quarter_mm = quarter_mm(slack, resolution);
            Some(candidate)
        })
        .collect()
}

/// `layout` with the most slack the search reaches, starting above `start` pixels.
fn polish(
    ctx: &Ctx,
    anchor: Option<usize>,
    layout: &[PixelPlacement],
    start: i32,
) -> Vec<PixelPlacement> {
    let mut best = layout.to_vec();
    for margin in start + 1..=cap_px(ctx.grid().resolution) {
        let depth = DepthTable::of_sets(ctx.classes(), |a, b| grow_set(ctx.no_fit(a, b), margin));
        let free = free_maps(ctx, anchor, margin);
        let field = Field {
            depth: &depth,
            free: &free,
        };
        let mut trial = best.clone();
        let mobile: Vec<usize> = (0..trial.len()).collect();
        let resolved = separate(&field, &mut trial, &mobile, POLISH_ITERATIONS);
        if !resolved || !is_legal(ctx, &field, &trial) {
            break;
        }
        best = trial;
    }
    best
}

/// The free origins of every class with the tower's set grown by `margin`.
fn free_maps(ctx: &Ctx, anchor: Option<usize>, margin: i32) -> Vec<FreeMap> {
    (0..ctx.classes())
        .map(|class| {
            let mut free = ctx.free_map(class, anchor);
            if let Some(t) = anchor {
                let set = &ctx.towers()[t].forbidden(class).spans;
                free.remove(&OriginSet {
                    spans: grow_set(set, margin),
                });
            }
            free
        })
        .collect()
}

/// `set` grown by `margin` pixels in the max norm.
fn grow_set(set: &SpanRaster, margin: i32) -> SpanRaster {
    if set.rows.is_empty() {
        return SpanRaster::default();
    }
    let rows = (0..set.rows.len() as i32 + 2 * margin)
        .map(|index| {
            let y = set.min_y - margin + index;
            let mut spans: Vec<(i32, i32)> = (y - margin..=y + margin)
                .flat_map(|row| set.row(row))
                .map(|&(a, b)| (a - margin, b + margin))
                .collect();
            merge_spans(&mut spans);
            spans
        })
        .collect();
    SpanRaster {
        min_y: set.min_y - margin,
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growing_a_set_adds_a_ring_of_the_margin() {
        let grown = grow_set(&SpanRaster::rect(0, 0, 3, 1), 2);
        assert_eq!(grown, SpanRaster::rect(-2, -2, 5, 3));
        assert_eq!(grow_set(&SpanRaster::default(), 3), SpanRaster::default());
    }
}

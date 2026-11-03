//! The `Exact` member inside a level (`refine_exact`): refines the best lattice candidates
//! after the raster members, then replaces the raster winner when a refined layout has more
//! copies and passes the vector check. Layouts of this member are in millimetres, so they
//! bypass the pixel shortlist and the raster check; the vector check is the gate.

use super::refine_exact::{self, ExactLayout};
use super::select::{Candidate, CheckFailure, Member, RankKey, TieKey};
use super::sweep::MAX_SWEEP_VARIANTS;
use super::{Checks, Ctx, PackOutcome};
use crate::layout::PixelPlacement;
use crate::options::PackOptions;
use crate::validation::VectorValidator;

/// The refined layouts of a level, most copies first, with the validators that gate them.
pub(super) struct ExactRun {
    layouts: Vec<ExactLayout>,
    validators: Vec<(Option<usize>, VectorValidator)>,
}

/// Runs the member when requested. It does not run with `support_overlap_mm`: that rule is
/// raster-only, so the vector gate cannot vouch for a layout built to the exact contact.
pub(super) fn run(
    ctx: &Ctx,
    checks: &Checks,
    candidates: &[Candidate],
    members: &[Member],
    options: &PackOptions,
) -> Result<ExactRun, String> {
    let mut run = ExactRun {
        layouts: Vec::new(),
        validators: Vec::new(),
    };
    if !members.contains(&Member::Exact) || options.support_overlap_mm.is_some() {
        return Ok(run);
    }
    let effort = options.attempts.clamp(1, MAX_SWEEP_VARIANTS);
    let seeds = refine_exact::seeds(candidates, ctx.grid(), effort);
    for anchor in refine_exact::anchors(&seeds) {
        run.validators.push((anchor, checks.vector(anchor)?));
    }
    run.layouts = refine_exact::refine(ctx, &seeds, &run.validators, options.max_objects);
    run.layouts
        .sort_by_key(|l| std::cmp::Reverse(l.placements.len()));
    Ok(run)
}

/// Swaps in the first refined layout that beats `outcome` and passes the vector check;
/// failures go to the report.
pub(super) fn publish(run: ExactRun, ctx: &Ctx, outcome: &mut PackOutcome) {
    let best = run.layouts.first().map(|l| l.placements.len());
    outcome
        .member_counts
        .extend(best.map(|count| super::MemberCount {
            member: Member::Exact,
            count,
        }));
    for layout in &run.layouts {
        let count = layout.placements.len();
        if count <= outcome.placements.len() {
            break;
        }
        let Some((_, validator)) = run.validators.iter().find(|(a, _)| *a == layout.anchor) else {
            continue;
        };
        if let Err(message) = validator.check(&layout.placements) {
            outcome.validation.vector_failures.push(CheckFailure {
                rank: outcome.validation.shortlisted,
                member: Member::Exact,
                count,
                message,
            });
            continue;
        }
        outcome.key = key_of(ctx, layout);
        outcome.member = Member::Exact;
        outcome.placements = layout.placements.clone();
        outcome.tower_anchor = layout.anchor.map(|a| ctx.towers()[a].anchor);
        outcome.validation.winner_rank = outcome.validation.shortlisted;
        return;
    }
}

/// The rank key of a refined layout: its count, no robustness or canonical claim, and the
/// box of the layout rounded to the pixel grid (only compared across ladder levels).
fn key_of(ctx: &Ctx, layout: &ExactLayout) -> RankKey {
    let grid = ctx.grid();
    let pixel = |mm: f64, origin: f64| ((mm - origin) / grid.resolution).round() as i32;
    let rounded: Vec<PixelPlacement> = layout
        .placements
        .iter()
        .map(|p| PixelPlacement {
            class: p.print,
            x: pixel(p.x_mm, grid.origin.x),
            y: pixel(p.y_mm, grid.origin.y),
        })
        .collect();
    RankKey {
        count: layout.placements.len(),
        robustness_quarter_mm: 0,
        canonical_clear: true,
        bbox_area_px: ctx.bbox_area(&rounded),
        tie: TieKey {
            member: Member::Exact,
            anchor: layout.anchor.map_or(0, |a| a as u32),
            detail: Vec::new(),
        },
    }
}

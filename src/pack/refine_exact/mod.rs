//! Exact lattice refinement (member `Exact`, research 2026-11-03): the top lattice candidates
//! of the raster members re-solved with continuous parameters in vector geometry.
//!
//! Why: a raster lattice lives on whole pixels with conservative cover, which costs up to
//! ~0.5 mm per contact at 0.25 mm (growing the bed by 0.25-0.5 mm per side gained 1-4 copies
//! on the locked goat plates). Here the basis `u`, `v`, the phase and, for a double lattice,
//! the offset `w` are real numbers, the pair rules are the exact predicates of
//! `VectorValidator` (no raster, no no-fit map) and the zones are its exact zone tests, tower
//! included. Admissibility of a lattice is a finite list of pair shifts (`lattice`).
//!
//! Per seed (`search`): a pattern search over those parameters that keeps every contact
//! clear and improves (usable sites, pull of the sites just outside the plate, smaller cell).
//! Minimising the cell alone is wrong: the densest lattice of U1 holds 53 copies against the
//! raster lattice's 58, because it stops fitting the plate edges. Then the copies the raster
//! fill added around the lattice are re-seated (`fill`). A seed's result is kept only with
//! at least the seed's copies. Nothing is random or timed: seeds and searches are fixed
//! ordered lists with fixed budgets, so the result is the same for any thread count and a
//! larger effort only adds seeds.
//!
//! Measured (goat bench, 0.25 mm, same rules): the exact lattices reach what the finer
//! ladder levels reach (U1 58 -> 59, P1S 47, A2L double 81) and lift weaker raster lattices
//! by 1-2 (A2L 75 -> 77), but `Improve` beats them in the full pipeline (U1 56, P1S 47, A2L
//! 79 either way): their layouts are not raster-legal, so `Improve` cannot start from them
//! (a rounded exact layout does not separate: the raster at 0.25 mm has no legal layout near
//! it), and compacting them or an `Improve` layout in exact geometry never freed a whole
//! copy (the slack is ~3% of the cell, spread over every contact).
//!
//! The layouts are in millimetres, not on the pixel grid, so they skip the raster check
//! (conservative cover would reject a layout built to sit at the exact contact); the vector
//! check, the publication gate, decides. The raster-only `support_overlap_mm` rule is not
//! checked by the vector check, so the member does not run when it is set.

mod fill;
mod lattice;
mod oracle;
mod search;
mod seed;
mod sites;
#[cfg(test)]
mod tests;

use crate::geometry::{Point, Rect};
use crate::kind_masks::OriginGrid;
use crate::pack::context::Ctx;
use crate::pack::select::{Candidate, Member};
use crate::validation::{PrintPlacement, VectorValidator};
use lattice::{Contacts, Params};
use oracle::Oracle;
use rayon::prelude::*;
use seed::Seed;
use sites::Field;

/// Most seeds refined, however large the effort.
const MAX_SEEDS: usize = 8;
/// The effort at which the seeds are full.
const MAX_EFFORT: usize = 63;
/// Largest uniform scale-up tried to make a seed exactly admissible, as a fraction.
const REPAIR_GROWTH: f64 = 0.02;

/// A refined layout in plate millimetres.
#[derive(Clone, Debug)]
pub(crate) struct ExactLayout {
    /// Index into the run's tower anchors; `None` without a tower.
    pub(crate) anchor: Option<usize>,
    pub(crate) placements: Vec<PrintPlacement>,
}

/// Seeds refined at `effort`: the best distinct lattices among `candidates` by rank key.
pub(crate) fn seeds(candidates: &[Candidate], grid: &OriginGrid, effort: usize) -> Vec<Seed> {
    let wanted = 1 + effort.saturating_sub(1) * (MAX_SEEDS - 1) / MAX_EFFORT;
    let mut ranked: Vec<&Candidate> = candidates.iter().collect();
    ranked.sort_by(|a, b| a.key.cmp(&b.key));
    let mut found: Vec<Seed> = Vec::new();
    let mut seen: Vec<(Option<usize>, Vec<i64>)> = Vec::new();
    for candidate in ranked {
        if found.len() >= wanted {
            break;
        }
        let detail = &candidate.key.tie.detail;
        // The lattice and classes, not the phase or the fill corner.
        let keep = if candidate.key.tie.member == Member::Lattice {
            4
        } else {
            5
        };
        let lattice = detail[..keep.min(detail.len())].to_vec();
        let id = (candidate.anchor, lattice);
        if seen.contains(&id) {
            continue;
        }
        if let Some(seed) = Seed::decode(candidate, grid) {
            seen.push(id);
            found.push(seed);
        }
    }
    found
}

/// The anchors the seeds need a validator for, ascending and without repeats.
pub(crate) fn anchors(seeds: &[Seed]) -> Vec<Option<usize>> {
    let mut anchors: Vec<Option<usize>> = seeds.iter().map(|s| s.anchor).collect();
    anchors.sort_unstable();
    anchors.dedup();
    anchors
}

/// Every seed refined, in seed order; seeds whose refinement is not at least as large as the
/// seed are left out. `validators` holds one validator per anchor of `seeds`.
pub(crate) fn refine(
    ctx: &Ctx,
    seeds: &[Seed],
    validators: &[(Option<usize>, VectorValidator)],
    cap: usize,
) -> Vec<ExactLayout> {
    seeds
        .par_iter()
        .filter_map(|seed| {
            let (_, validator) = validators
                .iter()
                .find(|(anchor, _)| *anchor == seed.anchor)?;
            let oracle = Oracle::new(validator, ctx.classes());
            refine_seed(ctx, &oracle, seed, cap)
        })
        .collect()
}

fn refine_seed(ctx: &Ctx, oracle: &Oracle, seed: &Seed, cap: usize) -> Option<ExactLayout> {
    let regions: Vec<Rect> = seed
        .classes
        .iter()
        .map(|&class| origin_region(ctx, class).map(|r| tighten(oracle, class, r)))
        .collect::<Option<_>>()?;
    let mut contacts = Contacts::new(&seed.classes);
    let start = admissible_start(oracle, &mut contacts, seed)?;
    let field = Field {
        oracle,
        classes: &seed.classes,
        regions: &regions,
    };
    let (found, _) = search::search(&field, &mut contacts, start, seed.dof());
    // The seed's own lattice is always a contender: the result then never has fewer sites
    // than the raster lattice had.
    let mut best: Option<Vec<(usize, Point)>> = None;
    for x in [found, start] {
        let mut sites = field.sites(&x).usable;
        if cap > 0 {
            sites.truncate(cap);
        }
        let placed = fill::seat_extras(oracle, &seed.classes, &x, &sites, &seed.extras, cap);
        if best.as_ref().is_none_or(|b| placed.len() > b.len()) {
            best = Some(placed);
        }
    }
    let placed = best?;
    (placed.len() >= seed.count).then(|| ExactLayout {
        anchor: seed.anchor,
        placements: to_placements(&placed),
    })
}

/// The seed's parameters, grown uniformly if the exact oracle finds the raster lattice a hair
/// too tight (the raster is conservative, so this is rare); `None` when no growth under
/// `REPAIR_GROWTH` makes it admissible.
fn admissible_start(oracle: &Oracle, contacts: &mut Contacts, seed: &Seed) -> Option<Params> {
    for k in 0..=10 {
        let scale = 1.0 + REPAIR_GROWTH * f64::from(k) / 10.0;
        let mut x = seed.params;
        for v in &mut x[..6] {
            *v *= scale;
        }
        if contacts.admissible(oracle, &x) {
            return Some(x);
        }
    }
    None
}

/// Box of the feasible origins of `class`, plate mm, from the raster base map grown by two
/// pixels (the map is conservative; the exact zone test decides).
fn origin_region(ctx: &Ctx, class: usize) -> Option<Rect> {
    let (x0, y0, x1, y1) = ctx.origins(class).bounds()?;
    let grid = ctx.grid();
    let to_mm = |px: i32, origin: f64| origin + f64::from(px) * grid.resolution;
    let margin = 2.0 * grid.resolution;
    Some(Rect::new(
        to_mm(x0, grid.origin.x) - margin,
        to_mm(y0, grid.origin.y) - margin,
        to_mm(x1, grid.origin.x) + margin,
        to_mm(y1, grid.origin.y) + margin,
    ))
}

fn to_placements(placed: &[(usize, Point)]) -> Vec<PrintPlacement> {
    placed
        .iter()
        .map(|&(print, at)| PrintPlacement {
            print,
            x_mm: at.x,
            y_mm: at.y,
        })
        .collect()
}

/// Probes per side of `tighten`, evenly across the box.
const PROBES: usize = 3;
const PROBE_BISECTIONS: usize = 24;

/// The raster box of feasible origins cut down to where the exact zone test says copies fit.
/// The raster box is grown by two pixels, and the search pulls sites towards it: left as is,
/// the pull would stop at the raster edge, short of the real one. Each side moves to the
/// most generous boundary found on `PROBES` lines across the box (a bisection assuming the
/// line is blocked outside and free inside, which holds for a plate edge; where it does not
/// the box stays larger, never smaller than the raster's feasible set).
fn tighten(oracle: &Oracle, class: usize, region: Rect) -> Rect {
    let free = |x: f64, y: f64| oracle.zones_clear(class, Point { x, y });
    let across = |lo: f64, hi: f64, k: usize| lo + (hi - lo) * (k as f64 + 0.5) / PROBES as f64;
    // The boundary on a line: `from` is where it starts looking, `to` the far (free) end.
    let boundary = |from: f64, to: f64, at: &dyn Fn(f64) -> bool| -> f64 {
        if at(from) {
            return from;
        }
        if !at(to) {
            return to;
        }
        let (mut blocked, mut open) = (from, to);
        for _ in 0..PROBE_BISECTIONS {
            let mid = 0.5 * (blocked + open);
            if at(mid) {
                open = mid;
            } else {
                blocked = mid;
            }
        }
        open
    };
    let (cx, cy) = (
        0.5 * (region.min_x + region.max_x),
        0.5 * (region.min_y + region.max_y),
    );
    let mut out = Rect::new(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for k in 0..PROBES {
        let y = across(region.min_y, region.max_y, k);
        let x = across(region.min_x, region.max_x, k);
        out.min_x = out.min_x.min(boundary(region.min_x, cx, &|v| free(v, y)));
        out.max_x = out.max_x.max(boundary(region.max_x, cx, &|v| free(v, y)));
        out.min_y = out.min_y.min(boundary(region.min_y, cy, &|v| free(x, v)));
        out.max_y = out.max_y.max(boundary(region.max_y, cy, &|v| free(x, v)));
    }
    out
}

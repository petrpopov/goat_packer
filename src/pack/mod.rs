//! The deterministic packing portfolio (design 2026-11-02, section 9): lattices, sweeps, edge
//! fill and improvement over typed kinds. Owned by work package P3 (this file and the
//! skeleton), P4-P6 (members); the submodules are declared here so no one else edits this file.
//!
//! A run: `context::Ctx` holds the masks, no-fit sets and free maps of every rotation class
//! and the tower at each candidate anchor; members propose layouts; `select` ranks them,
//! raster-checks the shortlist and vector-checks the winner.
//!
//! Frame convention: `pack` takes one `ObjectPrint` per allowed rotation class, each already
//! in the plate-aligned frame of its rotation (the mesh turned by `rotation_deg` about the
//! frame centre; supports are computed in that frame because they are not
//! rotation-equivariant). Placing a print is a pure translation of its frame origin, so a
//! placement is (class, x, y) and never an angle.

pub mod bound;
pub mod context;
pub mod edge_fill;
pub mod free_map;
pub mod improve;
pub mod ladder;
pub mod lattice;
mod members;
pub(crate) mod legacy;
pub mod no_fit;
pub mod robustness;
pub(crate) mod rows;
pub mod select;
pub mod sweep;
pub mod tower;
pub mod tower_scan;

pub use bound::UpperBounds;
pub use context::{Ctx, TowerLayer};
pub use ladder::LevelReport;
pub use free_map::{FreeCounts, FreeMap};
pub use select::{CheckFailure, Member, RankKey, TieKey, ValidationReport};
pub use tower::{TowerSpec, anchor_candidates};
pub use tower_scan::scan_anchors;

use crate::geometry::Point;
use crate::keepout::PlateRules;
use crate::kind_masks::{KindShapes, MaskRules, OriginGrid, PlateZone, plate_zones};
use crate::object_print::ObjectPrint;
use crate::options::PackOptions;
use crate::validation::{
    LayoutRules, PrintPlacement, RasterValidator, VectorValidator, check_prints,
};
use members::member_candidates;
use rayon::prelude::*;
use select::{Candidate, choose, failure_summary};
use serde::Serialize;
use std::time::Instant;

/// The members `pack` runs on fixed rotation classes. `Mixed` needs a footprint that may
/// turn freely, so only the legacy search runs it.
pub const DEFAULT_MEMBERS: [Member; 6] = [
    Member::Lattice,
    Member::DoubleLattice,
    Member::TwoBlock,
    Member::Sweep,
    Member::Rows,
    Member::Improve,
];

/// The published layout of a run and how it was found.
#[derive(Clone, Debug, Serialize)]
pub struct PackOutcome {
    /// Every copy: its rotation class (index into the prints) and where that print's frame
    /// origin goes on the plate, mm.
    pub placements: Vec<PrintPlacement>,
    /// The tower anchor (`wipe_tower_x/y`) the layout was packed around; `None` without one.
    pub tower_anchor: Option<Point>,
    pub member: Member,
    pub key: RankKey,
    /// The most copies each member placed (before validation), in run order.
    pub member_counts: Vec<MemberCount>,
    /// Per member whose layouts `Improve` took as a start, the best count it reached.
    pub improved_counts: Vec<MemberCount>,
    pub validation: ValidationReport,
    /// The raster resolution of the published layout, mm (a ladder's winning level).
    pub resolution_mm: f64,
    /// The most copies of any mix of classes the plate holds at the winning resolution
    /// (`bound`); `None` when no norm ball fits the no-fit region.
    pub upper_bound: Option<usize>,
    /// The same per class: layouts of that class alone.
    pub class_bounds: Vec<Option<usize>>,
    /// One report per level the ladder ran, coarse first. Carries wall times, so like
    /// `timings` it is never serialized: two runs must write identical outcome files.
    #[serde(skip)]
    pub levels: Vec<LevelReport>,
    /// Wall-clock time per stage, for benchmarks only (never part of a result).
    #[serde(skip)]
    pub timings: StageTimes,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct MemberCount {
    pub member: Member,
    pub count: usize,
}

/// Seconds per stage of a run.
#[derive(Clone, Copy, Debug, Default)]
pub struct StageTimes {
    /// Kind masks, base free maps and tower layers.
    pub context_s: f64,
    /// The upper bound (`bound`).
    pub bound_s: f64,
    pub members_s: f64,
    /// Ranking and both checks.
    pub select_s: f64,
}

/// Packs copies of `prints` (one per allowed rotation class) on the plate of `rules`, with
/// the tower of `tower` at one of its candidate anchors, using every default member.
///
/// From `options` the run takes `resolution`, `clearance`, `attempts` (the effort: sweep
/// variants and the lattice budget), `max_objects` and `support_overlap_mm`; the plate comes
/// from `rules` (the bed fields and `lock_orientation` belong to the legacy entry points).
/// Keepouts must be in the plate frame.
pub fn pack(
    prints: &[ObjectPrint],
    rules: &PlateRules,
    tower: Option<&TowerSpec>,
    options: &PackOptions,
) -> Result<PackOutcome, String> {
    pack_members(prints, rules, tower, options, &DEFAULT_MEMBERS)
}

/// `pack` with an explicit member list (benchmarks; a member's own count).
///
/// The run climbs the resolution ladder of `options` (`ladder`): the best layout of any level
/// is published, and a level that reaches the upper bound ends the climb.
pub fn pack_members(
    prints: &[ObjectPrint],
    rules: &PlateRules,
    tower: Option<&TowerSpec>,
    options: &PackOptions,
    members: &[Member],
) -> Result<PackOutcome, String> {
    check_run(prints, options, members)?;
    ladder::run(options, members, None, |resolution, level_members| {
        let level = PackOptions {
            resolution,
            ..options.clone()
        };
        pack_level(prints, rules, tower, &level, level_members)
    })
}

/// Continues the ladder of `options` from `coarse`, an earlier outcome for the same prints,
/// rules, tower and members: only the levels `coarse` has not run, finer members only, and a
/// finer layout replaces `coarse` when it ranks strictly higher. Callers use it to climb the
/// ladder for the plates that matter after a coarse pass over all of them.
pub fn refine_members(
    prints: &[ObjectPrint],
    rules: &PlateRules,
    tower: Option<&TowerSpec>,
    options: &PackOptions,
    members: &[Member],
    coarse: PackOutcome,
) -> Result<PackOutcome, String> {
    check_run(prints, options, members)?;
    ladder::run(options, members, Some(coarse), |resolution, level_members| {
        let level = PackOptions {
            resolution,
            ..options.clone()
        };
        pack_level(prints, rules, tower, &level, level_members)
    })
}

/// `refine_members` with every default member.
pub fn refine(
    prints: &[ObjectPrint],
    rules: &PlateRules,
    tower: Option<&TowerSpec>,
    options: &PackOptions,
    coarse: PackOutcome,
) -> Result<PackOutcome, String> {
    refine_members(prints, rules, tower, options, &DEFAULT_MEMBERS, coarse)
}

/// One complete run at `options.resolution`.
fn pack_level(
    prints: &[ObjectPrint],
    rules: &PlateRules,
    tower: Option<&TowerSpec>,
    options: &PackOptions,
    members: &[Member],
) -> Result<PackOutcome, String> {
    let started = Instant::now();
    let zones = plate_zones(rules, None)?;
    let grid = OriginGrid {
        origin: Point {
            x: rules.printable_area.min_x,
            y: rules.printable_area.min_y,
        },
        resolution: options.resolution,
    };
    let ctx = fixed_context(prints, rules, tower, options, &zones, grid)?;
    let context_s = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let bounds = bound::upper_bounds(&ctx);
    let bound_s = started.elapsed().as_secs_f64();
    let started = Instant::now();
    // Nothing places more copies than the bound, so the members may stop there.
    let capped = PackOptions {
        max_objects: capped_objects(options.max_objects, bounds.overall),
        ..options.clone()
    };
    let candidates = member_candidates(&ctx, members, &capped);
    let member_counts = best_counts(members, &candidates);
    let improved_counts = improve::improved_counts(&candidates);
    let members_s = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let checks = Checks::new(prints, rules, options, &ctx)?;
    let (winner, validation) = checks
        .choose(candidates)
        .map_err(|report| failure_summary(&report))?;
    let select_s = started.elapsed().as_secs_f64();
    Ok(PackOutcome {
        placements: print_placements(&winner, &grid),
        tower_anchor: winner.anchor.map(|a| ctx.towers()[a].anchor),
        member: winner.key.tie.member,
        key: winner.key,
        member_counts,
        improved_counts,
        validation,
        resolution_mm: options.resolution,
        upper_bound: bounds.overall,
        class_bounds: bounds.per_class.clone(),
        levels: Vec::new(),
        timings: StageTimes {
            context_s,
            bound_s,
            members_s,
            select_s,
        },
    })
}

/// The object cap the members get: the caller's (0 = none) lowered to the bound.
fn capped_objects(max_objects: usize, bound: Option<usize>) -> usize {
    match (max_objects, bound) {
        (0, Some(bound)) => bound,
        (cap, Some(bound)) => cap.min(bound),
        (cap, None) => cap,
    }
}

fn check_run(
    prints: &[ObjectPrint],
    options: &PackOptions,
    members: &[Member],
) -> Result<(), String> {
    if prints.is_empty() {
        return Err("no rotation class to pack".to_owned());
    }
    check_prints(prints)?;
    let bad = |v: f64| !v.is_finite() || v < 0.0;
    if bad(options.resolution)
        || options.resolution == 0.0
        || bad(options.clearance)
        || options.support_overlap_mm.is_some_and(bad)
    {
        return Err("invalid packing resolution, clearance or support overlap".to_owned());
    }
    options.foreign_gaps.check()?;
    if let Some(member) = members.iter().find(|m| !DEFAULT_MEMBERS.contains(m)) {
        return Err(format!(
            "member {member:?} does not run on fixed rotation classes yet"
        ));
    }
    Ok(())
}

/// Masks of every class, base maps, and the tower at each candidate anchor.
fn fixed_context<'a>(
    prints: &[ObjectPrint],
    rules: &PlateRules,
    tower: Option<&TowerSpec>,
    options: &PackOptions,
    zones: &'a [PlateZone],
    grid: OriginGrid,
) -> Result<Ctx<'a>, String> {
    let mask_rules = MaskRules {
        clearance_mm: options.clearance,
        foreign: options.foreign_gaps,
        body_rule: rules.body_rule,
        support_overlap_mm: options.support_overlap_mm,
    };
    let masks = prints
        .iter()
        .map(|print| KindShapes::new(print, mask_rules).rasterize(options.resolution))
        .collect();
    let mut ctx = Ctx::new(masks, (0..prints.len()).collect(), zones, grid);
    if let Some(spec) = tower {
        let anchors = scan_anchors(spec, zones)?;
        ctx.add_towers(anchors.iter().map(|&a| (a, spec.zones(a))).collect());
    }
    Ok(ctx)
}

fn best_counts(members: &[Member], candidates: &[Candidate]) -> Vec<MemberCount> {
    members
        .iter()
        .map(|&member| MemberCount {
            member,
            count: candidates
                .iter()
                .filter(|c| c.key.tie.member == member)
                .map(|c| c.key.count)
                .max()
                .unwrap_or(0),
        })
        .collect()
}

/// A candidate's copies as print placements: class id and frame origin on the plate, mm.
pub(crate) fn print_placements(candidate: &Candidate, grid: &OriginGrid) -> Vec<PrintPlacement> {
    candidate
        .placements
        .iter()
        .map(|p| {
            let (x_mm, y_mm) = p.mm(grid);
            PrintPlacement {
                print: p.class,
                x_mm,
                y_mm,
            }
        })
        .collect()
}

/// The checks of a fixed-class run: per anchor, the plate zones plus that anchor's tower.
/// Rules are built for the anchors candidates use only (a scan lists hundreds).
struct Checks<'a> {
    prints: &'a [ObjectPrint],
    grid: OriginGrid,
    base: LayoutRules,
    /// Tower zones per anchor slot.
    towers: Vec<Vec<PlateZone>>,
}

impl<'a> Checks<'a> {
    fn new(
        prints: &'a [ObjectPrint],
        plate: &PlateRules,
        options: &PackOptions,
        ctx: &Ctx,
    ) -> Result<Checks<'a>, String> {
        Ok(Checks {
            prints,
            grid: *ctx.grid(),
            base: LayoutRules::new(
                plate,
                None,
                options.clearance,
                options.foreign_gaps,
                options.support_overlap_mm,
            )?,
            towers: ctx.towers().iter().map(|t| t.zones.clone()).collect(),
        })
    }

    /// The rules of a candidate at `anchor`: the plate plus that anchor's tower.
    fn rules(&self, anchor: Option<usize>) -> LayoutRules {
        let mut rules = self.base.clone();
        if let Some(slot) = anchor {
            rules.zones.extend(self.towers[slot].iter().cloned());
        }
        rules
    }

    /// `select::choose` with one raster validator per anchor: the anchors' candidates are
    /// checked in parallel (each validator's mask cache serves every candidate at its
    /// anchor), then the vector check runs on the winner.
    fn choose(
        &self,
        candidates: Vec<Candidate>,
    ) -> Result<(Candidate, ValidationReport), ValidationReport> {
        let vector = |c: &Candidate| -> Result<(), String> {
            VectorValidator::new(self.prints, &self.rules(c.anchor))?
                .check(&print_placements(c, &self.grid))
        };
        let raster = |shortlist: &[Candidate]| self.raster_verdicts(shortlist);
        choose(candidates, raster, vector).map_err(|(_, report)| report)
    }

    /// One raster verdict per candidate, in shortlist order.
    fn raster_verdicts(&self, shortlist: &[Candidate]) -> Vec<Result<(), String>> {
        let mut anchors: Vec<Option<usize>> = shortlist.iter().map(|c| c.anchor).collect();
        anchors.sort_unstable();
        anchors.dedup();
        let by_anchor: Vec<Vec<(usize, Result<(), String>)>> = anchors
            .into_par_iter()
            .map(|anchor| {
                let mine: Vec<usize> = (0..shortlist.len())
                    .filter(|&i| shortlist[i].anchor == anchor)
                    .collect();
                match RasterValidator::new(self.prints, &self.rules(anchor), self.grid) {
                    Ok(mut validator) => mine
                        .into_iter()
                        .map(|i| {
                            (
                                i,
                                validator.check(&print_placements(&shortlist[i], &self.grid)),
                            )
                        })
                        .collect(),
                    Err(error) => mine.into_iter().map(|i| (i, Err(error.clone()))).collect(),
                }
            })
            .collect();
        let mut verdicts: Vec<Option<Result<(), String>>> = vec![None; shortlist.len()];
        for (index, verdict) in by_anchor.into_iter().flatten() {
            verdicts[index] = Some(verdict);
        }
        verdicts
            .into_iter()
            .map(|v| v.expect("every candidate has an anchor slot"))
            .collect()
    }
}

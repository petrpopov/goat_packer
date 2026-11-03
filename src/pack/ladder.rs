//! The resolution ladder (research 2026-11-03): the same run at a coarse and then finer raster
//! resolutions, the best layout kept. The raster at 0.25 mm loses up to ~0.5 mm per contact
//! (lattice vectors and conservative cover are whole pixels), which is where the locked goat
//! plates still had room: at 0.15-0.2 mm U1 went 58 -> 59 and A2L 74 -> 78, P1S stayed 47.
//!
//! Every level is a complete run (its own `Ctx`, members, raster check and vector check), so
//! a winner of any level is as safe as a single-resolution winner; the vector check is the
//! gate and does not depend on the resolution. Levels run one after the other: a finer
//! level's context costs more memory (A2L at 0.15 mm: ~380 MB peak) and its members already
//! use every core. Finer levels run the lattice family and `Improve` only: the greedy
//! members (`Sweep`, `Rows`) trail the lattices by 2-5 copies on these plates at every
//! resolution and cost a full run per tower anchor.

use super::PackOutcome;
use super::select::Member;
use crate::options::PackOptions;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Members that run on the finer levels, when the caller asked for them.
const FINE_MEMBERS: [Member; 4] = [
    Member::Lattice,
    Member::DoubleLattice,
    Member::TwoBlock,
    Member::Improve,
];

/// What one level of the ladder did.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LevelReport {
    pub resolution_mm: f64,
    /// Copies of the level's published layout; 0 when it failed.
    pub count: usize,
    pub member: Option<Member>,
    /// The bound of any mix of classes at this resolution (`pack::bound`).
    pub upper_bound: Option<usize>,
    pub wall_s: f64,
    /// Why the level produced no layout.
    pub error: Option<String>,
}

/// The resolutions to run, coarse first: `options.resolutions`, or `options.resolution`
/// alone. Repeats are dropped (the first one counts); each must be positive and finite.
pub(super) fn levels(options: &PackOptions) -> Result<Vec<f64>, String> {
    let given: &[f64] = if options.resolutions.is_empty() {
        std::slice::from_ref(&options.resolution)
    } else {
        &options.resolutions
    };
    let mut levels: Vec<f64> = Vec::new();
    for &resolution in given {
        if !resolution.is_finite() || resolution <= 0.0 {
            return Err("invalid packing resolution".to_owned());
        }
        if !levels.contains(&resolution) {
            levels.push(resolution);
        }
    }
    Ok(levels)
}

/// The members a level runs: all of them on the first, the lattice family and `Improve` on
/// the finer ones.
fn members_for(finer: bool, members: &[Member]) -> Vec<Member> {
    if !finer {
        return members.to_vec();
    }
    members
        .iter()
        .copied()
        .filter(|m| FINE_MEMBERS.contains(m))
        .collect()
}

/// True when `a` ranks strictly above `b` (a finer level replaces a coarser winner only then):
/// more copies, then the rank key's slack and canonical flags, then the smaller box in mm²
/// (the key's own box is in pixels of its level).
fn outranks(a: &PackOutcome, b: &PackOutcome) -> bool {
    let area = |o: &PackOutcome| o.key.bbox_area_px as f64 * o.resolution_mm * o.resolution_mm;
    let rank = |o: &PackOutcome| {
        (
            o.key.count,
            o.key.robustness_quarter_mm,
            o.key.canonical_clear,
        )
    };
    match rank(a).cmp(&rank(b)) {
        std::cmp::Ordering::Equal => area(a) < area(b),
        order => order.is_gt(),
    }
}

/// Runs `level_run(resolution, members)` for each level and returns the best outcome with the
/// ladder's report. Stops when a level reaches its upper bound. A level that fails is
/// recorded; the run fails only when no level produced a layout.
///
/// With a `seed` (the outcome of an earlier run of the same plate) the levels it already ran
/// are skipped, every other level counts as a finer one, and the seed is the layout to beat:
/// `pack_members` followed by this equals one `pack_members` over the whole ladder.
pub(super) fn run(
    options: &PackOptions,
    members: &[Member],
    seed: Option<PackOutcome>,
    mut level_run: impl FnMut(f64, &[Member]) -> Result<PackOutcome, String>,
) -> Result<PackOutcome, String> {
    let mut reports: Vec<LevelReport> = seed.as_ref().map_or_else(Vec::new, |s| s.levels.clone());
    let seeded = seed.is_some();
    let mut best = seed;
    let mut first_error: Option<String> = None;
    for (index, resolution) in levels(options)?.into_iter().enumerate() {
        if reports.iter().any(|r| r.resolution_mm == resolution) {
            continue;
        }
        if best.as_ref().is_some_and(at_bound) {
            break;
        }
        let run_members = members_for(seeded || index > 0, members);
        if run_members.is_empty() {
            break;
        }
        let started = Instant::now();
        let result = level_run(resolution, &run_members);
        let wall_s = started.elapsed().as_secs_f64();
        match result {
            Ok(outcome) => {
                reports.push(report_of(resolution, &outcome, wall_s));
                if best.as_ref().is_none_or(|b| outranks(&outcome, b)) {
                    best = Some(outcome);
                }
            }
            Err(error) => {
                reports.push(failure_of(resolution, &error, wall_s));
                first_error.get_or_insert(error);
            }
        }
    }
    let mut best = best.ok_or_else(|| first_error.unwrap_or_else(|| "no level ran".to_owned()))?;
    best.levels = reports;
    Ok(best)
}

/// The layout holds as many copies as the plate can at its resolution.
fn at_bound(outcome: &PackOutcome) -> bool {
    outcome.upper_bound.is_some_and(|b| outcome.placements.len() >= b)
}

fn report_of(resolution_mm: f64, outcome: &PackOutcome, wall_s: f64) -> LevelReport {
    LevelReport {
        resolution_mm,
        count: outcome.placements.len(),
        member: Some(outcome.member),
        upper_bound: outcome.upper_bound,
        wall_s,
        error: None,
    }
}

fn failure_of(resolution_mm: f64, error: &str, wall_s: f64) -> LevelReport {
    LevelReport {
        resolution_mm,
        count: 0,
        member: None,
        upper_bound: None,
        wall_s,
        error: Some(error.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(resolution: f64, resolutions: &[f64]) -> PackOptions {
        let rules = crate::pack::bound::tests::plate(10.0, 10.0);
        PackOptions {
            resolution,
            resolutions: resolutions.to_vec(),
            ..crate::pack::bound::tests::options(&rules)
        }
    }

    #[test]
    fn the_ladder_is_the_list_or_the_single_resolution() {
        assert_eq!(levels(&options(0.5, &[])).unwrap(), vec![0.5]);
        let ladder = options(0.5, &[0.25, 0.2, 0.25, 0.15]);
        assert_eq!(levels(&ladder).unwrap(), vec![0.25, 0.2, 0.15]);
        assert!(levels(&options(0.5, &[0.25, 0.0])).is_err());
        assert!(levels(&options(0.5, &[f64::NAN])).is_err());
    }

    #[test]
    fn finer_levels_keep_only_the_lattice_family_and_improve() {
        let all = [Member::Lattice, Member::Sweep, Member::Rows, Member::Improve];
        assert_eq!(members_for(false, &all), all.to_vec());
        assert_eq!(members_for(true, &all), vec![Member::Lattice, Member::Improve]);
        assert!(members_for(true, &[Member::Sweep]).is_empty());
    }
}

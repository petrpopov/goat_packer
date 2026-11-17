//! `--orientation auto`: the turn of the whole plate (every rotation class turned alike).
//!
//! A lattice is dense inside and loses copies along the bed edges, and how many depends on the
//! angle between the lattice and the edges: the U1 goat at 0.5 mm margins (2026-11-17) packs
//! 49 as a lattice at classes 0/180, 52 at 5/185, 51 at 325/145, and the count jumps between
//! neighbouring 5-degree turns (49..53 over the sweep), so the turn is searched, not derived.
//!
//! Every turn is screened with its own supports, modelled at `Precision::Fast`, packed by the
//! lattice family at the coarse resolution with a small lattice budget. The classes turned
//! rigidly (`ObjectPrint::turned`) would cost no model, but tree supports are not
//! rotation-equivariant (the roof lines stay in the bed frame; the U1 support base at 5
//! degrees differs from the turned 0-degree one by up to 0.65 mm) and the lattice count
//! swings with that: the rigid screen gave 5° 47 copies where its own model packs 52. The
//! best screened turn is then modelled at the job's precision and packed by the whole
//! pipeline beside the unturned plate, and the better real pack wins: the unturned plate is
//! always packed, so the search never returns fewer copies than a fixed turn of 0 would.

use goat_packer::{
    PackOptions,
    pack::{Member, PackOutcome, pack_members},
    progress::{self, Stage, StageDetail},
    support_model::precision::Precision,
};
use serde::{Deserialize, Serialize};

use crate::AppResult;
use crate::args::{SupportSource, turned_rotations};
use crate::from_model::modelled_plate_at;
use crate::gaps::GapVariant;
use crate::job::Job;
use crate::model::{Modelled, model_plate};
use crate::pack_cache::cached_members;
use crate::plan::PackPlan;
use crate::prepare::pack_variants;

/// Step between screened turns, degrees. 10 degrees misses the best U1 turn (5/185: 52 regular
/// against 49-50 at 0 and 10).
const STEP_DEG: f64 = 5.0;

/// Screened turns besides 0 that are modelled and packed in full. Each costs a model and a
/// pack (U1 at two threads, precise: ~15 + ~22 s).
const FINALISTS: usize = 1;

/// The members a turn is screened with: the regular ones. They decide how the lattice meets
/// the edges; `Improve` and the sweeps cost most of a pack and follow the lattices' count.
const SCREEN_MEMBERS: [Member; 3] = [Member::Lattice, Member::DoubleLattice, Member::TwoBlock];

/// The lattice budget of a screen (`Budget::for_effort`; the full budget is 64).
const SCREEN_EFFORT: usize = 64;

/// One screened or packed turn.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TurnCount {
    pub turn_deg: f64,
    pub count: usize,
}

/// What the search did, for the report.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OrientationReport {
    /// The turn of the published plate.
    pub turn_deg: f64,
    /// The rigidly turned classes' lattice count per screened turn.
    pub screened: Vec<TurnCount>,
    /// The turns modelled and packed in full, with their counts.
    pub packed: Vec<TurnCount>,
    pub screen_seconds: f64,
}

/// The search's result: the winning turn's model, outcome and gap variants, and the report.
pub struct Found {
    pub modelled: Modelled,
    pub outcome: PackOutcome,
    pub variants: Vec<GapVariant>,
    pub report: OrientationReport,
}

/// Screens the turns of the job's classes, packs the unturned plate and the best screened
/// turns in full, and keeps the best pack.
pub fn search(job: &Job) -> AppResult<Found> {
    let base = model_plate(job, &job.args.rotations)?;
    if base.source != SupportSource::Model {
        return Err("--orientation auto screens every turn with the support model, and the \
                    model does not cover this project: pack with a fixed --orientation"
            .into());
    }
    let started = std::time::Instant::now();
    let turns = screen_turns(&job.args.rotations);
    let screened = screen(job, &turns)?;
    let screen_seconds = started.elapsed().as_secs_f64();
    let finalists = finalists(&screened);
    eprintln!(
        "orientation: screened {} turns in {:.1} s ({}); packing turns {:?} in full",
        screened.len(),
        screen_seconds,
        screened
            .iter()
            .map(|t| format!("{}°: {}", t.turn_deg, t.count))
            .collect::<Vec<_>>()
            .join(", "),
        finalists,
    );
    let mut best: Option<(f64, Modelled, PackOutcome, Vec<GapVariant>)> = None;
    let mut packed = Vec::new();
    let mut base = Some(base);
    for turn in finalists {
        let model = match base.take().filter(|_| turn == 0.0) {
            Some(model) => model,
            None => model_plate(job, &turned_rotations(&job.args.rotations, turn)?)?,
        };
        let (modelled, outcome, variants) = pack_variants(job, model)?;
        eprintln!("orientation: turn {turn}° packs {}", outcome.placements.len());
        packed.push(TurnCount {
            turn_deg: turn,
            count: outcome.placements.len(),
        });
        // Strictly better only: on a tie the earlier turn stays, and turn 0 comes first.
        if best.as_ref().is_none_or(|(_, _, held, _)| outranks(&outcome, held)) {
            best = Some((turn, modelled, outcome, variants));
        }
    }
    let (turn_deg, modelled, outcome, variants) = best.expect("turn 0 is always packed");
    Ok(Found {
        modelled,
        outcome,
        variants,
        report: OrientationReport {
            turn_deg,
            screened,
            packed,
            screen_seconds,
        },
    })
}

/// True when `a` ranks above `b`: more copies with a regular layout's head start, then more
/// copies.
fn outranks(a: &PackOutcome, b: &PackOutcome) -> bool {
    (a.key.preferred, a.key.count) > (b.key.preferred, b.key.count)
}

/// The turns worth screening: every `STEP_DEG` up to the symmetry of the class set (0/180
/// repeats after 180 degrees: turning it by 180 swaps the classes).
fn screen_turns(rotations: &[f64]) -> Vec<f64> {
    let period = [90.0, 180.0]
        .into_iter()
        .find(|&p| same_set(rotations, p))
        .unwrap_or(360.0);
    let steps = (period / STEP_DEG).round() as usize;
    (0..steps).map(|i| i as f64 * STEP_DEG).collect()
}

/// Whether turning `rotations` by `turn` gives the same set of classes.
fn same_set(rotations: &[f64], turn: f64) -> bool {
    let close = |a: f64, b: f64| {
        let d = (a - b).rem_euclid(360.0);
        d < 1e-6 || 360.0 - d < 1e-6
    };
    rotations
        .iter()
        .all(|&r| rotations.iter().any(|&q| close(r + turn, q)))
}

/// The lattice count of every turn, its classes modelled at `Precision::Fast`.
fn screen(job: &Job, turns: &[f64]) -> AppResult<Vec<TurnCount>> {
    turns
        .iter()
        .enumerate()
        .map(|(index, &turn)| {
            progress::stage_step(
                Stage::Packing,
                &format!("screening plate turn {turn}°..."),
                None,
                &StageDetail::new(index, turns.len()).with("turn_deg", turn),
            );
            let rotations = turned_rotations(&job.args.rotations, turn)?;
            let plan: PackPlan = modelled_plate_at(job, &rotations, Precision::Fast)?.plan;
            let options = PackOptions {
                resolutions: vec![job.args.resolution],
                attempts: SCREEN_EFFORT,
                ..plan.options(&job.args)
            };
            // Cached like a full pack: a rerun over the same model screens in seconds.
            let count = cached_members(job.args.cache_dir.as_deref(), &plan, &options, &SCREEN_MEMBERS, || {
                pack_members(&plan.prints, &plan.rules, plan.tower.as_ref(), &options, &SCREEN_MEMBERS)
            })
            .map_or(0, |outcome| outcome.placements.len());
            Ok(TurnCount {
                turn_deg: turn,
                count,
            })
        })
        .collect()
}

/// Turn 0, then the `FINALISTS` best screened other turns (more copies first, then the
/// smaller turn).
fn finalists(screened: &[TurnCount]) -> Vec<f64> {
    let mut others: Vec<&TurnCount> = screened.iter().filter(|t| t.turn_deg != 0.0).collect();
    others.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then(a.turn_deg.total_cmp(&b.turn_deg))
    });
    std::iter::once(0.0)
        .chain(others.into_iter().take(FINALISTS).map(|t| t.turn_deg))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_stops_at_the_symmetry_of_the_classes() {
        assert_eq!(screen_turns(&[0.0, 180.0]).len(), 36);
        assert_eq!(screen_turns(&[0.0, 90.0, 180.0, 270.0]).len(), 18);
        assert_eq!(screen_turns(&[0.0]).len(), 72);
        assert_eq!(screen_turns(&[10.0, 190.0])[1], 5.0);
    }

    #[test]
    fn turn_zero_is_always_packed_then_the_best_screened_turn() {
        let screened: Vec<TurnCount> = [(0.0, 49), (5.0, 52), (10.0, 52), (20.0, 50)]
            .into_iter()
            .map(|(turn_deg, count)| TurnCount { turn_deg, count })
            .collect();
        assert_eq!(finalists(&screened), vec![0.0, 5.0]);
        let flat: Vec<TurnCount> = [(0.0, 49), (5.0, 48)]
            .into_iter()
            .map(|(turn_deg, count)| TurnCount { turn_deg, count })
            .collect();
        assert_eq!(finalists(&flat), vec![0.0, 5.0], "a worse screen still gets its real pack");
    }
}

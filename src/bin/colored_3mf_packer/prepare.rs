//! From the source project to a packed plate: modelling, packing and ordering.

use std::time::Instant;

use goat_packer::{
    PackedPlacement, PrintPlacement,
    pack::{LevelReport, PackOutcome},
    plate_centre::plate_centre,
    progress::{self, LayoutSource, Stage},
};

use crate::AppResult;
use crate::assembly::tower_rect;
use crate::gaps::{GapVariant, choose, floor_warning, refine_rungs};
use crate::job::Job;
use crate::model::{Modelled, model_plate};
use crate::order::{BAMBU_OBSTACLE_MM, barriers, canonical_first, centre_out};
use crate::plan::{MemberResult, PackPlan, PackState, PackSummary, STATE_VERSION};
use crate::probe::time_probe;
use goat_packer::support_model::flavour::Family;

/// Models the plate (probes, tower), packs it and orders the copies.
pub fn prepare_plate(job: &Job) -> AppResult<PackState> {
    // A single object that already overruns a time window fails before anything else costs time.
    let probe_sample = match job.window_s {
        Some(window_s) => Some(time_probe(job, window_s)?),
        None => None,
    };
    let (modelled, outcome, variants) = pack_variants(job)?;
    // Centre-out time-window plate: the kept objects gather around the smallest heat zone's
    // centre (any prefix of a valid layout is valid, so the order is all it takes).
    let centre = if job.args.time.centre_out() {
        Some(plate_centre(&job.profile.heat_zones, job.profile.usable_bed()?))
    } else {
        None
    };
    let mut state = PackState {
        version: STATE_VERSION,
        plan: modelled.plan,
        placements: Vec::new(),
        tower_anchor: None,
        centre,
        probe_seconds: modelled.probe_seconds,
        probe_sample,
        summary: PackSummary {
            probes: modelled.summaries,
            model_seconds: modelled.model_seconds,
            ..PackSummary::default()
        },
    };
    state.summary.gap_variants = variants;
    place(job, &mut state, outcome)?;
    Ok(state)
}

/// Probes the classes once, then packs the plan once per foreign-gap pair (the prints are
/// shared, only the masks change). Returns the winning plan with its outcome and every
/// variant packed, for the report.
fn pack_variants(job: &Job) -> AppResult<(Modelled, PackOutcome, Vec<GapVariant>)> {
    if let Some(warning) = floor_warning(job.args.foreign_gap_floor) {
        progress::stage(Stage::Packing, &format!("warning: {warning}"), None, None);
    }
    let mut model = model_plate(job)?;
    let (variants, mut outcomes): (Vec<_>, Vec<_>) =
        pack_foreign_pairs(job, &model.plan).into_iter().unzip();
    let Some(best) = choose(&variants) else {
        let first = variants.iter().find_map(|v| v.error.clone());
        return Err(first.unwrap_or_else(|| "no gap variant to pack".to_owned()).into());
    };
    let chosen = &variants[best];
    model.plan = model.plan.with_foreign_gaps(chosen.foreign());
    println!(
        "chosen gaps: foreign base {} mm, band {} mm: {} objects",
        chosen.foreign_base_mm, chosen.foreign_band_mm, chosen.count
    );
    let outcome = outcomes[best].take().expect("the chosen variant packed");
    Ok((model, outcome, variants))
}

/// One pack of `plan` per `--foreign-gaps` pair, in the order given, each with its wall time.
/// A pair that yields no valid layout is recorded with its error, not hidden. Every pair is
/// packed at the coarsest resolution first; only the pairs near the best climb the finer
/// levels (`gaps::REFINE_SLACK`).
fn pack_foreign_pairs(job: &Job, plan: &PackPlan) -> Vec<(GapVariant, Option<PackOutcome>)> {
    let mut packed = pack_coarse_pairs(job, plan);
    let counts: Vec<Option<usize>> =
        packed.iter().map(|(v, o)| o.as_ref().map(|_| v.count)).collect();
    for rung in refine_rungs(&counts) {
        let (variant, outcome) = &mut packed[rung];
        let Some(coarse) = outcome.take() else { continue };
        let started = Instant::now();
        let rung_plan = plan.with_foreign_gaps(variant.foreign());
        let refined = match rung_plan.refine(&job.args, coarse.clone()) {
            Ok(refined) => refined,
            Err(error) => {
                eprintln!("finer resolutions failed, coarse layout kept: {error}");
                coarse
            }
        };
        variant.wall_s += started.elapsed().as_secs_f64();
        variant.count = refined.placements.len();
        variant.levels = refined.levels.clone();
        eprintln!(
            "foreign gaps {}:{} refined -> {} objects ({})",
            variant.foreign_base_mm,
            variant.foreign_band_mm,
            variant.count,
            levels_line(&refined.levels),
        );
        *outcome = Some(refined);
    }
    packed
}

/// The coarse pass of `pack_foreign_pairs`.
fn pack_coarse_pairs(job: &Job, plan: &PackPlan) -> Vec<(GapVariant, Option<PackOutcome>)> {
    job.args
        .foreign_gaps
        .iter()
        .map(|&foreign| {
            progress::stage(
                Stage::Packing,
                &format!(
                    "packing (foreign gaps base {} band {}; {} attempts, seed {})...",
                    foreign.base_mm, foreign.band_mm, job.args.attempts, job.args.seed
                ),
                None,
                None,
            );
            let started = Instant::now();
            let result = plan.with_foreign_gaps(foreign).pack_coarse(&job.args);
            let mut variant = GapVariant {
                foreign_base_mm: foreign.base_mm,
                foreign_band_mm: foreign.band_mm,
                count: 0,
                wall_s: started.elapsed().as_secs_f64(),
                error: None,
                levels: Vec::new(),
            };
            let outcome = match result {
                Ok(outcome) => {
                    variant.count = outcome.placements.len();
                    variant.levels = outcome.levels.clone();
                    Some(outcome)
                }
                Err(error) => {
                    variant.error = Some(error.to_string());
                    None
                }
            };
            eprintln!(
                "foreign gaps {}:{} -> {} objects in {:.1} s{}",
                foreign.base_mm,
                foreign.band_mm,
                variant.count,
                variant.wall_s,
                variant.error.as_deref().map_or(String::new(), |e| format!(" ({e})")),
            );
            (variant, outcome)
        })
        .collect()
}

/// Packs the plan, orders the copies (centre-out on window plates, then the canonical copy of
/// each class first) and records the outcome in `state`.
pub fn arrange(job: &Job, state: &mut PackState) -> AppResult<()> {
    let args = &job.args;
    progress::stage(
        Stage::Packing,
        &format!("packing ({} attempts, seed {})...", args.attempts, args.seed),
        None,
        None,
    );
    let outcome = state.plan.pack(args)?;
    place(job, state, outcome)
}

/// Orders the copies of a packed `outcome` (centre-out on window plates, then the canonical
/// copy of each class first) and records it in `state`.
fn place(job: &Job, state: &mut PackState, outcome: PackOutcome) -> AppResult<()> {
    log_outcome(&outcome);
    let mut placements = outcome.placements.clone();
    if let Some(centre) = state.centre {
        centre_out(&state.plan.prints, &mut placements, centre);
    }
    let zones = state.plan.zones(outcome.tower_anchor)?;
    let near_edge_mm = match job.flavour.flavour.family() {
        Family::Bambu => BAMBU_OBSTACLE_MM,
        Family::Orca | Family::SnapmakerOrca => 0.0,
    };
    state.summary.canonical =
        canonical_first(&state.plan.prints, &barriers(&zones), near_edge_mm, &mut placements);
    state.summary.member = format!("{:?}", outcome.member);
    state.summary.resolution_mm = outcome.resolution_mm;
    state.summary.upper_bound = outcome.upper_bound;
    state.summary.class_bounds = outcome.class_bounds.clone();
    state.summary.levels = outcome.levels.clone();
    state.summary.member_counts = outcome
        .member_counts
        .iter()
        .map(|result| MemberResult {
            member: format!("{:?}", result.member),
            count: result.count,
        })
        .collect();
    state.placements = placements;
    state.tower_anchor = outcome.tower_anchor;
    Ok(())
}

/// `0.25 mm: 55 in 2.1 s, 0.2 mm: 56 in 2.4 s, ...` for the progress line.
fn levels_line(levels: &[LevelReport]) -> String {
    levels
        .iter()
        .map(|l| format!("{} mm: {} in {:.1} s", l.resolution_mm, l.count, l.wall_s))
        .collect::<Vec<_>>()
        .join(", ")
}

fn log_outcome(outcome: &PackOutcome) {
    let counts: Vec<String> = outcome
        .member_counts
        .iter()
        .map(|result| format!("{:?} {}", result.member, result.count))
        .collect();
    eprintln!(
        "packed {} objects with {:?} at {} mm ({}); upper bound {} (classes {:?}); levels: {}; \
         tower anchor {:?}",
        outcome.placements.len(),
        outcome.member,
        outcome.resolution_mm,
        counts.join(", "),
        outcome.upper_bound.map_or("n/a".to_owned(), |b| b.to_string()),
        outcome.class_bounds,
        levels_line(&outcome.levels),
        outcome.tower_anchor.map(|anchor| (anchor.x, anchor.y)),
    );
}

/// Tells the webapp which layout is being built: the first class's silhouette and first-layer
/// support once, and every copy as the placement it is (class rotation about its origin).
pub fn show_layout(state: &PackState, source: LayoutSource) {
    let plan: &PackPlan = &state.plan;
    let outline = plan.prints.first().map(|print| {
        print.silhouette.iter().chain(&print.support_base).cloned().collect::<Vec<_>>()
    });
    let placements: Vec<PackedPlacement> = state
        .placements
        .iter()
        .map(|placement: &PrintPlacement| PackedPlacement {
            x_mm: placement.x_mm,
            y_mm: placement.y_mm,
            angle_deg: plan.prints[placement.print].rotation_deg,
        })
        .collect();
    progress::layout(
        source,
        &outline.unwrap_or_default(),
        &placements,
        plan.rules.printable_area,
        &[],
        tower_rect(plan, state.tower_anchor),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{test_job, test_plan, test_state};
    use goat_packer::ForeignGaps;

    #[test]
    fn arranging_records_the_layout_the_canonical_copies_and_the_members() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let mut state = test_state(test_plan(&job, 2));
        arrange(&job, &mut state).unwrap();
        assert!(state.placements.len() >= 25);
        assert!(!state.summary.member.is_empty() && !state.summary.member_counts.is_empty());
        // One canonical copy per class in use, and it stands first among its class.
        for canonical in &state.summary.canonical {
            let class = state.placements[canonical.index].print;
            assert_eq!(state.plan.prints[class].rotation_deg, canonical.rotation_deg);
            assert!(state.placements[..canonical.index].iter().all(|p| p.print != class));
        }
        assert_eq!(state.tower_anchor, None);
    }

    /// Squares of 10 mm with a first-layer support patch 2 mm wider on every side: a foreign
    /// base gap of g keeps neighbours g further apart than a bare touch.
    fn supported_plan(job: &Job) -> PackPlan {
        let mut plan = test_plan(job, 1);
        let ring = |h: f64| {
            let p = |x, y| goat_packer::Point { x, y };
            vec![vec![p(-h, -h), p(h, -h), p(h, h), p(-h, h), p(-h, -h)]]
        };
        plan.prints[0].support_base = ring(7.0);
        plan
    }

    #[test]
    fn every_foreign_pair_is_packed_and_the_best_count_wins_with_the_safer_tie() {
        let dir = tempfile::tempdir().unwrap();
        let mut job = test_job(dir.path());
        job.args.foreign_gaps = vec![
            ForeignGaps { base_mm: 6.0, band_mm: 1.0 },
            ForeignGaps { base_mm: 0.0, band_mm: 0.5 },
            ForeignGaps { base_mm: 0.0, band_mm: 0.25 },
        ];
        let packed = pack_foreign_pairs(&job, &supported_plan(&job));
        let counts: Vec<usize> = packed.iter().map(|(variant, _)| variant.count).collect();
        assert!(counts[0] < counts[1], "a 6 mm base gap costs copies: {counts:?}");
        // No band supports here: the band gap does not matter, so the two loose rungs tie and
        // the larger band gap wins.
        assert_eq!(counts[1], counts[2]);
        let variants: Vec<GapVariant> = packed.into_iter().map(|(variant, _)| variant).collect();
        assert_eq!(choose(&variants), Some(1));
        assert!(variants.iter().all(|v| v.error.is_none() && v.wall_s >= 0.0));
    }

    #[test]
    fn the_chosen_variant_is_what_the_plan_remembers() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let plan = supported_plan(&job).with_foreign_gaps(ForeignGaps { base_mm: 0.25, band_mm: 0.75 });
        let mut state = test_state(plan);
        arrange(&job, &mut state).unwrap();
        assert_eq!(state.plan.foreign_gaps, ForeignGaps { base_mm: 0.25, band_mm: 0.75 });
        // The layout was packed under those gaps and passes the exact check under them.
        state.plan.validate(&job.args, &state.placements, state.tower_anchor).unwrap();
    }

    #[test]
    fn a_window_plate_is_ordered_from_the_centre_out() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let mut state = test_state(test_plan(&job, 1));
        state.centre = Some((30.0, 30.0));
        arrange(&job, &mut state).unwrap();
        let distance = |p: &PrintPlacement| (p.x_mm - 30.0).hypot(p.y_mm - 30.0);
        // The first copy of the class is the canonical one and may be swapped in; the rest
        // follow the centre-out order.
        let rest: Vec<f64> = state.placements[1..].iter().map(distance).collect();
        assert!(rest.windows(2).all(|pair| pair[0] <= pair[1] + 1e-9), "{rest:?}");
    }
}

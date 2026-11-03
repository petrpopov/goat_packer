//! From the source project to a packed plate: modelling, packing and ordering.

use goat_packer::{
    PackedPlacement, PrintPlacement,
    pack::PackOutcome,
    plate_centre::plate_centre,
    progress::{self, LayoutSource, Stage},
};

use crate::AppResult;
use crate::assembly::tower_rect;
use crate::job::Job;
use crate::model::model_plate;
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
    let modelled = model_plate(job)?;
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
            ..PackSummary::default()
        },
    };
    arrange(job, &mut state)?;
    Ok(state)
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

fn log_outcome(outcome: &PackOutcome) {
    let counts: Vec<String> = outcome
        .member_counts
        .iter()
        .map(|result| format!("{:?} {}", result.member, result.count))
        .collect();
    eprintln!(
        "packed {} objects with {:?} ({}); tower anchor {:?}",
        outcome.placements.len(),
        outcome.member,
        counts.join(", "),
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

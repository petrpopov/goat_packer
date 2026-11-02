//! From the source project to a first packed plate: probe, footprint, tower position, packing
//! and the support sample.

use goat_packer::{
    PackResult, Rect, extract_footprint,
    colored_packer::PrimeTowerBounds,
    plate_centre::{RelocatedTower, ensure_relocated_tower_clear, plate_centre, relocate_keepout},
    progress::{self, Stage},
};

use crate::AppResult;
use crate::job::Job;
use crate::plan::{PackPlan, PackState, SupportRule, support_edge_reserve_mm};
use crate::probe::{footprint_from_probe, probe_source};
use crate::source::model_center_on_bed;
use crate::support_learning::sample_supports;
use crate::tower::{choose_tower_position, keep_in_gui_range};

/// Probes the source, packs the plate and learns the supports from a sample slice.
pub fn prepare_plate(job: &Job) -> AppResult<PackState> {
    let args = &job.args;
    let profile = &job.profile;
    let probe = probe_source(job)?;
    let silhouette = extract_footprint(&job.footprint_stl)?;
    let model_world_center = model_center_on_bed(&args.source_3mf, &job.footprint_stl);
    let found = footprint_from_probe(job, &probe, &silhouette, model_world_center);
    // Centre-out time-window plate (spec 2026-10-26-centre-out-zones-tower-design): the kept
    // objects gather around the smallest heat zone's centre, and the tower moves there too so
    // every colour change travels a short way. The tower's size does not depend on its
    // position, so the probe keepout shifted by the same vector is the reservation — no
    // re-probe; the first final slice verifies the slicer really moved it.
    let centre = if args.time.centre_out() {
        Some(plate_centre(&profile.heat_zones, profile.usable_bed()?))
    } else {
        None
    };
    let fixed_keepouts = profile.fixed_keepouts(found.purge_tip_overshoot)?;
    let centre_relocation = relocate_to_centre(job, centre, probe.tower, &fixed_keepouts)?;
    let mut footprint = found.footprint;
    // Bed bounds and the prime-tower keepout follow the patch UNION the whole-model
    // silhouette: the slicer's bed-collision warning measures the full model extent, which
    // hangs below the printed patch on a lying animal (its tail), silently eating the edge
    // margin — the manual 3.5/4.0 mm gap complaint. Every branch of `footprint_from_probe`
    // has already set bed_loops to that union: with rotations allowed the patch carries the
    // measured tip overshoot (~2 mm of Arachne miter past the silhouette at thin tips), and a
    // layout whose silhouette clears the bed edge can still print outside it — BambuStudio
    // then refuses the whole final slice with "too close to exclusion area" / "no object fully
    // inside it" (a real 17-object P1S goat plate died at mesh x=0.06 and x=255.7 on a 256 mm
    // bed this way).
    if footprint.bed_loops.is_none() {
        footprint.bed_loops = Some(footprint.loops.clone());
    }
    let lock_orientation = found.lock_orientation;
    let mut plan = PackPlan {
        footprint,
        lock_orientation,
        fixed_keepouts,
        tower_keepout: centre_relocation
            .map(|relocated| relocated.keepout)
            .or(probe.tower.map(|tower| tower.keepout)),
        relocation: centre_relocation,
        rule: if lock_orientation {
            SupportRule::Landing
        } else {
            SupportRule::Silhouette
        },
        patch_learned: false,
        mesh_silhouette: if lock_orientation { silhouette.loops.clone() } else { Vec::new() },
        support_edge_reserve_mm: support_edge_reserve_mm(profile),
    };
    progress::stage(
        Stage::Packing,
        &format!("packing ({} attempts, seed {})...", args.attempts, args.seed),
        None,
        None,
    );
    let (mut packed, mut tower_from_contest) = first_pack(job, &mut plan, centre, probe.tower)?;
    if sample_learned(job, &mut plan, &packed) {
        (packed, tower_from_contest) = first_pack(job, &mut plan, centre, probe.tower)?;
    }
    Ok(PackState {
        plan,
        packed,
        probe_tower: probe.tower,
        probe_seconds: probe.seconds,
        tower_from_contest,
        centre,
        probe_sample: probe.sample,
    })
}

/// Where the tower goes on a centre-out plate: next to the objects, inside the range the
/// slicer GUI keeps, clear of the fixed keepouts. `None` without a centre or a tower.
fn relocate_to_centre(
    job: &Job,
    centre: Option<(f64, f64)>,
    probe_tower: Option<PrimeTowerBounds>,
    fixed_keepouts: &[Rect],
) -> AppResult<Option<RelocatedTower>> {
    let usable_bed = job.profile.usable_bed()?;
    match (centre, probe_tower) {
        (Some(centre), Some(tower)) => {
            let moved = keep_in_gui_range(
                relocate_keepout(tower.keepout, centre, usable_bed),
                job.gui_tower.map(|range| range.keepout_min_range(tower.keepout)),
            );
            // The clamp only knows the bed; a centre near an exclusion corner or purge line
            // would put the printed tower on it.
            ensure_relocated_tower_clear(moved.keepout, fixed_keepouts)?;
            println!(
                "centre-out plate around ({:.1}, {:.1}): prime tower moves by ({:+.1}, {:+.1}) mm; \
                 keepout {:.2},{:.2} → {:.2},{:.2}",
                centre.0,
                centre.1,
                moved.shift.0,
                moved.shift.1,
                moved.keepout.min_x,
                moved.keepout.min_y,
                moved.keepout.max_x,
                moved.keepout.max_y
            );
            Ok(Some(moved))
        }
        (Some(centre), None) => {
            println!(
                "centre-out plate around ({:.1}, {:.1}); no tower to relocate",
                centre.0, centre.1
            );
            Ok(None)
        }
        (None, _) => Ok(None),
    }
}

/// The first layout. Without centre-out the tower may move: the source position competes with
/// the bed corners and the densest plate wins (spec 2026-10-31 rule 3). Also tells whether the
/// tower came from that contest.
fn first_pack(
    job: &Job,
    plan: &mut PackPlan,
    centre: Option<(f64, f64)>,
    probe_tower: Option<PrimeTowerBounds>,
) -> AppResult<(PackResult, bool)> {
    let (None, Some(tower)) = (centre, probe_tower) else {
        return Ok((plan.pack(job)?, false));
    };
    let (relocation, packed) = choose_tower_position(
        job,
        plan,
        tower.keepout,
        job.gui_tower.map(|range| range.keepout_min_range(tower.keepout)),
    )?;
    plan.tower_keepout = Some(relocation.map_or(tower.keepout, |relocated| relocated.keepout));
    plan.relocation = relocation;
    Ok((packed, relocation.is_some()))
}

/// The one-object probe comes from the source project; the slicer builds the supports of the
/// assembled plate a little differently (up to 0.4 mm wider bases, interfaces past the ears).
/// A few objects of the first layout, sliced as an assembled plate, print exactly what every
/// object of the full plate will (identical within 0.1 mm, sample vs plate within 0.5 mm); the
/// plate is packed again against those supports, height band by band — the tower contest
/// included: on a U1 job the bottom-left corner won the contest before the sample (48) and
/// packed 47 after it; the contest runs again on the learned supports. `true` when the
/// supports were learned.
fn sample_learned(job: &Job, plan: &mut PackPlan, packed: &PackResult) -> bool {
    if !(plan.lock_orientation && job.args.sample_objects > 0 && packed.placements.len() > 1) {
        return false;
    }
    match sample_supports(job, plan, packed) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("WARNING: sample slice failed, packing with the probe's supports only: {error}");
            false
        }
    }
}

//! Learning what the slicer really prints as supports, and reacting to supports that touch:
//! the sample slice, nudging touching objects apart, widening the reserved patch.

use goat_packer::{
    Footprint, PackResult, PackedPlacement, validate_result,
    gcode_collisions::{
        PlateAnalysis, SupportCollision, layer_zones_from_sample, match_placements, nudge_apart,
    },
    progress::{self, Stage},
};

use crate::AppResult;
use crate::job::Job;
use crate::plan::{PackPlan, pack_options};
use crate::slicing::slice_runs;

/// Slices a nudged plate may take before the support check falls back to a repack: each
/// round is a full final slice, and supports regenerate (and drift) on every one.
pub const MAX_NUDGE_ROUNDS: usize = 2;

/// A nudge smaller than this (mm) is not worth a log line.
const NUDGE_LOG_THRESHOLD_MM: f64 = 1e-9;

/// Slices `args.sample_objects` objects of `packed` (the one nearest the bed centre and its
/// nearest neighbours) as an assembled plate and moves their real supports into the plan:
/// the learned first-layer patch and the height-band `layer_zones`.
pub fn sample_supports(job: &Job, plan: &mut PackPlan, packed: &PackResult) -> AppResult<()> {
    let args = &job.args;
    let sample = sample_indices(job, packed)?;
    progress::stage(
        Stage::SampleSlice,
        &format!("slicing a sample of {} objects of the layout...", sample.len()),
        None,
        Some(sample.len()),
    );
    plan.build(job, packed)?;
    let sample_project = args.work_dir.join("sample.3mf");
    goat_packer::scheduler::subset_3mf_items(&args.output, &sample_project, &sample, false)
        .map_err(|error| format!("sample subset: {error}"))?;
    let slices = slice_runs(&job.profile, &sample_project, sample.len(), &args.work_dir, "sample")?;
    let gcodes: Vec<&[u8]> = slices.iter().map(|run| run.gcode.as_slice()).collect();
    let sampled: Vec<PackedPlacement> =
        sample.iter().map(|&index| packed.placements[index].clone()).collect();
    let (zones, analysis) = layer_zones_from_sample(&gcodes, &plan.footprint, &sampled)
        .map_err(|error| format!("sample slice: {error}"))?;
    let sampled_layout = PackResult { placements: sampled, ..packed.clone() };
    let mut learned = learn_support_patch(&plan.footprint, &sampled_layout, &analysis)?;
    println!(
        "sample slice: {} height bands of supports; first-layer patch {:.1}x{:.1} mm (probe \
         {:.1}x{:.1} mm)",
        zones.len(),
        learned.width,
        learned.height,
        plan.footprint.width,
        plan.footprint.height
    );
    learned.layer_zones = zones;
    plan.footprint = learned;
    Ok(())
}

/// The sample: the object nearest the bed centre and its nearest neighbours.
fn sample_indices(job: &Job, packed: &PackResult) -> AppResult<Vec<usize>> {
    let bed = job.profile.usable_bed()?;
    let centre = ((bed.min_x + bed.max_x) * 0.5, (bed.min_y + bed.max_y) * 0.5);
    let distance = |index: usize, to: (f64, f64)| {
        let placement = &packed.placements[index];
        (placement.x_mm - to.0).hypot(placement.y_mm - to.1)
    };
    let indices: Vec<usize> = (0..packed.placements.len()).collect();
    let first = *indices
        .iter()
        .min_by(|&&a, &&b| distance(a, centre).total_cmp(&distance(b, centre)))
        .expect("more than one placement");
    let at = (packed.placements[first].x_mm, packed.placements[first].y_mm);
    let mut sample = indices;
    sample.sort_by(|&a, &b| distance(a, at).total_cmp(&distance(b, at)).then(a.cmp(&b)));
    sample.truncate(job.args.sample_objects.min(packed.placements.len()));
    Ok(sample)
}

/// The layout with every touching pair moved apart (`gcode_collisions::nudge_apart`), still
/// valid under the current rule; `None` when an object cannot be matched or moved.
pub fn nudge_touching(
    job: &Job,
    plan: &PackPlan,
    packed: &PackResult,
    analysis: &PlateAnalysis,
) -> AppResult<Option<Vec<PackedPlacement>>> {
    let matched = match_placements(&plan.footprint, &packed.placements, &analysis.first_layer)
        .map_err(|error| format!("support check: {error}"))?;
    let mut contacts = Vec::new();
    for collision in &analysis.collisions {
        match (matched.get(&collision.support_object), matched.get(&collision.body_object)) {
            (Some(&a), Some(&b)) => contacts.push((a, b, collision.min_gap_mm)),
            _ => return Ok(None),
        }
    }
    let footprint = plan.rule_footprint();
    let options = pack_options(
        &job.args,
        &job.profile,
        plan.lock_orientation,
        &plan.fixed_keepouts,
        plan.tower_keepout,
    )?;
    let nudged = nudge_apart(&packed.placements, &contacts, |placements| {
        let mut layout = packed.clone();
        layout.placements = placements.to_vec();
        validate_result(&footprint, &options, &layout).is_ok()
    });
    if let Some(nudged) = &nudged {
        log_nudges(&packed.placements, nudged);
    }
    Ok(nudged)
}

fn log_nudges(before: &[PackedPlacement], after: &[PackedPlacement]) {
    for (index, (before, after)) in before.iter().zip(after).enumerate() {
        let shift = (after.x_mm - before.x_mm, after.y_mm - before.y_mm);
        if shift.0.hypot(shift.1) > NUDGE_LOG_THRESHOLD_MM {
            println!(
                "support check: placement {index} moves by ({:+.2}, {:+.2}) mm",
                shift.0, shift.1
            );
        }
    }
}

/// `learn_support_patch` against the current layout, with the context an operator needs.
pub fn learn_support_patch(
    footprint: &Footprint,
    packed: &PackResult,
    analysis: &PlateAnalysis,
) -> AppResult<Footprint> {
    goat_packer::gcode_collisions::learn_support_patch(
        footprint,
        &packed.placements,
        &analysis.first_layer,
    )
    .map_err(|error| format!("support check: {error}").into())
}

pub fn describe_collisions(collisions: &[SupportCollision]) {
    for collision in collisions {
        println!(
            "support check: support of object {} touches object {} on layers {}..{} ({} \
             layers, gap {:.2} mm)",
            collision.support_object,
            collision.body_object,
            collision.first_layer,
            collision.last_layer,
            collision.layers,
            collision.min_gap_mm
        );
    }
}

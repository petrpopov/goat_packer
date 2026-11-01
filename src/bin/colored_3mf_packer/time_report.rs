//! The `time` block of the report: the final plate's corrected time, or — with a time window —
//! the search for the prefix of the plate that fits it.

use goat_packer::{
    PackResult, loops_touch_rect,
    colored_packer::{PrimeTowerBounds, parse_prime_tower_bounds},
    gcode_collisions::{SupportCollision, analyze_plate},
    print_time::TimeSample,
    progress::{self, LayoutSource, Stage},
};

use crate::AppResult;
use crate::finish::{PlateRun, Verified};
use crate::placement::placement_body_loops;
use crate::report::TimeReport;
use crate::slicing::slice_project;
use crate::support_learning::describe_collisions;
use crate::time_window::{fit_window, hours, time_sample};
use crate::tower::{TowerOutcome, evaluate_final_tower};

/// What a time-window plate showed besides its time.
struct WindowPlate {
    tower: Option<PrimeTowerBounds>,
    collisions: Vec<SupportCollision>,
}

/// The time report of the verified plate. With a window the plate may shrink to the kept
/// prefix: `run.packed`, `verified.tower` and `verified.support_collisions` then describe the
/// chosen window plate.
pub fn report_time(run: &mut PlateRun, verified: &mut Verified) -> AppResult<Option<TimeReport>> {
    match (run.job.window_s, run.probe_sample) {
        (Some(window_s), Some(probe_sample)) => {
            window_report(run, verified, window_s, probe_sample).map(Some)
        }
        _ => Ok(single_plate_report(run, verified)),
    }
}

fn single_plate_report(run: &PlateRun, verified: &Verified) -> Option<TimeReport> {
    let job = run.job;
    match &verified.slice.estimate {
        Ok(estimate) => {
            let sample = time_sample(
                run.packed.placements.len(),
                estimate,
                job.slicer_change_s,
                &job.correction,
            );
            Some(TimeReport {
                window_hours: None,
                margin_pct: job.args.time.margin_pct,
                real_change_s: job.correction.real_change_s,
                slicer_change_s: job.slicer_change_s,
                prep_s: job.correction.prep_s,
                samples: vec![sample],
                chosen_count: sample.count,
                corrected_s: sample.corrected_s,
            })
        }
        Err(error) => {
            eprintln!("note: final slice time estimate unavailable ({error}); report has no time");
            None
        }
    }
}

fn window_report(
    run: &mut PlateRun,
    verified: &mut Verified,
    window_s: f64,
    probe_sample: TimeSample,
) -> AppResult<TimeReport> {
    let job = run.job;
    let estimate = verified
        .slice
        .estimate
        .as_ref()
        .map_err(|error| format!("final slice time estimate unavailable: {error}"))?;
    let max_sample = time_sample(
        run.packed.placements.len(),
        estimate,
        job.slicer_change_s,
        &job.correction,
    );
    let full = run.packed.clone();
    let mut last_window_plate: Option<WindowPlate> = None;
    let choice = fit_window(probe_sample, max_sample, window_s, |count| {
        let (sample, plate) = slice_window_prefix(run, &full, count)?;
        last_window_plate = Some(plate);
        Ok(sample)
    })?;
    let mut samples = vec![probe_sample, max_sample];
    let chosen = match choice {
        Some(choice) => {
            run.packed.placements.truncate(choice.count);
            run.show_layout(LayoutSource::Window)?;
            let plate = last_window_plate.expect("a chosen window plate was sliced");
            verified.tower = plate.tower;
            verified.support_collisions = plate.collisions;
            samples.extend(choice.samples);
            *samples.last().expect("window samples are non-empty")
        }
        None => {
            println!(
                "time window {:.1} h: max plate {} objects = {:.1} h corrected; fits",
                hours(window_s),
                max_sample.count,
                hours(max_sample.corrected_s)
            );
            max_sample
        }
    };
    Ok(TimeReport {
        window_hours: job.args.time.window_hours,
        margin_pct: job.args.time.margin_pct,
        real_change_s: job.correction.real_change_s,
        slicer_change_s: job.slicer_change_s,
        prep_s: job.correction.prep_s,
        samples,
        chosen_count: chosen.count,
        corrected_s: chosen.corrected_s,
    })
}

/// Builds and slices the first `count` objects of `full` as a window plate.
fn slice_window_prefix(
    run: &PlateRun,
    full: &PackResult,
    count: usize,
) -> AppResult<(TimeSample, WindowPlate)> {
    let job = run.job;
    // A prefix of a valid layout is valid (same rule as the scheduler's subset_3mf), and
    // packing order puts the kept objects together.
    let mut prefix = full.clone();
    prefix.placements.truncate(count);
    run.plan.build(job, &prefix)?;
    progress::stage(
        Stage::WindowSlice,
        &format!("slicing time-window plate ({count} objects)..."),
        Some(run.expected_slice_seconds(count)),
        Some(count),
    );
    let slice = slice_project(
        &job.profile,
        &job.args.output,
        count,
        &job.args.work_dir,
        &format!("final_window_{count}"),
    )?;
    let tower = parse_prime_tower_bounds(
        &slice.gcode,
        job.profile.tower_line_width_mm,
        job.args.clearance,
    );
    if evaluate_final_tower(run.probe_tower, tower, run.plan.tower_keepout)?
        == TowerOutcome::RepackRequired
    {
        // Fewer objects must not grow the tower; if it did and reaches a kept placement, the
        // layout's collision checks are void. A growth that hits nothing is accepted for the
        // same reason as on the max plate.
        let grown = tower.unwrap().keepout;
        if prefix.placements.iter().any(|placement| {
            loops_touch_rect(&placement_body_loops(&run.plan.footprint, placement), grown)
        }) {
            return Err(format!(
                "prime tower of the {count}-object time-window plate outgrew the reserved \
                 keepout and reaches a placement"
            )
            .into());
        }
    }
    let estimate = slice.estimate.map_err(|error| {
        format!("time-window slice ({count} objects) estimate unavailable: {error}")
    })?;
    // A prefix drops objects, and the slicer re-generates every support: check the plate
    // actually written, report (not repack) what still touches.
    let collisions = analyze_plate(&slice.gcode).collisions;
    describe_collisions(&collisions);
    let sample = time_sample(count, &estimate, job.slicer_change_s, &job.correction);
    Ok((sample, WindowPlate { tower, collisions }))
}

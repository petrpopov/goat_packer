//! The `time` block of the report: the plate's corrected time, or — with a time window — the
//! search for the prefix of the plate that fits it. The window needs the slicer's estimates,
//! so it always verifies (a window ignores the lack of `--verify`).

use goat_packer::{
    PrintPlacement, Rect,
    plate_centre::{loops_bounds, smallest_containing_zone, union_rect},
    progress::{LayoutSource, Stage},
    print_time::TimeSample,
    progress,
};

use crate::AppResult;
use crate::assembly::tower_rect;
use crate::report::TimeReport;
use crate::slicing::slice_project;
use crate::time_window::{fit_window, hours, time_sample};
use crate::verify::{PlateRun, Verified};

/// The time report of the verified plate. With a window the plate may shrink to the kept
/// prefix: `run.state.placements` then describes the chosen window plate.
pub fn report_time(run: &mut PlateRun, verified: &Verified) -> AppResult<Option<TimeReport>> {
    match (run.job.window_s, run.state.probe_sample) {
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
                run.state.placements.len(),
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
    verified: &Verified,
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
        run.state.placements.len(),
        estimate,
        job.slicer_change_s,
        &job.correction,
    );
    let full = run.state.placements.clone();
    let choice = fit_window(probe_sample, max_sample, window_s, |count| {
        slice_window_prefix(run, &full, count)
    })?;
    let mut samples = vec![probe_sample, max_sample];
    let chosen = match choice {
        Some(choice) => {
            run.state.placements.truncate(choice.count);
            run.show_layout(LayoutSource::Window);
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

/// Builds and slices the first `count` objects of `full` as a window plate. A prefix of a
/// valid layout is valid, but dropping objects regenerates supports too: the prefix is checked
/// by the same rules as the whole plate, so a clean full plate is not taken as evidence.
fn slice_window_prefix(
    run: &PlateRun,
    full: &[PrintPlacement],
    count: usize,
) -> AppResult<TimeSample> {
    let job = run.job;
    run.build_plate(&full[..count])?;
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
    let findings = run.check(&slice.gcode, count)?;
    if !findings.is_clean() {
        return Err(format!(
            "the {count}-object time-window plate shows problems: {}",
            findings.describe()
        )
        .into());
    }
    let estimate = slice.estimate.map_err(|error| {
        format!("time-window slice ({count} objects) estimate unavailable: {error}")
    })?;
    let sample = time_sample(count, &estimate, job.slicer_change_s, &job.correction);
    Ok(sample)
}

/// The smallest profile heat zone that holds the kept objects and the tower (window plates
/// only: a max-count plate fills the bed by construction).
pub fn heat_zone(run: &PlateRun) -> Option<String> {
    run.job.window_s?;
    let (state, profile) = (&run.state, &run.job.profile);
    let object_boxes = state.placements.iter().filter_map(|placement| {
        let print = &state.plan.prints[placement.print];
        let loops: Vec<_> = print.silhouette.iter().chain(&print.support_base).cloned().collect();
        let bounds = loops_bounds(&loops)?;
        Some(bounds.translated(placement.x_mm, placement.y_mm))
    });
    let bbox: Rect = object_boxes
        .chain(tower_rect(&state.plan, state.tower_anchor))
        .reduce(union_rect)?;
    let zone = smallest_containing_zone(&profile.heat_zones, bbox)?.name.clone();
    println!(
        "heat zone: {zone} (objects + tower x [{:.1}, {:.1}], y [{:.1}, {:.1}])",
        bbox.min_x, bbox.max_x, bbox.min_y, bbox.max_y
    );
    Some(zone)
}

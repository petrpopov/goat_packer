//! Packs copies of one painted 3MF object onto a printer's bed, without slicing the plate.
//!
//! The pipeline, one module per step:
//! 1. `job`: command line, printer profile, plate rules and the facts read from the source.
//! 2. `model`: per rotation class a few slices of one object (`probe`, cached in
//!    `probe_cache`), turned into the class's print (`class_print`); the prime tower (`tower`).
//! 3. `prepare`: pack through the core's portfolio and order the copies (`order`).
//! 4. `assembly`: write the 3MF; `report` the state and the report.
//! 5. `verify` (only with `--verify`, a time window or `--finish-state`): slice the plate,
//!    check it, learn from a failure (`learn`) and pack again once; `time_report` /
//!    `time_window` fit a time window.

mod args;
mod assembly;
mod class_print;
mod from_model;
mod gaps;
mod job;
mod learn;
mod loops_ops;
mod margins;
mod model;
mod own_gap;
mod order;
mod plan;
mod prepare;
mod probe;
mod probe_cache;
mod report;
mod slicing;
mod source;
#[cfg(test)]
mod test_support;
mod time_report;
mod time_window;
mod tower;
mod verify;

use std::{error::Error, fs};

use args::parse_args;
use job::Job;
use plan::PackState;
use prepare::prepare_plate;
use report::{ColoredPackReport, state_path, write_report, write_state};
use time_report::{heat_zone, report_time};
use verify::PlateRun;

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let job = Job::open(parse_args()?)?;
    // `--finish-state`: the plate was packed by an earlier run; its plan and layout come back
    // from the state file and only the verification runs.
    let (state, verify) = match &job.args.finish_state {
        Some(path) => (load_state(path)?, true),
        None => {
            let state = prepare_plate(&job)?;
            // A time window needs the slices: its estimates come from them.
            let verify = job.args.verify || job.window_s.is_some();
            (state, verify)
        }
    };
    if verify {
        finish_verified(&job, state)
    } else {
        finish_unverified(&job, state)
    }
}

fn load_state(path: &std::path::Path) -> AppResult<PackState> {
    let state = PackState::from_json(&fs::read(path)?)
        .map_err(|error| format!("pack state {}: {error}", path.display()))?;
    println!(
        "verifying a packed plate of {} objects from {}",
        state.placements.len(),
        path.display()
    );
    Ok(state)
}

/// Writes the 3MF, the state and the report of a plate that is not sliced.
fn finish_unverified(job: &Job, state: PackState) -> AppResult<()> {
    let mut run = PlateRun::new(job, state);
    run.build()?;
    let mut report = ColoredPackReport::of_packed_plate(job, &run.state)?;
    write_state(job, &run.state)?;
    report.state_path = Some(state_path(job).display().to_string());
    write_report(job, &report)
}

/// Slices and checks the plate (repacking once when a slice teaches something), then writes
/// the state and the report.
fn finish_verified(job: &Job, state: PackState) -> AppResult<()> {
    let mut run = PlateRun::new(job, state);
    let verified = run.verify()?;
    let time = report_time(&mut run, &verified)?;
    let mut report = ColoredPackReport::of_packed_plate(job, &run.state)?;
    report.repacked = run.repacked;
    report.final_sliced = true;
    report.support_collisions = verified.support_collisions;
    report.verification = Some(verified.verification);
    report.time = time;
    report.heat_zone = heat_zone(&run);
    // The state of the plate as written, so a later check starts from the same layout.
    write_state(job, &run.state)?;
    report.state_path = Some(state_path(job).display().to_string());
    write_report(job, &report)
}

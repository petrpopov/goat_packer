//! Packs copies of one painted 3MF object onto a printer's bed and slices the result.
//!
//! The pipeline, one module per step:
//! 1. `job`: command line, printer profile and the facts read from the source project.
//! 2. `probe`: slice one object; learn the prime tower, the time and the printed footprint.
//! 3. `prepare`: choose the tower position, pack, and learn the supports from a sample slice.
//! 4. `finish`: slice the assembled plate, check tower and supports, repack until clean.
//! 5. `time_report` / `report`: the time window and the verification report.

mod args;
mod assembly;
mod finish;
mod job;
mod placement;
mod plan;
mod prepare;
mod probe;
mod report;
mod slicing;
mod source;
mod support_learning;
#[cfg(test)]
mod test_support;
mod time_report;
mod time_window;
mod tower;

use std::{error::Error, fs};

use args::parse_args;
use finish::PlateRun;
use job::Job;
use plan::PackState;
use prepare::prepare_plate;

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let job = Job::open(parse_args()?)?;
    // `--finish-state`: the plate was packed by an earlier run without the final slice; its
    // plan and layout come back from the state file and only the final slice and its checks
    // run (spec 2026-11-01-sample-slice-layer-zones-design).
    let state = match &job.args.finish_state {
        Some(path) => load_state(path)?,
        None => prepare_plate(&job)?,
    };
    PlateRun::new(&job, state).finish()
}

fn load_state(path: &std::path::Path) -> AppResult<PackState> {
    let state: PackState = serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| format!("pack state {}: {error}", path.display()))?;
    println!(
        "finishing a packed plate of {} objects from {}",
        state.packed.placements.len(),
        path.display()
    );
    Ok(state)
}

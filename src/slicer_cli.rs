//! The one place that builds a slicer CLI invocation, so every slice gets the same guards.
use std::{path::Path, process::Command};

/// `--arrange 0` must never be dropped: without it OrcaSlicer re-arranges the plate while
/// slicing (the probe slice moved the goat to the bed centre, a dense U1 plate silently sliced
/// 25 of 38 objects, 2026-10-11 koza-mini). The packer's layout is then not what gets printed
/// and the final-slice check proves nothing. BambuStudio accepts the same flag
/// (`0-disable, 1-enable, others-auto`), so it is passed for both slicer kinds.
pub fn slice_command(slicer: &Path, extra: &[&str], output_dir: &Path, input: &Path) -> Command {
    let mut command = Command::new(slicer);
    command
        .args(["--arrange", "0"])
        .args(extra)
        .args(["--slice", "1", "--outputdir"])
        .arg(output_dir)
        .arg(input);
    command
}

//! Running the slicer CLI on a project and reading back its G-code and time estimate.

use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

use goat_packer::{
    colored_packer::{PrinterProfile, apply_cli_settings_workarounds},
    gcode_frame::{extruder_offset, gcode_to_plate_frame},
    scheduler::{SliceEstimate, parse_bambu_result, parse_orca_gcode},
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::AppResult;
use crate::source::source_project_settings;

/// One slicer run: the G-code (tower and first-layer parsing) plus the slicer's own time
/// estimate. The estimate is kept as a Result so a plate whose summary cannot be parsed still
/// packs when no time window depends on it.
pub struct SliceOutput {
    pub gcode: Vec<u8>,
    pub estimate: Result<SliceEstimate, String>,
}

/// Slices of the same project whose supports the packer learns from. Tree supports differ
/// between runs (`goat_packer::first_layer_footprint_of`): the union of a few runs stands for
/// the observed variability, not a guaranteed bound on a future slice. Single-core slicing would make
/// them repeatable, but only on the server; the GUI slices on all cores.
const SUPPORT_SLICE_RUNS: usize = 3;

/// Lines of slicer output quoted in a failure message.
const FAILURE_TAIL_LINES: usize = 15;

/// Three independent slices (`label`, `label_2`, ...) run sequentially to limit peak RSS.
pub fn slice_runs(
    profile: &PrinterProfile,
    project: &Path,
    count: usize,
    work_dir: &Path,
    label: &str,
) -> AppResult<Vec<SliceOutput>> {
    collect_support_runs(label, |run_label| {
        slice_project(profile, project, count, work_dir, run_label)
    })
}

fn collect_support_runs<T, E>(
    label: &str,
    mut run: impl FnMut(&str) -> Result<T, E>,
) -> Result<Vec<T>, E> {
    // Preserve all realizations and their union, but bound each job to ONE live slicer.
    // Running three at once multiplied peak RSS before the server queue could limit it.
    (0..SUPPORT_SLICE_RUNS)
        .map(|index| {
            let label = if index == 0 {
                label.to_owned()
            } else {
                format!("{label}_{}", index + 1)
            };
            run(&label)
        })
        .collect()
}

/// Slices `project` (holding `count` objects) into `<work_dir>/<label>_slice` and reads the
/// result in plate coordinates.
pub fn slice_project(
    profile: &PrinterProfile,
    project: &Path,
    count: usize,
    work_dir: &Path,
    label: &str,
) -> AppResult<SliceOutput> {
    let sanitized = work_dir.join(format!("{label}_cli.3mf"));
    let input = if prepare_cli_copy(profile, project, &sanitized)? {
        sanitized
    } else {
        project.to_owned()
    };
    let output_dir = work_dir.join(format!("{label}_slice"));
    if output_dir.exists() {
        fs::remove_dir_all(&output_dir)?;
    }
    fs::create_dir_all(&output_dir)?;
    let extra: &[&str] = if profile.no_check_for_final_tower_probe && label.starts_with("final") {
        &["--no-check"]
    } else {
        &[]
    };
    let mut command = goat_packer::slicer_cli::slice_command(
        Path::new(&profile.slicer_path),
        extra,
        &output_dir,
        &input,
    );
    let output = goat_packer::process_output::bounded_output(&mut command)?;
    let log_path = work_dir.join(format!("{label}_slice.log"));
    fs::write(&log_path, [&output.stdout[..], &output.stderr[..]].concat())?;
    if !output.status.success() {
        return Err(slicing_failure(profile, &command, &output, &log_path, &output_dir).into());
    }
    let gcode_path = fs::read_dir(&output_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "gcode")
        })
        .ok_or("slicer output did not contain G-code")?;
    // Everything below reads the slice in plate coordinates, the frame of the meshes, the bed
    // and the build items (see `gcode_frame`).
    let offset = extruder_offset(&source_project_settings(project)?)?;
    let gcode = gcode_to_plate_frame(&fs::read(gcode_path)?, offset);
    // Same sources as the scheduler's benchmark: BambuStudio writes its summary (with the
    // change count) to result.json, Orca only leaves the estimate in the G-code.
    let estimate = if profile.slicer_kind == "bambu" {
        fs::read(output_dir.join("result.json"))
            .map_err(|error| format!("cannot read result.json: {error}"))
            .and_then(|bytes| parse_bambu_result(&bytes).map_err(|error| error.to_string()))
    } else {
        parse_orca_gcode(&gcode, count).map_err(|error| error.to_string())
    };
    Ok(SliceOutput { gcode, estimate })
}

/// The message of a failed slice. BambuStudio/Orca print their real diagnosis to stdout (and
/// to result.json in the output dir), not to stderr — reporting only stderr yielded an empty
/// "slicing failed:" message on real plates. Surface the most specific source first, plus
/// enough context to re-run the exact failing command by hand.
fn slicing_failure(
    profile: &PrinterProfile,
    command: &std::process::Command,
    output: &std::process::Output,
    log_path: &Path,
    output_dir: &Path,
) -> String {
    let report_error = slicer_report_error(output_dir)
        .unwrap_or_else(|| "<slicer wrote no result.json>".to_owned());
    // Rendered from the Command itself: a hand-written copy dropped `--arrange 0`, and a
    // rerun without it lets Orca re-arrange the plate, i.e. reproduces a different slice.
    let rerun = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{} slicing failed ({})\n\
         slicer report: {report_error}\n\
         slicer output (tail):\n{}\n\
         full log: {}\n\
         output dir: {}\n\
         rerun manually: {rerun}",
        profile.printer_name,
        output.status,
        output_tail(&output.stdout, &output.stderr, FAILURE_TAIL_LINES),
        log_path.display(),
        output_dir.display(),
    )
}

/// Reads the slicer's own `result.json` from the output dir and extracts its `error_string` —
/// the single most useful line when a slice fails (e.g. "Some objects are located over the
/// boundary of the heated bed."). Returns `None` when the file or field is absent.
fn slicer_report_error(output_dir: &Path) -> Option<String> {
    let report = fs::read_dir(output_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.file_name().is_some_and(|name| name == "result.json"))?;
    let value: serde_json::Value = serde_json::from_slice(&fs::read(report).ok()?).ok()?;
    let error = value
        .get("error_string")
        .and_then(serde_json::Value::as_str)?
        .to_owned();
    (!error.is_empty()).then_some(error)
}

/// Last `max_lines` non-empty lines of the slicer's combined stdout+stderr, each indented, for
/// embedding in a failure message. Keeps `[error] ...` lines that BambuStudio logs mid-run.
fn output_tail(stdout: &[u8], stderr: &[u8], max_lines: usize) -> String {
    let combined = [stdout, stderr].concat();
    let text = String::from_utf8_lossy(&combined);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..]
        .iter()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rewrites `project` into `output` with CLI-slicing workarounds applied, returning whether a
/// modified copy was actually written (callers should slice the original file directly when
/// nothing needed changing). Two independent fixes are applied to `Metadata/project_settings.config`:
/// blanking `machine_start_gcode` when the profile requires it, and resizing `flush_volumes_matrix`
/// to `filament_colour.len()^2` when they've drifted apart. The latter is a stale-data artifact seen
/// on "single_*" projects trimmed down from a multi-color template: `filament_colour` shrinks to the
/// filaments actually in use, but `flush_volumes_matrix` is left sized for the old (larger) count.
/// OrcaSlicer's CLI rejects that outright ("Flush volumes matrix do not match to the correct
/// size!") even on a project whose object never references a second filament, so the mismatched
/// values are never actually read during slicing — only the size needs to be correct.
fn prepare_cli_copy(profile: &PrinterProfile, project: &Path, output: &Path) -> AppResult<bool> {
    let mut source = ZipArchive::new(fs::File::open(project)?)?;
    let mut entries = Vec::with_capacity(source.len());
    let mut changed = false;
    for index in 0..source.len() {
        let mut entry = source.by_index(index)?;
        let name = entry.name().to_owned();
        let is_dir = entry.is_dir();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        if name == "Metadata/project_settings.config" {
            let mut settings: serde_json::Value = serde_json::from_slice(&bytes)?;
            changed = apply_cli_settings_workarounds(profile, &mut settings);
            bytes = serde_json::to_vec(&settings)?;
        }
        entries.push((name, is_dir, bytes));
    }
    if !changed {
        return Ok(false);
    }
    let mut archive = ZipWriter::new(fs::File::create(output)?);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, is_dir, bytes) in entries {
        if is_dir {
            archive.add_directory(name, options)?;
        } else {
            archive.start_file(name, options)?;
            archive.write_all(&bytes)?;
        }
    }
    archive.finish()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_probes_keep_all_three_realizations_and_stop_on_failure() {
        let mut seen = Vec::new();
        let runs: Result<Vec<_>, ()> = collect_support_runs("probe", |label| {
            seen.push(label.to_owned());
            Ok(seen.len())
        });
        assert_eq!(runs.unwrap(), vec![1, 2, 3]);
        assert_eq!(seen, vec!["probe", "probe_2", "probe_3"]);
        let mut calls = 0;
        let failed = collect_support_runs("sample", |_| {
            calls += 1;
            if calls == 2 { Err("failed") } else { Ok(calls) }
        });
        assert!(failed.is_err());
        assert_eq!(calls, 2);
    }

    #[test]
    fn output_tail_keeps_the_last_non_empty_lines_of_combined_slicer_output() {
        let stdout = b"trace line 1\ntrace line 2\n\n[error] plate 1: boom\n";
        let stderr = b"";
        let tail = output_tail(stdout, stderr, 2);
        assert_eq!(tail, "  trace line 2\n  [error] plate 1: boom");
    }

    #[test]
    fn slicer_report_error_reads_error_string_from_result_json() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(slicer_report_error(dir.path()), None);
        fs::write(
            dir.path().join("result.json"),
            r#"{"error_string":"Some objects are located over the boundary of the heated bed.","return_code":-52}"#,
        )
        .unwrap();
        assert_eq!(
            slicer_report_error(dir.path()).as_deref(),
            Some("Some objects are located over the boundary of the heated bed.")
        );
    }
}

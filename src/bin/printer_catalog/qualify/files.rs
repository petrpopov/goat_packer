//! Files and processes of a qualification run: work directories, logged commands, the probe
//! slice's G-code and the report rows.

use std::{fs, path::Path};

use goat_packer::{
    orca_project::{failure_line, find_file},
    process_output::bounded_output,
};

use crate::AppResult;

const REPORT_HEADER: &str =
    "| vendor | printer_model | id | result | reason | seconds |\n|---|---|---|---|---|---|\n";

pub fn reset_dir(dir: &Path) -> std::io::Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)
}

pub fn join_numbers(values: &[f64]) -> String {
    values
        .iter()
        .map(f64::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// Runs the command with stdout+stderr saved to `log`; a non-zero exit becomes an error carrying
/// the last meaningful output line (the packer prints `error: …` last).
pub fn run_logged(command: &mut std::process::Command, log: &Path, what: &str) -> AppResult<()> {
    let output = bounded_output(command)
        .map_err(|e| format!("{what}: cannot start {:?}: {e}", command.get_program()))?;
    fs::write(log, [&output.stdout[..], &output.stderr[..]].concat())?;
    if output.status.success() {
        return Ok(());
    }
    let reason = failure_line(&output.stderr)
        .or_else(|| failure_line(&output.stdout))
        .unwrap_or_default();
    Err(format!("{what} failed ({}): {reason}", output.status).into())
}

/// Plain `plate_1.gcode` (Orca and BambuStudio CLI with `--slice 1 --outputdir`), else a G-code
/// inside a `.gcode.3mf`.
pub fn read_gcode(dir: &Path) -> AppResult<String> {
    if let Some(path) = find_file(dir, |n| n.ends_with(".gcode")) {
        return Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned());
    }
    let packaged =
        find_file(dir, |n| n.ends_with(".gcode.3mf")).ok_or("probe slice wrote no G-code")?;
    let mut zip = zip::ZipArchive::new(fs::File::open(packaged)?)?;
    let name = zip
        .file_names()
        .filter(|n| n.ends_with(".gcode"))
        .min()
        .map(str::to_owned)
        .ok_or("no G-code inside the .gcode.3mf")?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut zip.by_name(&name)?, &mut text)?;
    Ok(text)
}

pub fn report_row(
    vendor: &str,
    model: &str,
    id: &str,
    result: &str,
    reason: &str,
    seconds: f64,
) -> String {
    let cell = |s: &str| s.replace('|', "\\|").replace(['\n', '\r'], " ");
    format!(
        "| {} | {} | {} | {} | {} | {seconds:.0} |\n",
        cell(vendor),
        cell(model),
        cell(id),
        result,
        cell(reason)
    )
}

/// One `write` per row with O_APPEND: rows from parallel containers do not interleave.
pub fn append_report(path: &Path, row: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let fresh = !path.exists();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let text = if fresh {
        format!("{REPORT_HEADER}{row}")
    } else {
        row.to_owned()
    };
    file.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_rows_escape_cells_and_get_one_header() {
        let row = report_row(
            "Acme",
            "Box | 2",
            "acme_box",
            "failed",
            "line1\nline2",
            12.4,
        );
        assert_eq!(
            row,
            "| Acme | Box \\| 2 | acme_box | failed | line1 line2 | 12 |\n"
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.md");
        append_report(&path, &row).unwrap();
        append_report(&path, &row).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("| vendor |").count(), 1);
        assert_eq!(text.lines().count(), 4);
    }

    #[test]
    fn gcode_is_read_plain_or_from_a_gcode_3mf() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("plate_1.gcode"), ";LAYER_CHANGE\n").unwrap();
        assert_eq!(read_gcode(dir.path()).unwrap(), ";LAYER_CHANGE\n");
        let dir = tempfile::tempdir().unwrap();
        let mut zip =
            zip::ZipWriter::new(fs::File::create(dir.path().join("plate_1.gcode.3mf")).unwrap());
        zip.start_file(
            "Metadata/plate_1.gcode",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, b"G1 X1\n").unwrap();
        zip.finish().unwrap();
        assert_eq!(read_gcode(dir.path()).unwrap(), "G1 X1\n");
        assert!(read_gcode(tempfile::tempdir().unwrap().path()).is_err());
    }
}

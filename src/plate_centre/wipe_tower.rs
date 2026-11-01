//! The one project-settings edit the packer makes: moving the wipe tower of plate 1.

use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const PROJECT_SETTINGS_ENTRY: &str = "Metadata/project_settings.config";

/// Formats a millimetre setting the way BambuStudio writes them: up to three decimals, no
/// trailing zeros ("164.898", "160").
fn format_setting_mm(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

fn first_entry(settings: &serde_json::Value, key: &str) -> Result<f64, String> {
    let entry = settings
        .get(key)
        .and_then(serde_json::Value::as_array)
        .and_then(|values| values.first())
        .ok_or_else(|| format!("project settings have no {key}[0] to relocate the tower"))?;
    let text = entry
        .as_str()
        .ok_or_else(|| format!("project setting {key}[0] is not a string: {entry}"))?;
    text.trim()
        .parse::<f64>()
        .map_err(|error| format!("project setting {key}[0] = {text:?} is not a number: {error}"))
}

/// Adds `shift` to `wipe_tower_x[0]`/`wipe_tower_y[0]` (arrays of strings, one entry per
/// plate) and returns the new position. Only entry 0 changes: the packer's output is one
/// plate, and every other entry belongs to a plate the packer does not touch (see
/// `scheduler::patch_wipe_tower_per_plate` for what those entries mean). On error the settings
/// are left untouched.
pub fn shift_wipe_tower_settings(
    settings: &mut serde_json::Value,
    shift: (f64, f64),
) -> Result<(f64, f64), String> {
    let x = first_entry(settings, "wipe_tower_x")? + shift.0;
    let y = first_entry(settings, "wipe_tower_y")? + shift.1;
    settings["wipe_tower_x"][0] = serde_json::Value::String(format_setting_mm(x));
    settings["wipe_tower_y"][0] = serde_json::Value::String(format_setting_mm(y));
    Ok((x, y))
}

/// Rewrites `project` in place with the wipe tower of plate 1 shifted by `shift`. Every entry
/// other than `project_settings.config` is copied byte-for-byte; the settings JSON is
/// re-serialized (keys alphabetized, the same as `prepare_cli_copy` and nozzle-clean injection
/// already do — BambuStudio reads a settings map, order carries no meaning).
pub fn relocate_wipe_tower_in_3mf(project: &Path, shift: (f64, f64)) -> Result<(f64, f64), String> {
    let context = |error: &dyn std::fmt::Display| format!("{}: {error}", project.display());
    let mut archive = ZipArchive::new(fs::File::open(project).map_err(|e| context(&e))?)
        .map_err(|e| context(&e))?;
    let mut entries: Vec<(String, bool, Vec<u8>)> = Vec::with_capacity(archive.len());
    let mut moved = None;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| context(&e))?;
        let name = entry.name().to_owned();
        let is_dir = entry.is_dir();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| context(&e))?;
        if name == PROJECT_SETTINGS_ENTRY {
            let mut settings: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| context(&e))?;
            moved = Some(shift_wipe_tower_settings(&mut settings, shift)?);
            bytes = serde_json::to_vec(&settings).map_err(|e| context(&e))?;
        }
        entries.push((name, is_dir, bytes));
    }
    drop(archive);
    let moved =
        moved.ok_or_else(|| format!("{} has no {PROJECT_SETTINGS_ENTRY}", project.display()))?;
    let mut zip = ZipWriter::new(fs::File::create(project).map_err(|e| context(&e))?);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, is_dir, bytes) in entries {
        if is_dir {
            zip.add_directory(name.trim_end_matches('/'), options)
                .map_err(|e| context(&e))?;
        } else {
            zip.start_file(&name, options).map_err(|e| context(&e))?;
            zip.write_all(&bytes).map_err(|e| context(&e))?;
        }
    }
    zip.finish().map_err(|e| context(&e))?;
    Ok(moved)
}

/// The stored tower position of plate 1 (`wipe_tower_x[0]`, `wipe_tower_y[0]`).
pub fn wipe_tower_anchor(settings: &serde_json::Value) -> Result<(f64, f64), String> {
    Ok((
        first_entry(settings, "wipe_tower_x")?,
        first_entry(settings, "wipe_tower_y")?,
    ))
}

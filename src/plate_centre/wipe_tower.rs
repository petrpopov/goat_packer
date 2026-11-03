//! The one project-settings edit the packer makes: moving the wipe tower of plate 1.

use std::path::Path;

use crate::project_settings_edit::{edit_project_settings_in_3mf, format_setting_mm};

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

/// Rewrites `project` in place with the wipe tower of plate 1 shifted by `shift` (the rest of
/// the archive as `edit_project_settings_in_3mf` leaves it).
pub fn relocate_wipe_tower_in_3mf(project: &Path, shift: (f64, f64)) -> Result<(f64, f64), String> {
    edit_project_settings_in_3mf(project, |settings| shift_wipe_tower_settings(settings, shift))
}

/// The stored tower position of plate 1 (`wipe_tower_x[0]`, `wipe_tower_y[0]`).
pub fn wipe_tower_anchor(settings: &serde_json::Value) -> Result<(f64, f64), String> {
    Ok((
        first_entry(settings, "wipe_tower_x")?,
        first_entry(settings, "wipe_tower_y")?,
    ))
}

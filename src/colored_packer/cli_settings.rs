//! CLI-slicing workarounds on a project's `project_settings.config`.

use super::PrinterProfile;

/// CLI-slicing workarounds on a project's `project_settings.config`, shared by every binary
/// that slices a project copy (`colored_3mf_packer`'s `prepare_cli_copy`, `ams_calibration`).
/// Blanks `machine_start_gcode` when the profile requires it and resizes a stale
/// `flush_volumes_matrix` to `filament_colour.len()^2`: a "single_*" project trimmed down from a
/// multi-colour template keeps the old larger matrix, and OrcaSlicer's CLI rejects the mismatch
/// outright ("Flush volumes matrix do not match to the correct size!") even when no second
/// filament is ever used. Returns whether anything changed.
pub fn apply_cli_settings_workarounds(
    profile: &PrinterProfile,
    settings: &mut serde_json::Value,
) -> bool {
    let mut changed = false;
    if profile.blank_start_gcode_for_cli {
        settings["machine_start_gcode"] = serde_json::Value::String(String::new());
        changed = true;
    }
    if normalize_flush_volumes_matrix(settings) {
        changed = true;
    }
    changed
}

/// Resizes `settings["flush_volumes_matrix"]` to `filament_colour.len()^2` entries when it doesn't
/// already match, reusing values from the overlapping top-left block of the old matrix (row-major,
/// square) and filling any newly-added cells with `"0"`. Returns `false` (no-op) when either field
/// is missing/not an array or the size already matches.
pub fn normalize_flush_volumes_matrix(settings: &mut serde_json::Value) -> bool {
    let Some(filament_count) = settings
        .get("filament_colour")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
    else {
        return false;
    };
    let Some(matrix) = settings
        .get("flush_volumes_matrix")
        .and_then(serde_json::Value::as_array)
        .cloned()
    else {
        return false;
    };
    let expected_len = filament_count * filament_count;
    if matrix.len() == expected_len {
        return false;
    }
    let old_side = (matrix.len() as f64).sqrt().round() as usize;
    let old_side = if old_side * old_side == matrix.len() {
        old_side
    } else {
        0
    };
    let zero = serde_json::Value::String("0".to_owned());
    let mut resized = Vec::with_capacity(expected_len);
    for row in 0..filament_count {
        for col in 0..filament_count {
            let value = if row < old_side && col < old_side {
                matrix[row * old_side + col].clone()
            } else {
                zero.clone()
            };
            resized.push(value);
        }
    }
    settings["flush_volumes_matrix"] = serde_json::Value::Array(resized);
    true
}

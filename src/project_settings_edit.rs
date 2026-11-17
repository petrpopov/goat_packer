//! The only way the packer edits `Metadata/project_settings.config` of a finished 3MF: every
//! other entry is copied byte for byte, the settings JSON is re-serialized (keys alphabetized,
//! as `prepare_cli_copy` and nozzle-clean injection already do; the slicers read a settings
//! map, order carries no meaning).

use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const PROJECT_SETTINGS_ENTRY: &str = "Metadata/project_settings.config";

/// Formats a millimetre setting the way BambuStudio writes them: up to three decimals, no
/// trailing zeros ("164.898", "160").
pub fn format_setting_mm(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// The preset a setting belongs to, as `different_settings_to_system` lists them: one
/// `;`-separated key list per slot, the print first, then one per filament, the printer last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetSlot {
    Print,
    Printer,
}

/// Lists `key` as changed from the system preset in its slot of `different_settings_to_system`.
///
/// Every value the packer forces into a project must be listed, or the GUIs drop it: on loading
/// a project Bambu Studio and the Orca family give every key their slot does not list the
/// system preset's value (the shared preset loader replaces the "non-different" values by the
/// base preset's). Seen 2026-11-17 on the owner's P1S plate in Bambu Studio 02.09: the packer
/// wrote `prime_tower_rib_wall` 0 unlisted, the GUI turned it back to 1, sized the tower as a
/// ~36 mm rib square, clamped the anchor 12 mm lower than the packer reserved and the tower
/// conflicted with a goat. The list is kept sorted and without repeats, as the GUIs write it.
pub fn mark_changed_from_system(settings: &mut serde_json::Value, slot: PresetSlot, key: &str) {
    let filaments = settings
        .get("filament_colour")
        .and_then(serde_json::Value::as_array)
        .map_or(1, |colours| colours.len().max(1));
    let Some(map) = settings.as_object_mut() else {
        return;
    };
    let entry = map
        .entry("different_settings_to_system")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if let serde_json::Value::String(single) = entry {
        *entry = serde_json::Value::Array(vec![serde_json::Value::String(single.clone())]);
    }
    let Some(slots) = entry.as_array_mut() else {
        return;
    };
    if slots.len() < filaments + 2 {
        slots.resize(filaments + 2, serde_json::Value::String(String::new()));
    }
    let index = match slot {
        PresetSlot::Print => 0,
        PresetSlot::Printer => slots.len() - 1,
    };
    let current = slots[index].as_str().unwrap_or_default();
    let mut keys: Vec<&str> = current.split(';').filter(|k| !k.trim().is_empty()).collect();
    keys.push(key);
    keys.sort_unstable();
    keys.dedup();
    slots[index] = serde_json::Value::String(keys.join(";"));
}

/// Rewrites `project` in place after `edit` changed its settings JSON, and returns what `edit`
/// returned. On an `edit` error the file is left untouched.
pub fn edit_project_settings_in_3mf<T>(
    project: &Path,
    edit: impl FnOnce(&mut serde_json::Value) -> Result<T, String>,
) -> Result<T, String> {
    let context = |error: &dyn std::fmt::Display| format!("{}: {error}", project.display());
    let mut archive = ZipArchive::new(fs::File::open(project).map_err(|e| context(&e))?)
        .map_err(|e| context(&e))?;
    let mut entries: Vec<(String, bool, Vec<u8>)> = Vec::with_capacity(archive.len());
    let mut edit = Some(edit);
    let mut result = None;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| context(&e))?;
        let name = entry.name().to_owned();
        let is_dir = entry.is_dir();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| context(&e))?;
        if name == PROJECT_SETTINGS_ENTRY
            && let Some(edit) = edit.take()
        {
            let mut settings: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| context(&e))?;
            result = Some(edit(&mut settings)?);
            bytes = serde_json::to_vec(&settings).map_err(|e| context(&e))?;
        }
        entries.push((name, is_dir, bytes));
    }
    drop(archive);
    let result =
        result.ok_or_else(|| format!("{} has no {PROJECT_SETTINGS_ENTRY}", project.display()))?;
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
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_forced_key_joins_its_slot_sorted_once() {
        let mut settings = json!({
            "filament_colour": ["#FFFFFF", "#000000"],
            "different_settings_to_system": ["seam_position;wall_loops", "", "", "machine_start_gcode"],
        });
        mark_changed_from_system(&mut settings, PresetSlot::Print, "prime_tower_rib_wall");
        mark_changed_from_system(&mut settings, PresetSlot::Print, "prime_tower_rib_wall");
        mark_changed_from_system(&mut settings, PresetSlot::Printer, "layer_change_gcode");
        assert_eq!(
            settings["different_settings_to_system"],
            json!([
                "prime_tower_rib_wall;seam_position;wall_loops",
                "",
                "",
                "layer_change_gcode;machine_start_gcode"
            ])
        );
    }

    #[test]
    fn a_project_without_the_list_gets_one_slot_per_preset() {
        let mut settings = json!({"filament_colour": ["#FFFFFF", "#000000", "#FF0000"]});
        mark_changed_from_system(&mut settings, PresetSlot::Printer, "layer_change_gcode");
        assert_eq!(
            settings["different_settings_to_system"],
            json!(["", "", "", "", "layer_change_gcode"])
        );
    }
}

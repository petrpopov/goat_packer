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

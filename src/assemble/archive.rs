//! The 3MF archive as an ordered list of entries.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

pub(super) const CORE_MODEL_ENTRY: &str = "3D/3dmodel.model";
pub(super) const MODEL_SETTINGS_ENTRY: &str = "Metadata/model_settings.config";
pub(super) const PROJECT_SETTINGS_ENTRY: &str = "Metadata/project_settings.config";
pub(super) const CUT_INFORMATION_ENTRY: &str = "Metadata/cut_information.xml";
pub(super) const PLATE_JSON_ENTRY: &str = "Metadata/plate_1.json";
pub(super) const RELATIONSHIPS_ENTRY: &str = "3D/_rels/3dmodel.model.rels";
pub(super) const LAYER_HEIGHTS_ENTRY: &str = "Metadata/layer_heights_profile.txt";

pub(super) type Entries = Vec<(String, Vec<u8>)>;

fn entry_index(entries: &Entries, name: &str) -> Option<usize> {
    entries
        .iter()
        .position(|(entry_name, _)| entry_name == name)
}

pub(super) fn entry<'a>(entries: &'a Entries, name: &str) -> Option<&'a [u8]> {
    entry_index(entries, name).map(|index| entries[index].1.as_slice())
}

pub(super) fn entry_bytes<'a>(entries: &'a Entries, name: &str) -> Result<&'a [u8], String> {
    entry(entries, name).ok_or_else(|| format!("template does not contain {name:?}"))
}

/// Replaces an entry in place — preserving the template's entry order, which the zip writer
/// and downstream diffing rely on — or appends it when the template lacks it.
pub(super) fn set_entry(entries: &mut Entries, name: &str, bytes: Vec<u8>) {
    match entry_index(entries, name) {
        Some(index) => entries[index].1 = bytes,
        None => entries.push((name.to_owned(), bytes)),
    }
}

/// Reads every template entry in archive order. The rewrite must keep that order (contract in
/// the spec: the level is cosmetic, the order is not — new entries are appended last).
pub(super) fn read_template_entries(template: &Path) -> Result<Entries, String> {
    let mut archive = ZipArchive::new(
        File::open(template)
            .map_err(|error| format!("cannot open template {template:?}: {error}"))?,
    )
    .map_err(|error| format!("{template:?} is not a readable 3MF archive: {error}"))?;
    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("cannot read template entry {index}: {error}"))?;
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read template entry {name:?}: {error}"))?;
        entries.push((name, bytes));
    }
    Ok(entries)
}

pub(super) fn write_archive(output: &Path, entries: &Entries) -> Result<(), String> {
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let mut archive = ZipWriter::new(
        File::create(output).map_err(|error| format!("cannot create {output:?}: {error}"))?,
    );
    // Deflate level 6 matches the Python artifact's shape; the level itself is cosmetic, the
    // entry order is not (see `read_template_entries`).
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6));
    for (name, bytes) in entries {
        archive
            .start_file(name.as_str(), options)
            .map_err(|error| format!("cannot write zip entry {name:?}: {error}"))?;
        archive
            .write_all(bytes)
            .map_err(|error| format!("cannot write zip entry {name:?}: {error}"))?;
    }
    archive
        .finish()
        .map_err(|error| format!("cannot finish {output:?}: {error}"))?;
    Ok(())
}

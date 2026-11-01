//! A 3MF archive as a list of `(name, is_directory, bytes)` entries, in archive order.

use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use super::SchedulerResult;

pub(super) type Entries = Vec<(String, bool, Vec<u8>)>;

pub(super) fn read_zip_entries(path: &Path) -> SchedulerResult<Entries> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        entries.push((entry.name().to_owned(), entry.is_dir(), data));
    }
    Ok(entries)
}

pub(super) fn write_zip_entries(path: &Path, entries: Entries) -> SchedulerResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = ZipWriter::new(File::create(path)?);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, is_dir, data) in entries {
        if is_dir {
            writer.add_directory(name, options)?;
        } else {
            writer.start_file(name, options)?;
            writer.write_all(&data)?;
        }
    }
    writer.finish()?;
    Ok(())
}

pub(super) fn entry_index(entries: &Entries, name: &str) -> SchedulerResult<usize> {
    entries
        .iter()
        .position(|entry| entry.0 == name)
        .ok_or_else(|| format!("archive entry not found: {name}").into())
}

/// One text entry of the archive, read straight from the file.
pub(super) fn read_entry(source: &Path, name: &str) -> SchedulerResult<Vec<u8>> {
    let mut archive = ZipArchive::new(File::open(source)?)?;
    let mut data = Vec::new();
    archive.by_name(name)?.read_to_end(&mut data)?;
    Ok(data)
}

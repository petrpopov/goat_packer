//! Reading the source 3MF the plate is built from.

use std::{
    fs,
    io::Read,
    path::Path,
};

use goat_packer::{
    assemble::source_model_center,
    colored_packer::{PrinterProfile, detect_slicer_kind_from_printer_model},
    embedded_mesh::{embedded_mesh_triangles, stl_center_3d, write_binary_stl},
    orca_project,
};
use zip::ZipArchive;

use crate::AppResult;

/// `Metadata/project_settings.config` of the source project.
pub fn source_project_settings(source: &Path) -> AppResult<serde_json::Value> {
    Ok(serde_json::Value::Object(orca_project::project_settings(source)?))
}

/// The project was made for the printer the profile describes, and the profile's slicer is the
/// one that printer needs.
pub fn validate_source_printer(source: &Path, profile: &PrinterProfile) -> AppResult<()> {
    let configured = source_project_settings(source)?;
    let printer = configured
        .get("printer_model")
        .and_then(serde_json::Value::as_str)
        .ok_or("source project has no printer_model")?;
    if printer != profile.printer_name {
        return Err(format!(
            "source printer '{printer}' does not match profile '{}'",
            profile.printer_name
        )
        .into());
    }
    match detect_slicer_kind_from_printer_model(printer) {
        Some(detected) if detected != profile.slicer_kind => {
            return Err(format!(
                "printer_model '{printer}' implies slicer '{detected}', but profile '{}' is \
                 configured for slicer '{}'",
                profile.id, profile.slicer_kind
            )
            .into());
        }
        Some(_) => {}
        None => println!(
            "note: no known slicer mapping for printer_model '{printer}'; trusting profile \
             slicer_kind '{}'",
            profile.slicer_kind
        ),
    }
    Ok(())
}

/// How many `paint_color` attributes the source mesh carries (the colouring the plate must keep).
pub fn paint_attribute_count(source: &Path) -> AppResult<usize> {
    let mut archive = ZipArchive::new(fs::File::open(source)?)?;
    let mut root = Vec::new();
    archive.by_name("3D/3dmodel.model")?.read_to_end(&mut root)?;
    let text = String::from_utf8_lossy(&root);
    let marker = "p:path=\"/";
    let start = text.find(marker).ok_or("source core has no component path")? + marker.len();
    let end = text[start..]
        .find('"')
        .map(|offset| start + offset)
        .ok_or("component path is unterminated")?;
    let path = &text[start..end];
    let mut mesh = Vec::new();
    archive.by_name(path)?.read_to_end(&mut mesh)?;
    Ok(String::from_utf8_lossy(&mesh).matches("paint_color").count())
}

/// Writes the collision STL: the 3MF's own embedded mesh with the component's transform baked
/// in (see the call site in `run` for why a separately exported STL cannot be trusted).
pub fn derive_embedded_stl(source_3mf: &Path, output_stl: &Path) -> AppResult<()> {
    let triangles = embedded_mesh_triangles(source_3mf).map_err(|error| {
        format!("failed to derive collision STL from embedded mesh: {error}")
    })?;
    // The Python exporter refused to write an STL with no triangles; keep that loud failure.
    if triangles.is_empty() {
        return Err(
            "failed to derive collision STL from embedded mesh: embedded mesh has no triangles"
                .into(),
        );
    }
    write_binary_stl(output_stl, &triangles).map_err(|error| {
        format!("failed to derive collision STL from embedded mesh: {error}")
    })?;
    Ok(())
}

/// The source model's bbox center in bed coordinates: the baked STL's center transformed by
/// the source 3MF's build item. Falls back to None (patch centers itself) when the archive
/// cannot be read — the probe slice still packs, just with the strip-center caveat.
pub fn model_center_on_bed(source_3mf: &Path, baked_stl: &Path) -> Option<(f64, f64)> {
    let (cx, cy, cz) = stl_center_3d(baked_stl).ok()?;
    let core = read_core_entry(source_3mf).ok()?;
    source_model_center(&core, (cx, cy, cz)).ok()
}

fn read_core_entry(source_3mf: &Path) -> AppResult<Vec<u8>> {
    let file = fs::File::open(source_3mf)?;
    let mut archive = ZipArchive::new(file)?;
    let mut entry = archive.by_name("3D/3dmodel.model")?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use goat_packer::colored_packer::load_printer_profile;

    #[test]
    fn validate_source_printer_rejects_slicer_kind_mismatch() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profiles/corner_bed.json");
        let mut profile = load_printer_profile(&fixture).unwrap();
        // The fixture project's own printer_model.
        profile.printer_name = "Bambu Lab P1S".to_owned();
        let source = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/bambu.3mf"));
        validate_source_printer(source, &profile).unwrap();

        let mut mismatched = profile.clone();
        mismatched.slicer_kind = "orca".to_owned();
        let error = validate_source_printer(source, &mismatched).unwrap_err();
        assert!(error.to_string().contains("slicer"));
    }
}

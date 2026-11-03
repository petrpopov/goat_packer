//! Reading the source 3MF the plate is built from.

use std::{
    fs,
    io::Read,
    path::Path,
};

use goat_packer::{
    colored_packer::{PrinterProfile, detect_slicer_kind_from_printer_model},
    embedded_mesh::{embedded_mesh_triangles, write_binary_stl},
    orca_project,
};
use zip::ZipArchive;

use crate::AppResult;

const CORE_MODEL_ENTRY: &str = "3D/3dmodel.model";
const APPLICATION_TAG: &str = "<metadata name=\"Application\">";

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

/// Writes the collision STL `verify_outputs` reads as its second opinion: the 3MF's own
/// embedded mesh with the component's transform baked in. The packer itself collides against
/// `Job::mesh`, which comes from the same file.
pub fn derive_embedded_stl(source_3mf: &Path, output_stl: &Path) -> AppResult<()> {
    let triangles = embedded_mesh_triangles(source_3mf)
        .map_err(|error| format!("failed to derive the embedded STL: {error}"))?;
    if triangles.is_empty() {
        return Err("failed to derive the embedded STL: the embedded mesh has no triangles".into());
    }
    write_binary_stl(output_stl, &triangles)
        .map_err(|error| format!("failed to derive the embedded STL: {error}"))?;
    Ok(())
}

/// One entry of the source archive, byte for byte.
pub fn source_entry(source: &Path, name: &str) -> AppResult<Vec<u8>> {
    let mut archive = ZipArchive::new(fs::File::open(source)?)?;
    let mut bytes = Vec::new();
    archive
        .by_name(name)
        .map_err(|error| format!("{} has no {name}: {error}", source.display()))?
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The `Application` metadata of the source's core model (the GUI that saved the project).
pub fn source_application(source: &Path) -> Option<String> {
    let model = source_entry(source, CORE_MODEL_ENTRY).ok()?;
    let text = String::from_utf8_lossy(&model);
    let start = text.find(APPLICATION_TAG)? + APPLICATION_TAG.len();
    let end = text[start..].find("</metadata>")? + start;
    let value = quick_xml::escape::unescape(text[start..end].trim()).ok()?;
    (!value.is_empty()).then(|| value.into_owned())
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

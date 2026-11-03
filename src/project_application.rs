//! Which slicer GUI a project belongs to: the `Application` metadata of `3D/3dmodel.model`
//! (with Orca's `OrcaSlicer` version tag, `support_model::flavour::project_generator`) and the
//! re-stamp `source_project --gui-flavour` applies so the GUI the user prints with opens an
//! exported STL project as its own. BambuStudio and its forks load `project_settings.config`
//! only from a 3MF whose `Application` they recognise (CLAUDE.md, AMS calibration), and the
//! packer resolves its target GUI from the same metadata.
use std::{fs, io::Read as _, path::Path};

use crate::orca_project::{Error, rewrite_entries};
use crate::support_model::flavour::{Family, Flavour, project_generator};

const MODEL_ENTRY: &str = "3D/3dmodel.model";
const APPLICATION: &str = "Application";
/// OrcaSlicer's own version tag (2.3.2 on), written right after `Application`.
const ORCA_TAG: &str = "OrcaSlicer";

/// The text of `<metadata name="{name}">…</metadata>` in a model XML, unescaped and trimmed.
pub fn model_metadata(xml: &str, name: &str) -> Option<String> {
    let (_, range) = metadata_element(xml, name)?;
    let open = format!("<metadata name=\"{name}\">");
    let start = range.start + open.len();
    let end = range.end - "</metadata>".len();
    let value = quick_xml::escape::unescape(xml[start..end].trim()).ok()?;
    (!value.is_empty()).then(|| value.into_owned())
}

/// The generator of a model XML (`project_generator` over `Application` and the Orca tag).
pub fn model_generator(xml: &str) -> Option<String> {
    let application = model_metadata(xml, APPLICATION)?;
    Some(project_generator(&application, model_metadata(xml, ORCA_TAG).as_deref()))
}

/// The generator of a project 3MF, `None` without a readable `Application`.
pub fn project_application(project: &Path) -> Option<String> {
    let mut archive = zip::ZipArchive::new(fs::File::open(project).ok()?).ok()?;
    let mut xml = String::new();
    archive.by_name(MODEL_ENTRY).ok()?.read_to_string(&mut xml).ok()?;
    model_generator(&xml)
}

/// The model XML stamped for the `target` GUI. A BambuStudio target gets BambuStudio's own
/// `Application` and loses the Orca tag (else the project still reads as Orca's export); a
/// Snapmaker Orca target gets `Snapmaker_Orca-2.4.0` (what the owner's U1 projects carry) and
/// keeps the Orca tag: Snapmaker Orca 2.4.0 has no such tag and ignores it, while the vanilla
/// Orca CLI that slices U1 probes on the server reads the project as its own export through it.
/// A vanilla Orca target needs no stamp but must come from an Orca export: we do not invent
/// Orca's compatibility string for a BambuStudio project. `None` = leave the XML as it is.
pub fn stamped_model(xml: &str, target: Flavour) -> Result<Option<String>, Error> {
    let generator = model_generator(xml).ok_or("exported project has no Application metadata")?;
    let exporter = Flavour::from_application(&generator)
        .map(|(flavour, _)| flavour)
        .ok_or_else(|| format!("exported project names an unknown application {generator:?}"))?;
    match target.family() {
        Family::Orca if exporter.family() != Family::Orca => Err(format!(
            "an OrcaSlicer project needs the OrcaSlicer export; this one was written by {generator}"
        )
        .into()),
        Family::Orca => Ok(None),
        Family::Bambu => {
            let application = target.own_application().expect("BambuStudio has its own stamp");
            let xml = without_metadata(xml, ORCA_TAG);
            Ok(Some(with_metadata(&xml, APPLICATION, application)?))
        }
        Family::SnapmakerOrca => {
            if exporter.family() == Family::Bambu {
                return Err(format!(
                    "a Snapmaker Orca project needs the OrcaSlicer export; this one was written by {generator}"
                )
                .into());
            }
            let application = target.own_application().expect("Snapmaker Orca has its own stamp");
            Ok(Some(with_metadata(xml, APPLICATION, application)?))
        }
    }
}

/// Stamps a project 3MF in place for the `target` GUI (`stamped_model`); returns the project's
/// generator afterwards.
pub fn stamp_project(project: &Path, target: Flavour) -> Result<String, Error> {
    let mut stamped = false;
    rewrite_entries(project, |name, bytes| {
        if name != MODEL_ENTRY {
            return Ok(bytes);
        }
        stamped = true;
        let xml = String::from_utf8(bytes)?;
        Ok(stamped_model(&xml, target)?.unwrap_or(xml).into_bytes())
    })?;
    if !stamped {
        return Err(format!("{} has no {MODEL_ENTRY}", project.display()).into());
    }
    project_application(project).ok_or_else(|| "stamped project lost its Application".into())
}

/// The element `<metadata name="{name}">…</metadata>` with its line (leading indent and the
/// trailing newline) and the element's own byte range.
fn metadata_element(xml: &str, name: &str) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let open = format!("<metadata name=\"{name}\">");
    let start = xml.find(&open)?;
    let end = xml[start..].find("</metadata>")? + start + "</metadata>".len();
    let line_start = xml[..start].rfind('\n').map_or(0, |at| at + 1);
    let line_start = if xml[line_start..start].trim().is_empty() { line_start } else { start };
    let line_end = if xml[end..].starts_with('\n') { end + 1 } else { end };
    Some((line_start..line_end, start..end))
}

fn with_metadata(xml: &str, name: &str, value: &str) -> Result<String, Error> {
    let (_, element) = metadata_element(xml, name).ok_or_else(|| format!("exported project has no {name} metadata"))?;
    let escaped = quick_xml::escape::escape(value);
    Ok(format!("{}<metadata name=\"{name}\">{escaped}</metadata>{}", &xml[..element.start], &xml[element.end..]))
}

fn without_metadata(xml: &str, name: &str) -> String {
    match metadata_element(xml, name) {
        Some((line, _)) => format!("{}{}", &xml[..line.start], &xml[line.end..]),
        None => xml.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The head of a model XML as OrcaSlicer 2.4.2 exports it (`--export-3mf`).
    const ORCA_EXPORT: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\">\n \
        <metadata name=\"Application\">BambuStudio-02.06.00.51</metadata>\n \
        <metadata name=\"OrcaSlicer\">2.4.2</metadata>\n \
        <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n <resources/>\n</model>\n";
    const BAMBU_EXPORT: &str = "<model>\n <metadata name=\"Application\">BambuStudio-02.08.02.61</metadata>\n \
        <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n</model>\n";

    #[test]
    fn an_orca_export_is_read_as_orca() {
        assert_eq!(model_generator(ORCA_EXPORT).as_deref(), Some("OrcaSlicer-2.4.2"));
        assert_eq!(model_generator(BAMBU_EXPORT).as_deref(), Some("BambuStudio-02.08.02.61"));
        assert_eq!(model_generator("<model/>"), None);
    }

    #[test]
    fn a_vanilla_orca_target_keeps_the_orca_export_and_refuses_a_bambu_one() {
        assert_eq!(stamped_model(ORCA_EXPORT, Flavour::Orca242).unwrap(), None);
        assert!(stamped_model(BAMBU_EXPORT, Flavour::Orca242).is_err());
    }

    #[test]
    fn a_snapmaker_target_gets_snapmakers_application_and_keeps_the_orca_tag() {
        let stamped = stamped_model(ORCA_EXPORT, Flavour::Snapmaker240).unwrap().unwrap();
        assert!(stamped.contains(" <metadata name=\"Application\">Snapmaker_Orca-2.4.0</metadata>\n"));
        assert!(stamped.contains("<metadata name=\"OrcaSlicer\">2.4.2</metadata>"));
        let generator = model_generator(&stamped).unwrap();
        assert_eq!(Flavour::from_application(&generator), Some((Flavour::Snapmaker240, true)));
        assert!(stamped_model(BAMBU_EXPORT, Flavour::Snapmaker240).is_err());
    }

    #[test]
    fn a_bambu_target_gets_bambus_application_without_the_orca_tag() {
        let stamped = stamped_model(ORCA_EXPORT, Flavour::Bambu0208).unwrap().unwrap();
        assert!(!stamped.contains("OrcaSlicer"));
        assert!(stamped.contains("<metadata name=\"BambuStudio:3mfVersion\">1</metadata>"));
        assert_eq!(model_generator(&stamped).as_deref(), Some("BambuStudio-02.08.02.61"));
        // BambuStudio's own export is already its own.
        assert_eq!(stamped_model(BAMBU_EXPORT, Flavour::Bambu0208).unwrap().as_deref(), Some(BAMBU_EXPORT));
    }

    #[test]
    fn stamping_a_project_rewrites_only_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("p.3mf");
        {
            use std::io::Write as _;
            let mut zip = zip::ZipWriter::new(fs::File::create(&project).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file(MODEL_ENTRY, options).unwrap();
            zip.write_all(ORCA_EXPORT.as_bytes()).unwrap();
            zip.start_file("Metadata/project_settings.config", options).unwrap();
            zip.write_all(b"{\"printer_model\":\"Acme\"}").unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(stamp_project(&project, Flavour::Snapmaker240).unwrap(), "Snapmaker_Orca-2.4.0");
        assert_eq!(project_application(&project).as_deref(), Some("Snapmaker_Orca-2.4.0"));
        let settings = crate::orca_project::project_settings(&project).unwrap();
        assert_eq!(settings["printer_model"], "Acme");
    }
}

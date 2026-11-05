//! Everything one run of the packer is given and derives once at startup: the command line,
//! the printer profile, the plate rules and the facts read from the source project.

use std::fs;

use goat_packer::{
    PlateRules,
    assemble::{ItemFrame, source_item_transform_of},
    colored_packer::{
        PrinterProfile,
        bed_rules::{GuiFlavour, ProjectBed, plate_rules},
        load_printer_profile,
    },
    embedded_mesh::{ObjectMesh, load_object_mesh},
    first_layer::{ProjectSettings, SlicerBuild},
    plate_centre::wipe_tower_anchor,
    print_time::{TimeCorrection, slicer_change_seconds},
    scheduler::object_count_3mf,
    support_model::flavour::{Flavour, ResolvedFlavour},
    tower_model::force_tower_shape,
};

use crate::AppResult;
use crate::args::Args;
use crate::gaps::OwnGap;
use crate::source::{
    source_application, source_entry, source_project_settings, validate_source_printer,
};
use crate::time_window::window_seconds;

const CORE_MODEL_ENTRY: &str = "3D/3dmodel.model";
const PROJECT_SETTINGS_ENTRY: &str = "Metadata/project_settings.config";

pub struct Job {
    pub args: Args,
    pub profile: PrinterProfile,
    /// The plate's zones in the plate frame (G-code-frame keepouts already moved by the
    /// project's `extruder_offset`).
    pub rules: PlateRules,
    /// `project_settings.config` byte for byte (with the forced tower shape, when it changed
    /// anything): the support probes depend on all of it.
    pub settings_bytes: Vec<u8>,
    pub settings: ProjectSettings,
    /// The slicer GUI the plate is opened in (the project's own, else the profile's).
    pub flavour: ResolvedFlavour,
    pub build: SlicerBuild,
    /// Every part of the source object with its item transform applied (design §4.1).
    pub mesh: ObjectMesh,
    pub frame: ItemFrame,
    /// The source's stored `wipe_tower_x/y`; `None` without a tower setting.
    pub tower_anchor: Option<(f64, f64)>,
    /// The project's `extruder_offset`: plate = G-code + this.
    pub gcode_offset: (f64, f64),
    /// The project's support-to-own-model gaps, read once here: the probes slice with them and
    /// the output 3MF copies them (`gaps::check_output`).
    pub own_gap: OwnGap,
    pub window_s: Option<f64>,
    /// The source project's per-change assumption, read once: the assembled output copies
    /// project_settings.config byte-for-byte, so it holds for every plate sliced.
    pub slicer_change_s: f64,
    pub correction: TimeCorrection,
}

impl Job {
    pub fn open(args: Args) -> AppResult<Job> {
        let mut profile = load_printer_profile(&args.profile_path)?;
        if let Some(kind) = &args.slicer {
            profile = profile.for_slicer(kind)?;
        }
        apply_edge_margin(&mut profile, args.edge_margin);
        validate_source_printer(&args.source_3mf, &profile)?;
        let source_count = object_count_3mf(&args.source_3mf)?;
        if source_count != 1 {
            return Err(format!(
                "source project must contain exactly one painted object; found {source_count}"
            )
            .into());
        }
        let mut settings_value = source_project_settings(&args.source_3mf)?;
        let flavour = resolve_flavour(&args, &profile)?;
        if let Some(warning) = flavour_warning(&args, &flavour) {
            // A plain line like the gap-floor warning; the report repeats it (`warnings`).
            eprintln!("warning: {warning}");
        }
        let build = SlicerBuild::from_name(flavour.flavour.name())
            .ok_or_else(|| format!("no slicer build for flavour {}", flavour.flavour.name()))?;
        // The tower the GUI prints is the one the output 3MF asks for: the model, the probe
        // copies and the output all read the forced shape (`tower::apply_tower_shape`).
        let tower_forced = args
            .tower_width
            .is_some_and(|width| force_tower_shape(&mut settings_value, build, width));
        if tower_forced {
            eprintln!("{}", tower_note(build, args.tower_width.unwrap_or_default()));
        }
        let settings = ProjectSettings::from_json(&settings_value);
        // The GUI's own acceptance band (Snapmaker Orca's 3.5 mm) belongs to the target GUI.
        profile.gui_flavour = Some(GuiFlavour::try_from(flavour.flavour.name().to_owned())?);
        let bed = ProjectBed::from_settings(&settings_value)?;
        let rules = plate_rules(&profile, &bed)?;
        let own_gap = OwnGap::decide(&settings_value, args.keep_own_gap)?;
        if let Some(note) = own_gap.forced_note() {
            eprintln!("{note}");
        }
        let slicer_change_s = slicer_change_seconds(&settings_value);
        let correction = args.time.correction(&profile);
        if args.time.ams_correction && correction.real_change_s.is_none() {
            eprintln!(
                "note: profile '{}' has no filament_change_seconds (tool changer); AMS change \
                 correction is off",
                profile.id
            );
        }
        let window_s = args.time.window_hours.map(window_seconds);
        fs::create_dir_all(&args.work_dir)?;
        // The collision geometry is the mesh the 3MF embeds, never a separate STL: a drifted
        // STL silently invalidated collisions (a real near-zero-clearance touch between two
        // placed goats on a P1S plate).
        let mesh = load_object_mesh(&args.source_3mf)?;
        let core = source_entry(&args.source_3mf, CORE_MODEL_ENTRY)?;
        let frame = ItemFrame {
            source: source_item_transform_of(&core)?,
            origin: (mesh.frame_origin[0], mesh.frame_origin[1]),
        };
        let settings_bytes = if tower_forced {
            serde_json::to_vec_pretty(&settings_value)?
        } else {
            source_entry(&args.source_3mf, PROJECT_SETTINGS_ENTRY)?
        };
        Ok(Job {
            rules,
            settings_bytes,
            settings,
            flavour,
            build,
            mesh,
            frame,
            tower_anchor: wipe_tower_anchor(&settings_value).ok(),
            gcode_offset: bed.gcode_offset,
            own_gap,
            window_s,
            slicer_change_s,
            correction,
            profile,
            args,
        })
    }
}

/// The warning of a project whose slicer version has no calibration of its own: the support
/// envelope then takes the widest margins of the family (`ResolvedFlavour::margins`).
pub fn flavour_warning(args: &Args, flavour: &ResolvedFlavour) -> Option<String> {
    flavour.approximated.then(|| {
        let application = source_application(&args.source_3mf).unwrap_or_default();
        format!(
            "slicer version {application} not calibrated; using the widest margins (as {})",
            flavour.flavour.name()
        )
    })
}

/// The progress line of a forced tower.
fn tower_note(build: SlicerBuild, width_mm: f64) -> String {
    let rib = if build == SlicerBuild::BambuStudio { ", rib wall off" } else { "" };
    format!("prime tower forced to {width_mm} mm wide{rib} (--tower-width keep leaves the project's)")
}

/// `--edge-margin` replaces the profile's margin for the model body: the old field and, in a
/// profile with `bed_rules`, its body band, which is what the plate rules read.
fn apply_edge_margin(profile: &mut PrinterProfile, edge_margin: Option<f64>) {
    let Some(margin) = edge_margin else {
        return;
    };
    profile.edge_margin_mm = margin;
    if let Some(rules) = profile.bed_rules.as_mut() {
        rules.body_edge_band_mm = margin;
    }
}

/// The GUI the owner opens the plate in: with `--slicer`, that slicer's GUI from the profile's
/// `slicers` (explicit: the user said what they print with); else the project's
/// `Application`, else the profile's `gui_flavour`, else the family of its `slicer_kind`
/// (design §5.3).
fn resolve_flavour(args: &Args, profile: &PrinterProfile) -> AppResult<ResolvedFlavour> {
    let application = source_application(&args.source_3mf);
    let by_kind = match profile.slicer_kind.as_str() {
        "bambu" => Flavour::NEWEST_BAMBU,
        _ => Flavour::Orca242,
    };
    let profile_name = profile
        .gui_flavour
        .as_ref()
        .map_or_else(|| by_kind.name().to_owned(), ToString::to_string);
    let explicit = args.slicer.as_ref().map(|_| profile_name.as_str());
    Ok(Flavour::resolve(
        explicit,
        application.as_deref(),
        Some(&profile_name),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture_job;
    use goat_packer::support_model::flavour::FlavourSource;

    #[test]
    fn an_uncalibrated_slicer_version_warns_and_names_itself() {
        let dir = tempfile::tempdir().unwrap();
        let job = fixture_job(dir.path());
        let mut flavour = ResolvedFlavour {
            flavour: Flavour::NEWEST_BAMBU,
            source: FlavourSource::Application,
            approximated: false,
        };
        assert_eq!(flavour_warning(&job.args, &flavour), None);
        flavour.approximated = true;
        let warning = flavour_warning(&job.args, &flavour).unwrap();
        // The fixture project was saved by BambuStudio 02.00.00.00.
        assert!(warning.starts_with("slicer version BambuStudio-02.00.00.00 not calibrated; using the widest margins"), "{warning}");
    }
}

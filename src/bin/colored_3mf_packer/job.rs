//! Everything one run of the packer is given and derives once at startup: the command line,
//! the printer profile and facts read from the source project.

use std::{fs, path::PathBuf};

use goat_packer::{
    colored_packer::{PrinterProfile, load_printer_profile},
    gcode_frame::extruder_offset,
    print_time::{TimeCorrection, slicer_change_seconds},
    scheduler::object_count_3mf,
};

use crate::AppResult;
use crate::args::Args;
use crate::source::{derive_embedded_stl, source_project_settings, validate_source_printer};
use crate::time_window::window_seconds;
use crate::tower::{GuiTowerRange, gui_tower_range};

pub struct Job {
    pub args: Args,
    pub profile: PrinterProfile,
    /// The collision mesh: the 3MF's own embedded mesh, baked into an STL in the work dir.
    pub footprint_stl: PathBuf,
    pub gui_tower: Option<GuiTowerRange>,
    pub window_s: Option<f64>,
    /// The source project's per-change assumption, read once: the assembled output copies
    /// project_settings.config byte-for-byte, so it holds for every plate sliced.
    pub slicer_change_s: f64,
    pub correction: TimeCorrection,
}

impl Job {
    pub fn open(args: Args) -> AppResult<Job> {
        let mut profile = load_printer_profile(&args.profile_path)?;
        if let Some(edge_margin) = args.edge_margin {
            profile.edge_margin_mm = edge_margin;
        }
        validate_source_printer(&args.source_3mf, &profile)?;
        // The profile's purge lines come from the start G-code, i.e. G-code coordinates; slices
        // are read in plate coordinates (see `gcode_frame`), so the lines move by the same offset.
        let settings = source_project_settings(&args.source_3mf)?;
        let gcode_offset = extruder_offset(&settings)?;
        for line in &mut profile.purge_lines {
            line[0] += gcode_offset.0;
            line[1] += gcode_offset.1;
            line[2] += gcode_offset.0;
            line[3] += gcode_offset.1;
        }
        let slicer_change_s = slicer_change_seconds(&settings);
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
        let source_count = object_count_3mf(&args.source_3mf)?;
        if source_count != 1 {
            return Err(format!(
                "source project must contain exactly one painted object; found {source_count}"
            )
            .into());
        }
        // Collide against the exact mesh the 3MF embeds, not a separately-provided STL: an
        // independently exported STL can drift from the embedded mesh (different export pass,
        // tool version, or manual edit) while keeping the same overall bounding box, silently
        // invalidating every collision check that assumes the two are identical. This
        // reproduced as a real near-zero-clearance touch between two placed Goat 3 objects on a
        // real P1S plate: the standalone STL's keyring-hole position did not match the embedded
        // mesh's.
        let footprint_stl = args.work_dir.join("embedded_source.stl");
        derive_embedded_stl(&args.source_3mf, &footprint_stl)?;
        let gui_tower = gui_tower_range(&args.source_3mf, &profile, &footprint_stl)?;
        Ok(Job {
            args,
            profile,
            footprint_stl,
            gui_tower,
            window_s,
            slicer_change_s,
            correction,
        })
    }
}

//! Shared fixtures of the unit tests.

use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

use goat_packer::colored_packer::PrinterProfile;

use crate::args::{Args, LayoutMode, SAMPLE_OBJECTS, TimeOptions};

fn write_binary_stl(path: &Path, triangles: &[[[f32; 3]; 3]]) {
    let mut f = File::create(path).unwrap();
    f.write_all(&[0u8; 80]).unwrap();
    f.write_all(&(triangles.len() as u32).to_le_bytes())
        .unwrap();
    for tri in triangles {
        f.write_all(&[0u8; 12]).unwrap();
        for vertex in tri {
            for value in vertex {
                f.write_all(&value.to_le_bytes()).unwrap();
            }
        }
        f.write_all(&[0u8; 2]).unwrap();
    }
}

pub fn square_stl(dir: &Path) -> PathBuf {
    let path = dir.join("square.stl");
    let tris = [
        [[0., 0., 0.], [4., 0., 0.], [4., 4., 0.]],
        [[0., 0., 0.], [4., 4., 0.], [0., 4., 0.]],
        [[0., 0., 1.], [4., 0., 1.], [4., 4., 1.]],
    ];
    write_binary_stl(&path, &tris);
    path
}

pub fn test_profile() -> PrinterProfile {
    PrinterProfile {
        id: "test".into(),
        printer_name: "Test Printer".into(),
        slicer_kind: "bambu".into(),
        slicer_path: "/bin/true".into(),
        blank_start_gcode_for_cli: false,
        no_check_for_final_tower_probe: false,
        printable_area: [0.0, 0.0, 20.0, 20.0],
        edge_margin_mm: 0.0,
        support_edge_margin_mm: None,
        exclusion_rectangles: vec![],
        purge_lines: vec![],
        tower_marker_style: "wipe_tower".into(),
        tower_line_width_mm: 0.4,
        nozzle_clean_gcode: None,
        filament_change_seconds: None,
        heat_zones: vec![],
    }
}

/// Small, fast packing settings working in `work_dir`.
pub fn test_args(work_dir: &Path) -> Args {
    Args {
        source_3mf: PathBuf::from("unused.3mf"),
        profile_path: PathBuf::from("unused.json"),
        output: PathBuf::from("unused_output.3mf"),
        work_dir: work_dir.to_path_buf(),
        clearance: 0.5,
        resolution: 0.5,
        angle_step: 90.0,
        attempts: 8,
        candidates: 100,
        stall: 0,
        max_objects: 0,
        seed: 1,
        edge_margin: None,
        layout: LayoutMode::Search,
        nozzle_clean: None,
        time: TimeOptions::default(),
        support_overlap_mm: None,
        sample_objects: SAMPLE_OBJECTS,
        final_slice: true,
        finish_state: None,
    }
}

pub fn test_job(dir: &Path) -> crate::job::Job {
    let mut args = test_args(dir);
    args.output = dir.join("packed.3mf");
    let profile = test_profile();
    let correction = args.time.correction(&profile);
    crate::job::Job {
        args,
        profile,
        footprint_stl: square_stl(dir),
        gui_tower: None,
        window_s: None,
        slicer_change_s: 0.0,
        correction,
    }
}

pub fn test_plan(job: &crate::job::Job) -> crate::plan::PackPlan {
    crate::plan::PackPlan {
        footprint: goat_packer::extract_footprint(&job.footprint_stl).unwrap(),
        lock_orientation: false,
        fixed_keepouts: vec![],
        tower_keepout: None,
        relocation: None,
        rule: crate::plan::SupportRule::Silhouette,
        patch_learned: false,
        mesh_silhouette: vec![],
        support_edge_reserve_mm: 0.0,
    }
}

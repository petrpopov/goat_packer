//! Shared fixtures of the unit tests.

use std::path::{Path, PathBuf};

use goat_packer::{
    Point, Rect,
    assemble::ItemFrame,
    colored_packer::PrinterProfile,
    embedded_mesh::{ObjectMesh, Part, PartKind},
    first_layer::{ProjectSettings, SlicerBuild},
    keepout::{BodyRule, PlateRules},
    object_print::ObjectPrint,
    support_model::flavour::{Flavour, FlavourSource, ResolvedFlavour},
};

use crate::args::{Args, TimeOptions};
use crate::job::Job;
use crate::plan::{PackPlan, PackState, PackSummary, STATE_VERSION};

pub fn test_profile() -> PrinterProfile {
    PrinterProfile {
        id: "test".into(),
        printer_name: "Test Printer".into(),
        slicer_kind: "bambu".into(),
        slicer_path: "/bin/true".into(),
        blank_start_gcode_for_cli: false,
        no_check_for_final_tower_probe: false,
        printable_area: [0.0, 0.0, 60.0, 60.0],
        edge_margin_mm: 0.0,
        support_edge_margin_mm: None,
        exclusion_rectangles: vec![],
        purge_lines: vec![],
        tower_marker_style: "wipe_tower".into(),
        tower_line_width_mm: 0.4,
        nozzle_clean_gcode: None,
        filament_change_seconds: None,
        heat_zones: vec![],
        bed_rules: None,
        gui_flavour: None,
    }
}

/// Small, fast packing settings working in `work_dir`.
pub fn test_args(work_dir: &Path) -> Args {
    Args {
        source_3mf: PathBuf::from("unused.3mf"),
        profile_path: PathBuf::from("unused.json"),
        output: work_dir.join("packed.3mf"),
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
        nozzle_clean: None,
        time: TimeOptions::default(),
        rotations: vec![0.0, 180.0],
        cache_dir: None,
        verify: false,
        support_overlap_mm: None,
        finish_state: None,
    }
}

/// A 10 x 10 x 5 mm box as the one normal part of an object.
pub fn box_mesh() -> ObjectMesh {
    let vertices: Vec<[f64; 3]> = (0..8)
        .map(|corner| {
            [
                f64::from(corner & 1) * 10.0 - 5.0,
                f64::from(corner >> 1 & 1) * 10.0 - 5.0,
                f64::from(corner >> 2 & 1) * 5.0,
            ]
        })
        .collect();
    let quads = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
    let triangles = quads
        .iter()
        .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
        .collect();
    ObjectMesh {
        parts: vec![Part {
            kind: PartKind::Normal,
            vertices,
            triangles,
            ..Part::default()
        }],
        frame_origin: [30.0, 30.0, 0.0],
    }
}

pub fn test_job(dir: &Path) -> Job {
    let args = test_args(dir);
    let profile = test_profile();
    let correction = args.time.correction(&profile);
    let mut rules = PlateRules {
        printable_area: Rect::new(0.0, 0.0, 60.0, 60.0),
        body_edge_band_mm: 0.0,
        print_edge_reserve_mm: 0.0,
        keepouts: vec![],
        body_rule: BodyRule::Silhouette,
    };
    rules.print_edge_reserve_mm = 0.0;
    Job {
        args,
        profile,
        rules,
        settings_bytes: Vec::new(),
        settings: ProjectSettings::from_json(&serde_json::json!({
            "layer_height": "0.2",
            "initial_layer_print_height": "0.2",
        })),
        flavour: ResolvedFlavour {
            flavour: Flavour::Bambu0208,
            source: FlavourSource::Profile,
            approximated: false,
        },
        build: SlicerBuild::BambuStudio,
        mesh: box_mesh(),
        frame: ItemFrame {
            source: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 30.0, 30.0, 0.0],
            origin: (30.0, 30.0),
        },
        tower_anchor: None,
        gcode_offset: (0.0, 0.0),
        window_s: None,
        slicer_change_s: 0.0,
        correction,
    }
}

fn square(side: f64) -> Vec<Vec<Point>> {
    let h = side / 2.0;
    let p = |x: f64, y: f64| Point { x, y };
    vec![vec![p(-h, -h), p(h, -h), p(h, h), p(-h, h), p(-h, -h)]]
}

/// A plan of `classes` square prints with no supports, on the test job's plate, no tower.
pub fn test_plan(job: &Job, classes: usize) -> PackPlan {
    let prints = (0..classes)
        .map(|class| ObjectPrint {
            rotation_deg: 180.0 * class as f64,
            silhouette: square(10.0),
            landing: square(10.0),
            part_hulls: square(10.0),
            part_boxes: vec![Rect::new(-5.0, -5.0, 5.0, 5.0)],
            body: vec![square(10.0)],
            ..ObjectPrint::default()
        })
        .collect();
    PackPlan {
        prints,
        rules: job.rules.clone(),
        tower: None,
    }
}

/// A job opened on the checked-in P1S fixture project: one object whose component and build
/// item both carry a translation, on a 256 mm bed. `dir` holds its profile and output.
pub fn fixture_job(dir: &Path) -> Job {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut profile: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixtures.join("profiles/corner_bed.json")).unwrap())
            .unwrap();
    profile["printer_name"] = "Bambu Lab P1S".into();
    let profile_path = dir.join("profile.json");
    std::fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
    let mut args = test_args(dir);
    args.source_3mf = fixtures.join("bambu.3mf");
    args.profile_path = profile_path;
    Job::open(args).unwrap()
}

/// A state with no layout yet: `arrange` (or a test) fills the placements.
pub fn test_state(plan: PackPlan) -> PackState {
    PackState {
        version: STATE_VERSION,
        plan,
        placements: Vec::new(),
        tower_anchor: None,
        centre: None,
        probe_seconds: 0.0,
        probe_sample: None,
        summary: PackSummary::default(),
    }
}

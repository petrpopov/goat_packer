//! Shared fixtures of the unit tests.

use std::path::{Path, PathBuf};

use goat_packer::{
    ForeignGaps, Point, Rect,
    assemble::ItemFrame,
    colored_packer::PrinterProfile,
    embedded_mesh::{ObjectMesh, Part, PartKind},
    first_layer::{ProjectSettings, SlicerBuild},
    keepout::{BodyRule, PlateRules},
    object_print::ObjectPrint,
    support_model::flavour::{Flavour, FlavourSource, ResolvedFlavour},
};

use crate::args::{Args, TimeOptions};
use crate::gaps::OwnGap;
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
        slicers: Default::default(),
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
        resolutions: Vec::new(),
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
        support_source: Default::default(),
        support_orders: vec![goat_packer::support_model::merge::MergeOrder::Along],
        support_margins: None,
        verify: false,
        support_overlap_mm: None,
        // One rung, the library defaults: the plain single-pack behaviour of the tests.
        foreign_gaps: vec![ForeignGaps::default()],
        foreign_gap_floor: ForeignGaps::default(),
        // The fixtures carry no preset bookkeeping: tests keep their project's own gaps.
        keep_own_gap: true,
        finish_state: None,
        slicer: None,
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
        support_clip_mm: 0.0,
        support_clip_sides: [true; 4],
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
        own_gap: OwnGap::user(0.2, 0.35),
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
        foreign_gaps: ForeignGaps::default(),
    }
}

/// A job opened on the checked-in P1S fixture project: one object whose component and build
/// item both carry a translation, on a 256 mm bed. `dir` holds its profile and output.
pub fn fixture_job(dir: &Path) -> Job {
    fixture_job_of(dir, "bambu.3mf", true)
}

/// The same project with a 0.4 mm nozzle and no `different_settings_to_system`: the user never
/// touched the own support gap, so the job forces ours (`keep_own_gap` off).
pub fn untouched_gap_job(dir: &Path) -> Job {
    let source = dir.join("untouched.3mf");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::copy(fixtures.join("bambu.3mf"), &source).unwrap();
    goat_packer::project_settings_edit::edit_project_settings_in_3mf(&source, |settings| {
        settings["nozzle_diameter"] = serde_json::json!(["0.4"]);
        Ok(())
    })
    .unwrap();
    fixture_job_of(dir, source.to_str().unwrap(), false)
}

fn fixture_job_of(dir: &Path, source: &str, keep_own_gap: bool) -> Job {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut profile: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixtures.join("profiles/corner_bed.json")).unwrap())
            .unwrap();
    profile["printer_name"] = "Bambu Lab P1S".into();
    let profile_path = dir.join("profile.json");
    std::fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
    let mut args = test_args(dir);
    args.source_3mf = fixtures.join(source);
    args.profile_path = profile_path;
    args.keep_own_gap = keep_own_gap;
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

//! First-layer kinds of work package I3 on generated meshes: the landing (spec 05 §4: slice
//! plane h0/2, multi-part union, ε_L per wall generator, refused settings) and the object
//! brim (spec 03 §5: auto width per build, thin islands, quantisation, ring extent).

use goat_packer::embedded_mesh::{ObjectMesh, Part, PartKind};
use goat_packer::first_layer::{
    CLASSIC_OVERSHOOT_MM, ProjectSettings, SlicerBuild, brim, landing, quantised,
};
use goat_packer::{Point, Rect, point_in_footprint, polygon_area};
use serde_json::json;

// ---------------------------------------------------------------------------------------
// Generated meshes (outward winding, plate coordinates).

/// A square frustum centred on the origin: half-size `bottom` at z = 0, `top` at `height`.
fn frustum(bottom: f64, top: f64, height: f64) -> Part {
    let corner = |half: f64, z: f64| -> [[f64; 3]; 4] {
        [
            [-half, -half, z],
            [half, -half, z],
            [half, half, z],
            [-half, half, z],
        ]
    };
    let mut vertices = corner(bottom, 0.0).to_vec();
    vertices.extend(corner(top, height));
    let mut triangles = vec![[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7]];
    for i in 0..4_u32 {
        let j = (i + 1) % 4;
        triangles.push([i, j, 4 + j]);
        triangles.push([i, 4 + j, 4 + i]);
    }
    Part {
        vertices,
        triangles,
        ..Part::default()
    }
}

/// Axis-aligned box from `min` with `size`.
fn cuboid(min: [f64; 3], size: [f64; 3]) -> Part {
    let mut part = frustum(0.5, 0.5, 1.0);
    for vertex in &mut part.vertices {
        for axis in 0..3 {
            let unit = if axis == 2 {
                vertex[2]
            } else {
                vertex[axis] + 0.5
            };
            vertex[axis] = min[axis] + unit * size[axis];
        }
    }
    part
}

fn mesh(parts: Vec<Part>) -> ObjectMesh {
    ObjectMesh::from_plate_parts(parts).unwrap()
}

/// The owner projects' first-layer settings (classic walls, 0.2 mm first layer).
fn settings(overrides: serde_json::Value) -> ProjectSettings {
    let mut base = json!({
        "initial_layer_print_height": "0.2",
        "layer_height": "0.28",
        "wall_generator": "classic",
        "slice_closing_radius": "0.049",
        "nozzle_diameter": ["0.4"],
        "initial_layer_line_width": "0.5",
        "brim_type": "auto_brim",
        "brim_width": "5",
        "brim_object_gap": "0.1",
        "filament_type": ["PLA", "PLA"],
        "outer_wall_speed": ["200"],
        "inner_wall_speed": ["200"],
        "sparse_infill_speed": ["200"],
        "internal_solid_infill_speed": ["200"],
        "top_surface_speed": ["200"],
        "support_speed": ["150"],
        "small_perimeter_speed": ["50%"],
    });
    for (key, value) in overrides.as_object().unwrap() {
        base[key] = value.clone();
    }
    ProjectSettings::from_json(&base)
}

fn bounds(loops: &[Vec<Point>]) -> Rect {
    Rect::of_points(loops.iter().flatten())
}

fn area(loops: &[Vec<Point>]) -> f64 {
    loops.iter().map(|ring| polygon_area(ring)).sum()
}

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

// ---------------------------------------------------------------------------------------
// Landing.

/// The section plane is the middle of layer 0: a frustum narrowing by 0.5 mm per mm puts the
/// outline at 10 − 0.5·h0/2, grown by ε_L (the disc polygons add at most 0.002 mm).
#[test]
fn landing_is_the_section_at_half_the_first_layer_grown_by_the_overshoot() {
    let object = mesh(vec![frustum(10.0, 5.0, 10.0)]);
    for first_layer in [0.2, 0.25, 0.3] {
        let settings = settings(json!({"initial_layer_print_height": first_layer.to_string()}));
        let printed = landing(&object, &settings, SlicerBuild::BambuStudio, 0.0).unwrap();
        let expected = 10.0 - 0.5 * first_layer / 2.0 + CLASSIC_OVERSHOOT_MM;
        let box_ = bounds(&printed.loops);
        assert!(near(printed.slice_z_mm, first_layer / 2.0, 1e-12));
        assert!(
            box_.max_x >= expected - 1e-6 && box_.max_x <= expected + 0.003,
            "{first_layer}: {box_:?}"
        );
        assert!(box_.min_y <= -expected + 1e-6 && box_.min_y >= -expected - 0.003);
        assert!(printed.measured);
    }
}

/// Every normal part's section is united; a part above the first layer adds nothing; a
/// negative part later in volume order removes material.
#[test]
fn landing_unites_the_normal_parts_and_drops_negative_volumes() {
    let left = cuboid([-10.0, -5.0, 0.0], [10.0, 10.0, 5.0]);
    let right = cuboid([0.0, -5.0, 0.0], [10.0, 10.0, 8.0]);
    let floating = cuboid([-30.0, -5.0, 3.0], [5.0, 5.0, 2.0]);
    let mut cut = cuboid([-2.0, -2.0, 0.0], [4.0, 4.0, 5.0]);
    cut.kind = PartKind::Negative;
    let object = mesh(vec![left, right, floating, cut]);
    let printed = landing(&object, &settings(json!({})), SlicerBuild::Orca25, 0.0).unwrap();
    let box_ = bounds(&printed.loops);
    // The frame is centred on all normal parts' bbox: x from −30 to 10 → centre −10.
    assert!(near(box_.min_x, -10.05 + 10.0, 0.003) && near(box_.max_x, 30.05 - 10.0 + 0.0, 0.003));
    assert!(
        !point_in_footprint(&printed.loops, 10.0, 0.0),
        "the hole of the negative part"
    );
    assert!(point_in_footprint(&printed.loops, 5.0, 0.0));
    let solid = 20.0 * 10.0 - 4.0 * 4.0;
    assert!(
        area(&printed.loops) > solid
            && area(&printed.loops) < solid + 0.05 * 2.0 * (60.0 + 16.0) + 0.05
    );
}

/// ε_L: 0.05 mm for classic walls; Arachne widens a bead to `min_bead_width`, so the bound is
/// ½·(mbw − mfs) of the nozzle and unmeasured.
#[test]
fn overshoot_follows_the_wall_generator() {
    let object = mesh(vec![cuboid([0.0, 0.0, 0.0], [10.0, 10.0, 5.0])]);
    let arachne = settings(
        json!({"wall_generator": "arachne", "min_bead_width": "85%", "min_feature_size": "25%"}),
    );
    let printed = landing(&object, &arachne, SlicerBuild::BambuStudio, 0.0).unwrap();
    assert!(near(printed.overshoot_mm, 0.12, 1e-12) && !printed.measured);
    let narrow = settings(
        json!({"wall_generator": "arachne", "min_bead_width": "0.3", "min_feature_size": "0.25"}),
    );
    let printed = landing(&object, &narrow, SlicerBuild::BambuStudio, 0.0).unwrap();
    assert!(near(printed.overshoot_mm, CLASSIC_OVERSHOOT_MM, 1e-12));
    let unknown = settings(json!({"wall_generator": "zigzag"}));
    assert_eq!(
        landing(&object, &unknown, SlicerBuild::BambuStudio, 0.0)
            .unwrap_err()
            .setting,
        "wall_generator"
    );
}

/// Every setting of 05 §2.3 is refused by name instead of guessed.
#[test]
fn landing_refuses_unmeasured_settings_by_name() {
    let object = mesh(vec![cuboid([0.0, 0.0, 0.0], [10.0, 10.0, 5.0])]);
    let cases = [
        (
            json!({"xy_contour_compensation": "0.1"}),
            "xy_contour_compensation",
        ),
        (
            json!({"xy_hole_compensation": "-0.05"}),
            "xy_hole_compensation",
        ),
        (json!({"detect_thin_wall": "1"}), "detect_thin_wall"),
        (
            json!({"fuzzy_skin": "external", "fuzzy_skin_first_layer": "1"}),
            "fuzzy_skin",
        ),
        (json!({"slicing_mode": "even_odd"}), "slicing_mode"),
        (json!({"raft_layers": "2"}), "raft_layers"),
        (
            json!({"enable_circle_compensation": "1"}),
            "enable_circle_compensation",
        ),
        (
            json!({"initial_layer_print_height": "0"}),
            "initial_layer_print_height",
        ),
    ];
    for (overrides, name) in cases {
        let error =
            landing(&object, &settings(overrides), SlicerBuild::BambuStudio, 0.0).unwrap_err();
        assert_eq!(error.setting, name, "{error}");
    }
    // Fuzzy skin away from layer 0 and thin walls under Arachne do not touch the landing.
    let fine = settings(json!({"fuzzy_skin": "external", "fuzzy_skin_first_layer": "0"}));
    assert!(landing(&object, &fine, SlicerBuild::BambuStudio, 0.0).is_ok());
}

/// A rotation class turns the section about the frame centre; the same rotation of the
/// unrotated landing gives the same outline.
#[test]
fn landing_of_a_rotation_class_is_the_rotated_section() {
    let object = mesh(vec![cuboid([0.0, 0.0, 0.0], [20.0, 6.0, 5.0])]);
    let settings = settings(json!({}));
    let flat = landing(&object, &settings, SlicerBuild::SnapmakerOrca, 0.0).unwrap();
    let turned = landing(&object, &settings, SlicerBuild::SnapmakerOrca, 90.0).unwrap();
    let (a, b) = (bounds(&flat.loops), bounds(&turned.loops));
    assert!(near(a.max_x, b.max_y, 1e-6) && near(a.max_y, -b.min_x, 1e-6));
    assert!(near(area(&flat.loops), area(&turned.loops), 1e-6));
    let again = landing(&object, &settings, SlicerBuild::SnapmakerOrca, 90.0).unwrap();
    assert_eq!(turned, again, "deterministic");
}

// ---------------------------------------------------------------------------------------
// Brim.

/// The corpus spire probe (6 × 6 × 40): the auto width is capped at the island's 6 mm side
/// and quantised to whole line pairs, 5.356 mm at a 0.25 mm first layer and 5.485 mm at
/// 0.2 mm; its loops' material reaches ±8.483 / ±8.606 mm (measured on every printer and
/// rotation, calibration corpus §2 item 10), the model 0.03 mm more for chordal offsets.
#[test]
fn tall_narrow_island_gets_its_capped_quantised_brim() {
    let spire = mesh(vec![cuboid([-3.0, -3.0, 0.0], [6.0, 6.0, 40.0])]);
    for (first_layer, width, reach, build) in [
        (0.25, 5.356, 8.483, SlicerBuild::Orca25),
        (0.25, 5.356, 8.483, SlicerBuild::SnapmakerOrca),
        (0.2, 5.485, 8.606, SlicerBuild::BambuStudio),
    ] {
        let settings = settings(json!({"initial_layer_print_height": first_layer.to_string()}));
        let printed = brim(&spire, &[1], &settings, build, 0.0).unwrap();
        assert!(
            near(printed.max_width_mm(), width, 0.001),
            "{build:?}: {:?}",
            printed.island_widths_mm
        );
        let box_ = bounds(&printed.loops);
        assert!(
            box_.max_x >= reach - 1e-3 && box_.max_x <= reach + 0.035,
            "{build:?}: {box_:?}"
        );
        assert!(
            !point_in_footprint(&printed.loops, 0.0, 0.0),
            "the brim is a ring"
        );
        assert!(
            !point_in_footprint(&printed.loops, 2.95, 0.0),
            "around the object, not on it"
        );
        assert!(point_in_footprint(&printed.loops, reach - 0.01, 0.0));
    }
}

/// A squat, wide object (the goat class) gets no auto brim in any build: the tipping term is
/// far below 5 mm and the thermal term (Orca family only) too.
#[test]
fn squat_objects_get_no_auto_brim() {
    let squat = mesh(vec![cuboid([-13.0, -12.0, 0.0], [26.0, 24.0, 44.0])]);
    for build in [
        SlicerBuild::BambuStudio,
        SlicerBuild::Orca24,
        SlicerBuild::Orca25,
        SlicerBuild::SnapmakerOrca,
    ] {
        let printed = brim(&squat, &[1], &settings(json!({})), build, 0.0).unwrap();
        assert!(
            printed.loops.is_empty() && printed.max_width_mm() == 0.0,
            "{build:?}"
        );
    }
}

/// A thin tall wall whose auto width is dropped gets the extra 5 mm brim in the Orca family
/// (quantised to 4.57 mm at 0.2 mm) and none in BambuStudio, whose guard is always false.
#[test]
fn thin_islands_get_the_extra_brim_outside_bambu_studio() {
    let wall = mesh(vec![cuboid([-10.0, -0.75, 0.0], [20.0, 1.5, 12.0])]);
    let settings = settings(json!({}));
    let bambu = brim(&wall, &[1], &settings, SlicerBuild::BambuStudio, 0.0).unwrap();
    assert_eq!(bambu.max_width_mm(), 0.0);
    let spacing = 0.5 - 0.2 * (1.0 - std::f64::consts::FRAC_PI_4);
    for build in [
        SlicerBuild::Orca24,
        SlicerBuild::Orca25,
        SlicerBuild::SnapmakerOrca,
    ] {
        let printed = brim(&wall, &[1], &settings, build, 0.0).unwrap();
        assert!(
            near(printed.max_width_mm(), quantised(5.0, spacing), 1e-9),
            "{build:?}"
        );
        assert!(!printed.loops.is_empty());
    }
}

/// Fixed brims use `brim_width` (quantised) on every island; inner brims fill holes; brim
/// ears and painted brims are refused.
#[test]
fn fixed_and_inner_brims_and_refusals() {
    let frame = {
        let outer = cuboid([-10.0, -10.0, 0.0], [20.0, 20.0, 5.0]);
        let mut hole = cuboid([-6.0, -6.0, 0.0], [12.0, 12.0, 5.0]);
        hole.kind = PartKind::Negative;
        mesh(vec![outer, hole])
    };
    let outer = settings(json!({"brim_type": "outer_only", "brim_width": "3"}));
    let printed = brim(&frame, &[1], &outer, SlicerBuild::Orca24, 0.0).unwrap();
    let spacing = 0.5 - 0.2 * (1.0 - std::f64::consts::FRAC_PI_4);
    assert!(near(printed.max_width_mm(), quantised(3.0, spacing), 1e-9));
    assert!(
        !point_in_footprint(&printed.loops, 0.0, 0.0),
        "no brim in the hole"
    );
    let both = settings(json!({"brim_type": "outer_and_inner", "brim_width": "3"}));
    let printed = brim(&frame, &[1], &both, SlicerBuild::Orca24, 0.0).unwrap();
    assert!(
        point_in_footprint(&printed.loops, -4.5, 0.0),
        "inner brim along the hole edge"
    );
    assert!(
        !point_in_footprint(&printed.loops, 0.0, 0.0),
        "the hole's middle stays clear"
    );
    let none = settings(json!({"brim_type": "no_brim"}));
    assert!(
        brim(&frame, &[1], &none, SlicerBuild::Orca24, 0.0)
            .unwrap()
            .loops
            .is_empty()
    );
    let ears = settings(json!({"brim_type": "brim_ears"}));
    assert_eq!(
        brim(&frame, &[1], &ears, SlicerBuild::BambuStudio, 0.0)
            .unwrap_err()
            .setting,
        "brim_type"
    );
}

/// The object's no-brim set is closed by one brim spacing in BambuStudio and Snapmaker Orca:
/// no brim between two islands 0.5 mm apart; Orca 2.4/2.5 brim the gap (03 §5.3).
#[test]
fn closing_the_no_brim_set_is_build_specific() {
    let pair = mesh(vec![
        cuboid([-10.0, -5.0, 0.0], [9.75, 10.0, 5.0]),
        cuboid([0.25, -5.0, 0.0], [9.75, 10.0, 5.0]),
    ]);
    let settings = settings(json!({"brim_type": "outer_only", "brim_width": "3"}));
    for (build, brimmed) in [
        (SlicerBuild::BambuStudio, false),
        (SlicerBuild::SnapmakerOrca, false),
        (SlicerBuild::Orca24, true),
        (SlicerBuild::Orca25, true),
    ] {
        let printed = brim(&pair, &[1, 1], &settings, build, 0.0).unwrap();
        assert_eq!(
            point_in_footprint(&printed.loops, 0.0, 0.0),
            brimmed,
            "{build:?}"
        );
        assert!(
            point_in_footprint(&printed.loops, 11.0, 0.0),
            "{build:?}: outside"
        );
    }
}

/// Builds are read from the 3MF `Application` and from profile `gui_flavour` names.
#[test]
fn slicer_builds_parse_from_application_and_flavour_names() {
    for (name, build) in [
        ("BambuStudio-02.08.02.61", SlicerBuild::BambuStudio),
        ("bambu-02.08", SlicerBuild::BambuStudio),
        ("OrcaSlicer-2.4.2", SlicerBuild::Orca24),
        ("orca-2.4", SlicerBuild::Orca24),
        ("OrcaSlicer 2.5.0-rc", SlicerBuild::Orca25),
        ("OrcaSlicer-2.6.1", SlicerBuild::Orca25),
        ("Snapmaker_Orca-2.4.0", SlicerBuild::SnapmakerOrca),
        ("snapmaker-orca-2.4", SlicerBuild::SnapmakerOrca),
    ] {
        assert_eq!(SlicerBuild::from_name(name), Some(build), "{name}");
    }
    assert_eq!(SlicerBuild::from_name("PrusaSlicer-2.8.1"), None);
}

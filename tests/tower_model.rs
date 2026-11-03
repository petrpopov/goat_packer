//! The prime-tower model of work package I3 on synthetic settings: the column per slicer
//! build against the measured goat towers (spec 04 §1, §8), brim and chamfer rings, plate
//! zones, anchor ranges, refused settings and determinism. The tool schedule has its own
//! file (`tower_schedule.rs`).

use std::collections::BTreeSet;

use goat_packer::Rect;
use goat_packer::first_layer::{ProjectSettings, SlicerBuild};
use goat_packer::object_print::PrintKind;
use goat_packer::point_in_footprint;
use goat_packer::tower_model::{
    LayerGrid, LayerOrdering, TOWER_EDGE_TOLERANCE_MM, ToolSchedule, TowerPlate, TowerPrint,
    printed_anchor, printed_tower,
};
use serde_json::json;

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

/// `base` with `overrides` written over it.
fn project(base: serde_json::Value, overrides: serde_json::Value) -> ProjectSettings {
    let mut values = base;
    for (key, value) in overrides.as_object().unwrap() {
        values[key] = value.clone();
    }
    ProjectSettings::from_json(&values)
}

/// The owner's P1S project (BambuStudio 02.08.02.61): rib wall, Saving prime volumes.
fn p1s() -> serde_json::Value {
    json!({
        "initial_layer_print_height": "0.2", "layer_height": "0.28", "nozzle_diameter": ["0.4"],
        "enable_prime_tower": "1", "prime_tower_width": "20", "prime_tower_brim_width": "3",
        "wipe_tower_x": ["221"], "wipe_tower_y": ["207.25"], "wipe_tower_rotation_angle": "0",
        "filament_prime_volume": ["45", "45", "30"], "prime_volume_mode": "Saving",
        "prime_tower_infill_gap": "150%", "prime_tower_rib_wall": "1", "prime_tower_rib_width": "8",
        "prime_tower_extra_rib_length": "0", "prime_tower_fillet_wall": "1",
        "filament_adhesiveness_category": ["100", "100", "100"],
        "filament_colour": ["#FFFFFF", "#000000", "#C12E1F"], "single_extruder_multi_material": "1",
        "timelapse_type": "0", "enable_wrapping_detection": "0",
    })
}

/// The owner's A2L project: 35 mm rib tower, auto brim, Default prime volumes.
fn a2l() -> serde_json::Value {
    json!({
        "initial_layer_print_height": "0.2", "layer_height": "0.28", "nozzle_diameter": ["0.4"],
        "prime_tower_width": "35", "prime_tower_brim_width": "-1",
        "wipe_tower_x": ["287.385", "153"], "wipe_tower_y": ["14.9999", "160"],
        "filament_prime_volume": ["45", "45", "30", "30"], "prime_volume_mode": "Default",
        "prime_tower_infill_gap": "150%", "prime_tower_rib_wall": "1", "prime_tower_rib_width": "8",
        "prime_tower_extra_rib_length": "0", "prime_tower_fillet_wall": "1",
        "filament_adhesiveness_category": ["100", "100", "100", "100"],
        "filament_colour": ["#FFFFFF", "#000000", "#C12E1F", "#FD8008"],
    })
}

/// The owner's U1 project (Snapmaker Orca 2.4.0): Type 2 rectangle, multi-tool ramming.
fn u1(width: &str) -> serde_json::Value {
    json!({
        "initial_layer_print_height": "0.25", "layer_height": "0.28", "nozzle_diameter": ["0.4", "0.4", "0.4", "0.4"],
        "prime_tower_width": width, "prime_tower_brim_width": "5", "prime_tower_brim_chamfer": "1",
        "prime_tower_brim_chamfer_max_width": "4", "wipe_tower_x": ["244.5"], "wipe_tower_y": ["245"],
        "prime_volume": "20", "wipe_tower_extra_spacing": "120%", "wipe_tower_extra_flow": "100%",
        "filament_multitool_ramming": ["1", "1", "1", "1"], "filament_multitool_ramming_volume": ["5", "5", "5", "5"],
        "filament_multitool_ramming_flow": ["25", "25", "25", "25"], "enable_filament_ramming": "0",
        "filament_minimal_purge_on_wipe_tower": ["15", "15", "15", "15"], "wipe_tower_wall_type": "rectangle",
        "single_extruder_multi_material": "0", "filament_colour": ["#FFFFFF", "#4C373C", "#A3413D", "#FD8008"],
    })
}

/// A goat-like schedule (spec 04 §8.4): filaments 1 and 2 on the first layers (one change on
/// layer 0), all three from layer 8 (two changes per layer), 1 and 3 on four layers, then
/// only 3 from layer `last` − 2, so the dilated sets change tools last on layer `last` − 1
/// (z 41.36 / 41.41 for `last` = 148: the measured goat tower tops).
fn goat_schedule(first_layer: f64, build: SlicerBuild, last: usize) -> ToolSchedule {
    let grid = LayerGrid::for_object(first_layer, 0.28, 44.743);
    let raw: Vec<BTreeSet<u32>> = (0..grid.count)
        .map(|layer| match layer {
            0..8 => BTreeSet::from([1, 2]),
            _ if layer + 6 < last => BTreeSet::from([1, 2, 3]),
            _ if layer + 2 < last => BTreeSet::from([1, 3]),
            _ => BTreeSet::from([3]),
        })
        .collect();
    ToolSchedule::from_sets(&grid, &raw, LayerOrdering::of(build))
}

/// The corpus marker probes: one filament up to layer 30, a second one from there to the top
/// (one change per layer, none on the first).
fn marker_schedule(first_layer: f64, build: SlicerBuild) -> ToolSchedule {
    let grid = LayerGrid::for_object(first_layer, 0.28, 11.6);
    let raw: Vec<BTreeSet<u32>> = (0..grid.count)
        .map(|layer| {
            if layer >= 30 {
                BTreeSet::from([1, 3])
            } else {
                BTreeSet::from([1])
            }
        })
        .collect();
    ToolSchedule::from_sets(&grid, &raw, LayerOrdering::of(build))
}

fn plate(printable: Rect) -> TowerPlate {
    TowerPlate {
        printable,
        object_height: 44.743,
    }
}

fn tower(
    settings: serde_json::Value,
    schedule: &ToolSchedule,
    build: SlicerBuild,
    printable: Rect,
) -> TowerPrint {
    let settings = project(settings, json!({}));
    printed_tower(&settings, schedule, build, &plate(printable))
        .unwrap()
        .unwrap()
}

const P1S_BED: Rect = Rect {
    min_x: 0.0,
    min_y: 0.0,
    max_x: 256.0,
    max_y: 256.0,
};
const A2L_BED: Rect = Rect {
    min_x: 0.0,
    min_y: 0.0,
    max_x: 330.0,
    max_y: 320.0,
};
const U1_BED: Rect = Rect {
    min_x: 0.5,
    min_y: 1.0,
    max_x: 270.5,
    max_y: 271.0,
};

// ---------------------------------------------------------------------------------------
// Columns against the measured towers (spec 04 §1, §8.3; calibration corpus §2 item 9).

#[test]
fn bambu_rib_towers_match_the_printed_goat_towers() {
    let p1s_tower = tower(
        p1s(),
        &goat_schedule(0.2, SlicerBuild::BambuStudio, 148),
        SlicerBuild::BambuStudio,
        P1S_BED,
    );
    assert_eq!(p1s_tower.box_size, (15.0, 14.0));
    assert!(near(p1s_tower.height, 41.36, 1e-9) && p1s_tower.max_changes == 2);
    // Measured wall centre line 18.489 × 17.638 on every P1S slice.
    assert!(
        near(p1s_tower.body_size.0, 18.489, 0.005) && near(p1s_tower.body_size.1, 17.638, 0.005),
        "{:?}",
        p1s_tower.body_size
    );
    assert!(near(p1s_tower.l0_brim_mm, 7.0 * 0.45708, 0.001));
    let reaches: Vec<f64> = p1s_tower.chamfer.iter().map(|ring| ring.reach_mm).collect();
    assert_eq!(reaches.len(), 5);
    assert!(near(reaches[0], 2.2, 0.001) && near(reaches[4], 0.44, 0.001));
    assert!(
        near(p1s_tower.chamfer[4].z_top, 1.6, 1e-9)
            && near(p1s_tower.chamfer[0].z_bottom, 0.2, 1e-9)
    );

    let a2l_tower = tower(
        a2l(),
        &goat_schedule(0.2, SlicerBuild::BambuStudio, 148),
        SlicerBuild::BambuStudio,
        A2L_BED,
    );
    assert_eq!(a2l_tower.box_size, (24.5, 24.5));
    assert!(
        near(a2l_tower.body_size.0, 28.578, 0.005) && near(a2l_tower.body_size.1, 28.578, 0.005)
    );
    // Auto brim by tower height: 7 loops at H_t ≈ 41.6.
    assert!(near(a2l_tower.l0_brim_mm, 7.0 * 0.45708, 0.001));
}

#[test]
fn bambu_single_change_towers_match_the_marker_probes() {
    let p1s_tower = tower(
        p1s(),
        &marker_schedule(0.2, SlicerBuild::BambuStudio),
        SlicerBuild::BambuStudio,
        P1S_BED,
    );
    // Measured 13.546 × 12.906 (P1S arms/lean50/mushroom probes).
    assert_eq!(p1s_tower.box_size, (11.0, 10.25));
    assert!(
        near(p1s_tower.body_size.0, 13.546, 0.01) && near(p1s_tower.body_size.1, 12.906, 0.01),
        "{:?}",
        p1s_tower.body_size
    );
    let a2l_tower = tower(
        a2l(),
        &marker_schedule(0.2, SlicerBuild::BambuStudio),
        SlicerBuild::BambuStudio,
        A2L_BED,
    );
    // Measured 21.582 × 21.785.
    assert!(
        near(a2l_tower.body_size.0, 21.582, 0.02) && near(a2l_tower.body_size.1, 21.785, 0.03),
        "{:?}",
        a2l_tower.body_size
    );
}

#[test]
fn type2_towers_differ_per_build_as_measured() {
    let printable = U1_BED;
    for (build, width, depth) in [
        (SlicerBuild::SnapmakerOrca, "20", 16.1),
        (SlicerBuild::Orca25, "20", 14.9),
        (SlicerBuild::Orca24, "20", 9.5),
        (SlicerBuild::SnapmakerOrca, "16", 19.7),
    ] {
        let schedule = goat_schedule(0.25, build, 148);
        let printed = tower(u1(width), &schedule, build, printable);
        let w: f64 = width.parse().unwrap();
        assert!(
            near(printed.body_size.0, w, 1e-9) && near(printed.body_size.1, depth, 1e-6),
            "{build:?}: {:?}",
            printed.body_size
        );
        assert!(
            near(printed.l0_brim_mm, 11.0 * 0.44635, 0.001),
            "L0 29.82 = 20 + 2·4.91"
        );
    }
    // Measured 20 × 7.1 for the R marker probes (no change on the first layer).
    let marker = tower(
        u1("20"),
        &marker_schedule(0.25, SlicerBuild::Orca25),
        SlicerBuild::Orca25,
        printable,
    );
    assert!(near(marker.body_size.1, 7.1, 1e-6));
}

#[test]
fn chamfer_rings_are_build_specific() {
    let schedule = goat_schedule(0.25, SlicerBuild::SnapmakerOrca, 148);
    let snapmaker = tower(u1("20"), &schedule, SlicerBuild::SnapmakerOrca, U1_BED);
    let reaches: Vec<f64> = snapmaker.chamfer.iter().map(|ring| ring.reach_mm).collect();
    // Measured 3.519, 3.079, …, 0.440 on the owner's plates: 8 … 1 loops at 0.4399.
    assert_eq!(reaches.len(), 8);
    assert!(near(reaches[0], 3.519, 0.001) && near(reaches[7], 0.44, 0.001));
    assert!(
        near(snapmaker.chamfer[7].z_top, 2.49, 1e-9)
            && near(snapmaker.chamfer[0].z_bottom, 0.25, 1e-9)
    );
    let orca = tower(
        u1("20"),
        &goat_schedule(0.25, SlicerBuild::Orca25, 148),
        SlicerBuild::Orca25,
        U1_BED,
    );
    assert!(orca.chamfer.is_empty());
    let mut off = u1("20");
    off["prime_tower_brim_chamfer"] = json!("0");
    assert!(
        tower(off, &schedule, SlicerBuild::SnapmakerOrca, U1_BED)
            .chamfer
            .is_empty()
    );
}

// ---------------------------------------------------------------------------------------
// Zones.

/// Material = centre line + half the tower line + 0.1 mm: the column on every tower layer, each
/// chamfer ring (with the column) on its own layer as tower body, the L0 brim ring alone as
/// tower brim.
#[test]
fn zones_split_body_rings_and_the_first_layer_brim() {
    let schedule = goat_schedule(0.25, SlicerBuild::SnapmakerOrca, 148);
    let printed = tower(u1("20"), &schedule, SlicerBuild::SnapmakerOrca, U1_BED);
    let anchor = (244.5, 245.0);
    let edge = 0.25 + TOWER_EDGE_TOLERANCE_MM;
    let body = printed.body_zones(anchor);
    assert_eq!(body.len(), 1 + 8);
    assert!(body.iter().all(|zone| zone.kind == PrintKind::TowerBody));
    let column = Rect::of_points(body[0].loops.iter().flatten());
    assert!(
        near(column.min_x, 244.5 - edge, 1e-6) && near(column.max_y, 245.0 + 16.1 + edge, 1e-6)
    );
    assert_eq!(body[0].z_range, (0.0, printed.height));
    let ring = Rect::of_points(body[1].loops.iter().flatten());
    assert!(near(ring.min_x, 244.5 - 3.519 - edge, 0.001) && near(body[1].z_range.1, 0.53, 1e-9));
    let brim = printed.brim_zone(anchor).unwrap();
    assert_eq!(brim.kind, PrintKind::TowerBrim);
    assert_eq!(brim.z_range, (0.0, 0.25));
    let outer = Rect::of_points(brim.loops.iter().flatten());
    assert!(near(outer.max_x, 264.5 + 4.910 + edge, 0.001));
    assert!(
        point_in_footprint(&brim.loops, 244.5 - 2.0, 250.0),
        "the brim ring"
    );
    assert!(
        !point_in_footprint(&brim.loops, 254.0, 250.0),
        "not the column"
    );
    assert_eq!(printed.zones(anchor).len(), body.len() + 1);
    // Band rule (design §4.1): a layer (z_prev, z] is in every band it overlaps.
    assert_eq!(body[1].bands(), 0..=0);
    assert_eq!(
        body[8].bands(),
        1..=1,
        "the last S ring prints on (2.21, 2.49]"
    );
    assert_eq!(brim.bands(), 0..=0);
    assert_eq!(body[0].bands(), 0..=20, "the column up to 41.41");
    let mut ring = body[1].clone();
    ring.z_range = (1.88, 2.16);
    assert_eq!(ring.bands(), 0..=1);
    ring.z_range = (1.72, 2.0);
    assert_eq!(ring.bands(), 0..=0);
}

/// Rib walls keep their notches in the offsets: the brim zone is the mitred outline.
#[test]
fn rib_zones_follow_the_filleted_outline() {
    let printed = tower(
        p1s(),
        &goat_schedule(0.2, SlicerBuild::BambuStudio, 148),
        SlicerBuild::BambuStudio,
        P1S_BED,
    );
    let bounds = printed.material_bounds();
    let reach = printed.l0_brim_mm + 0.25 + TOWER_EDGE_TOLERANCE_MM;
    assert!(
        near(bounds.min_x, -reach, 0.01) && near(bounds.max_x, printed.body_size.0 + reach, 0.01),
        "{bounds:?}"
    );
    let column = &printed.body_zones((0.0, 0.0))[0];
    // The box corner is inside the rib wall; the outline's own corner region is rounded.
    assert!(point_in_footprint(&column.loops, 9.0, 9.0));
    assert!(
        !point_in_footprint(&column.loops, -0.2, -0.2),
        "fillets round the rib ends"
    );
}

// ---------------------------------------------------------------------------------------
// Anchor ranges (spec 04 §6).

#[test]
fn anchor_ranges_reproduce_the_gui_and_cli_clamps() {
    let p1s_tower = tower(
        p1s(),
        &goat_schedule(0.2, SlicerBuild::BambuStudio, 148),
        SlicerBuild::BambuStudio,
        P1S_BED,
    );
    let range = p1s_tower.anchor_range.unwrap();
    assert!(
        near(range.min_x, 15.0, 1e-9)
            && near(range.max_x, 223.84, 0.01)
            && near(range.max_y, 223.84, 0.01)
    );
    // The A2L project's stored x = 330 − 15 − 27.615 sits exactly on the bound for the
    // plate's three filaments.
    let a2l_tower = tower(
        a2l(),
        &goat_schedule(0.2, SlicerBuild::BambuStudio, 148),
        SlicerBuild::BambuStudio,
        A2L_BED,
    );
    assert!(near(a2l_tower.anchor_range.unwrap().max_x, 287.385, 0.001));
    // Snapmaker: margin 1 + 5 mm brim, real depth 16.1 (re-clamped after every slice).
    let snapmaker = tower(
        u1("20"),
        &goat_schedule(0.25, SlicerBuild::SnapmakerOrca, 148),
        SlicerBuild::SnapmakerOrca,
        U1_BED,
    );
    let range = snapmaker.anchor_range.unwrap();
    assert_eq!((range.min_x, range.min_y), (6.0, 6.0));
    assert!(near(range.max_x, 244.5, 1e-9) && near(range.max_y, 271.0 - 16.1 - 6.0, 1e-6));
    // OrcaSlicer 2.5 snaps the stored 244.5 to 244.09 on every slice (69 corpus slices).
    let orca25 = tower(
        u1("20"),
        &goat_schedule(0.25, SlicerBuild::Orca25, 148),
        SlicerBuild::Orca25,
        U1_BED,
    );
    let range = orca25.anchor_range.unwrap();
    let printed = printed_anchor(SlicerBuild::Orca25, orca25.source_anchor, range).unwrap();
    assert!(near(printed.0, 244.09, 0.001) && near(printed.1, 245.0, 1e-9));
    assert_eq!(
        printed_anchor(SlicerBuild::Orca25, (200.0, 300.0), range),
        None,
        "relocated, unpredictable"
    );
    assert_eq!(
        printed_anchor(SlicerBuild::Orca24, (300.0, 300.0), range),
        Some((300.0, 300.0))
    );
    assert_eq!(
        printed_anchor(SlicerBuild::SnapmakerOrca, (300.0, 1.0), range),
        Some((range.max_x, range.min_y))
    );
}

// ---------------------------------------------------------------------------------------
// Refusals, absent towers, determinism.

#[test]
fn unmodelled_tower_settings_are_refused_by_name() {
    let schedule = goat_schedule(0.25, SlicerBuild::SnapmakerOrca, 148);
    let cases = [
        (
            u1("20"),
            json!({"wipe_tower_rotation_angle": "90"}),
            SlicerBuild::SnapmakerOrca,
            "wipe_tower_rotation_angle",
        ),
        (
            u1("20"),
            json!({"single_extruder_multi_material": "1"}),
            SlicerBuild::Orca24,
            "single_extruder_multi_material",
        ),
        (
            u1("20"),
            json!({"wipe_tower_wall_type": "cone"}),
            SlicerBuild::Orca25,
            "wipe_tower_wall_type",
        ),
        (
            u1("20"),
            json!({"wipe_tower_no_sparse_layers": "1"}),
            SlicerBuild::SnapmakerOrca,
            "wipe_tower_no_sparse_layers",
        ),
        (
            u1("20"),
            json!({"timelapse_type": "1"}),
            SlicerBuild::SnapmakerOrca,
            "timelapse_type",
        ),
        (
            u1("20"),
            json!({"enable_support": "1", "support_filament": "2"}),
            SlicerBuild::SnapmakerOrca,
            "support_filament",
        ),
        (
            p1s(),
            json!({"nozzle_diameter": ["0.4", "0.4"]}),
            SlicerBuild::BambuStudio,
            "nozzle_diameter",
        ),
        (
            u1("20"),
            json!({"prime_tower_width": "0"}),
            SlicerBuild::SnapmakerOrca,
            "prime_tower_width",
        ),
    ];
    for (base, overrides, build, name) in cases {
        let settings = project(base, overrides);
        let error = printed_tower(&settings, &schedule, build, &plate(U1_BED)).unwrap_err();
        assert_eq!(error.setting, name, "{error}");
    }
}

#[test]
fn no_tower_without_changes_or_when_disabled() {
    let grid = LayerGrid::for_object(0.25, 0.28, 10.0);
    let single = ToolSchedule::from_sets(
        &grid,
        &vec![BTreeSet::from([1]); grid.count],
        LayerOrdering::LoadedFirst,
    );
    let settings = project(u1("20"), json!({}));
    assert_eq!(
        printed_tower(
            &settings,
            &single,
            SlicerBuild::SnapmakerOrca,
            &plate(U1_BED)
        )
        .unwrap(),
        None
    );
    let disabled = project(u1("20"), json!({"enable_prime_tower": "0"}));
    let schedule = goat_schedule(0.25, SlicerBuild::SnapmakerOrca, 148);
    assert_eq!(
        printed_tower(
            &disabled,
            &schedule,
            SlicerBuild::SnapmakerOrca,
            &plate(U1_BED)
        )
        .unwrap(),
        None
    );
}

#[test]
fn towers_are_deterministic() {
    for build in [SlicerBuild::BambuStudio, SlicerBuild::SnapmakerOrca] {
        let settings = if build == SlicerBuild::BambuStudio {
            p1s()
        } else {
            u1("20")
        };
        let schedule = goat_schedule(0.2, build, 148);
        let first = tower(settings.clone(), &schedule, build, P1S_BED);
        let second = tower(settings, &schedule, build, P1S_BED);
        assert_eq!(first, second);
        assert_eq!(first.zones((10.0, 20.0)), second.zones((10.0, 20.0)));
    }
}

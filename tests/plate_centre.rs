use std::io::{Read, Write};

use goat_packer::{
    Point, Rect,
    colored_packer::PrinterProfile,
    plate_centre::{
        HeatZone, centre_out_order, loops_bounds, plate_centre, relocate_wipe_tower_in_3mf,
        shift_wipe_tower_settings, smallest_containing_zone,
    },
};
use serde_json::json;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

fn nested_zones() -> Vec<HeatZone> {
    vec![
        HeatZone { name: "inner".into(), rect: [102.0, 93.4, 228.0, 226.6] },
        HeatZone { name: "middle".into(), rect: [60.0, 49.0, 270.0, 271.0] },
        HeatZone { name: "full".into(), rect: [0.0, 0.0, 330.0, 320.0] },
    ]
}

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

fn assert_rect(actual: Rect, expected: [f64; 4]) {
    assert_close(actual.min_x, expected[0]);
    assert_close(actual.min_y, expected[1]);
    assert_close(actual.max_x, expected[2]);
    assert_close(actual.max_y, expected[3]);
}

// ---- profile -------------------------------------------------------------------------------

fn fixture_profile(name: &str) -> PrinterProfile {
    let path = format!("{}/tests/fixtures/profiles/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn a_profile_carries_its_bed_heat_soak_zones_smallest_first() {
    assert_eq!(fixture_profile("heat_zones.json").heat_zones, nested_zones());
}

#[test]
fn profiles_without_heat_zones_parse_to_an_empty_list() {
    for name in ["corner_bed.json", "margin_bed.json"] {
        assert!(fixture_profile(name).heat_zones.is_empty(), "{name}");
    }
}

// ---- centre --------------------------------------------------------------------------------

#[test]
fn centre_is_the_smallest_zone_centre_or_the_usable_bed_centre() {
    let bed = Rect::new(0.0, 0.0, 330.0, 320.0);
    let (x, y) = plate_centre(&nested_zones(), bed);
    assert_close(x, 165.0);
    assert_close(y, 160.0);
    let (x, y) = plate_centre(&[], Rect::new(3.5, 3.5, 266.5, 266.5));
    assert_close(x, 135.0);
    assert_close(y, 135.0);
}

#[test]
fn centre_comes_from_the_smallest_zone_regardless_of_profile_order() {
    let mut zones = nested_zones();
    zones.reverse();
    zones[0].rect = [0.0, 0.0, 300.0, 300.0]; // "full" first, centre (150, 150)
    let (x, y) = plate_centre(&zones, Rect::new(0.0, 0.0, 330.0, 320.0));
    assert_close(x, 165.0); // inner zone centre
    assert_close(y, 160.0);
}

// ---- keepout shift + clamp -----------------------------------------------------------------

// ---- settings patch ------------------------------------------------------------------------

#[test]
fn wipe_tower_patch_shifts_only_entry_zero_and_keeps_strings() {
    let mut settings = json!({
        "wipe_tower_x": ["283.572", "10", "20"],
        "wipe_tower_y": ["266.103", "30", "40"],
        "other": "untouched"
    });
    let moved = shift_wipe_tower_settings(&mut settings, (-118.674, -106.103)).unwrap();
    assert_close(moved.0, 164.898);
    assert_close(moved.1, 160.0);
    assert_eq!(settings["wipe_tower_x"], json!(["164.898", "10", "20"]));
    assert_eq!(settings["wipe_tower_y"], json!(["160", "30", "40"]));
    assert_eq!(settings["other"], json!("untouched"));
}

#[test]
fn wipe_tower_patch_rejects_missing_or_malformed_entries() {
    let mut missing = json!({ "wipe_tower_x": ["1"] });
    assert!(shift_wipe_tower_settings(&mut missing, (1.0, 1.0)).is_err());
    let mut empty = json!({ "wipe_tower_x": [], "wipe_tower_y": [] });
    assert!(shift_wipe_tower_settings(&mut empty, (1.0, 1.0)).is_err());
    let mut garbage = json!({ "wipe_tower_x": ["abc"], "wipe_tower_y": ["1"] });
    assert!(shift_wipe_tower_settings(&mut garbage, (1.0, 1.0)).is_err());
    // A failed patch leaves the settings untouched.
    assert_eq!(garbage, json!({ "wipe_tower_x": ["abc"], "wipe_tower_y": ["1"] }));
}

fn write_project(path: &std::path::Path, settings: &serde_json::Value) {
    let mut zip = ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = SimpleFileOptions::default();
    zip.start_file("3D/3dmodel.model", options).unwrap();
    zip.write_all(b"<model/>").unwrap();
    zip.start_file("Metadata/project_settings.config", options).unwrap();
    zip.write_all(&serde_json::to_vec(settings).unwrap()).unwrap();
    zip.finish().unwrap();
}

fn read_entry(path: &std::path::Path, name: &str) -> Vec<u8> {
    let mut archive = ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
    bytes
}

#[test]
fn project_rewrite_patches_the_settings_entry_and_keeps_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plate.3mf");
    write_project(
        &path,
        &json!({ "wipe_tower_x": ["283.572"], "wipe_tower_y": ["266.103"], "a": "b" }),
    );
    let moved = relocate_wipe_tower_in_3mf(&path, (-118.674, -106.103)).unwrap();
    assert_close(moved.0, 164.898);
    assert_close(moved.1, 160.0);
    let settings: serde_json::Value =
        serde_json::from_slice(&read_entry(&path, "Metadata/project_settings.config")).unwrap();
    assert_eq!(settings["wipe_tower_x"], json!(["164.898"]));
    assert_eq!(settings["wipe_tower_y"], json!(["160"]));
    assert_eq!(settings["a"], json!("b"));
    assert_eq!(read_entry(&path, "3D/3dmodel.model"), b"<model/>");
}

// ---- ordering ------------------------------------------------------------------------------

#[test]
fn centre_out_order_sorts_by_distance_with_index_ties() {
    let centres = [(0.0, 0.0), (10.0, 10.0), (12.0, 10.0), (8.0, 10.0), (10.0, 30.0)];
    // (12,10) and (8,10) are equally far: lower index first.
    assert_eq!(centre_out_order(&centres, (10.0, 10.0)), vec![1, 2, 3, 0, 4]);
    assert!(centre_out_order(&[], (0.0, 0.0)).is_empty());
}

#[test]
fn loops_bounds_covers_every_point() {
    let loops = vec![
        vec![Point { x: 1.0, y: 2.0 }, Point { x: 5.0, y: -1.0 }],
        vec![Point { x: -3.0, y: 7.0 }],
    ];
    assert_rect(loops_bounds(&loops).unwrap(), [-3.0, -1.0, 5.0, 7.0]);
    assert!(loops_bounds(&[]).is_none());
}

// ---- zones ---------------------------------------------------------------------------------

#[test]
fn smallest_containing_zone_is_reported() {
    let zones = nested_zones();
    let name = |bbox: Rect| smallest_containing_zone(&zones, bbox).map(|zone| zone.name.clone());
    assert_eq!(name(Rect::new(110.0, 100.0, 220.0, 220.0)), Some("inner".into()));
    // Edges count as inside.
    assert_eq!(name(Rect::new(102.0, 93.4, 228.0, 226.6)), Some("inner".into()));
    assert_eq!(name(Rect::new(100.0, 100.0, 220.0, 220.0)), Some("middle".into()));
    assert_eq!(name(Rect::new(10.0, 10.0, 300.0, 300.0)), Some("full".into()));
    assert_eq!(name(Rect::new(-1.0, 10.0, 300.0, 300.0)), None);
    assert!(smallest_containing_zone(&[], Rect::new(1.0, 1.0, 2.0, 2.0)).is_none());
}

#[test]
fn smallest_zone_does_not_depend_on_profile_order() {
    let mut zones = nested_zones();
    zones.reverse();
    let zone = smallest_containing_zone(&zones, Rect::new(110.0, 100.0, 220.0, 220.0)).unwrap();
    assert_eq!(zone.name, "inner");
}

// ---- tower verification --------------------------------------------------------------------

// ---- free tower spot -----------------------------------------------------------------------


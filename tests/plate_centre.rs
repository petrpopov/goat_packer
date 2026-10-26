use std::io::{Read, Write};

use goat_packer::{
    Point, Rect,
    colored_packer::PrinterProfile,
    plate_centre::{
        HeatZone, centre_out_order, ensure_relocated_tower_clear, loops_bounds, plate_centre,
        relocate_keepout,
        relocate_wipe_tower_in_3mf, shift_wipe_tower_settings, smallest_containing_zone,
        tower_corner_error_mm,
    },
};
use serde_json::json;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

fn a2l_zones() -> Vec<HeatZone> {
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

#[test]
fn a2l_profile_carries_the_bed_heat_soak_zones_smallest_first() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/profiles/bambu_a2l.json"))
        .unwrap();
    let profile: PrinterProfile = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(profile.heat_zones, a2l_zones());
}

#[test]
fn profiles_without_heat_zones_parse_to_an_empty_list() {
    for name in ["p1s.json", "snapmaker_u1.json"] {
        let path = format!("{}/profiles/{name}", env!("CARGO_MANIFEST_DIR"));
        let profile: PrinterProfile = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(profile.heat_zones.is_empty(), "{name}");
    }
}

// ---- centre --------------------------------------------------------------------------------

#[test]
fn centre_is_the_first_zone_centre_or_the_usable_bed_centre() {
    let bed = Rect::new(0.0, 0.0, 330.0, 320.0);
    let (x, y) = plate_centre(&a2l_zones(), bed);
    assert_close(x, 165.0);
    assert_close(y, 160.0);
    let (x, y) = plate_centre(&[], Rect::new(3.5, 3.5, 266.5, 266.5));
    assert_close(x, 135.0);
    assert_close(y, 135.0);
}

// ---- keepout shift + clamp -----------------------------------------------------------------

#[test]
fn keepout_moves_its_centre_onto_the_target() {
    let bed = Rect::new(0.0, 0.0, 330.0, 320.0);
    let probe = Rect::new(280.0, 260.0, 320.0, 310.0); // centre (300, 285)
    let moved = relocate_keepout(probe, (165.0, 160.0), bed);
    assert_rect(moved.keepout, [145.0, 135.0, 185.0, 185.0]);
    assert_close(moved.shift.0, -135.0);
    assert_close(moved.shift.1, -125.0);
}

#[test]
fn clamp_shrinks_the_shift_never_the_keepout() {
    let bed = Rect::new(0.0, 0.0, 330.0, 320.0);
    let probe = Rect::new(100.0, 100.0, 140.0, 150.0); // 40 x 50
    // Target near the top-right corner: the keepout would overhang by 20 mm in x and 15 in y.
    let moved = relocate_keepout(probe, (330.0, 310.0), bed);
    assert_rect(moved.keepout, [290.0, 270.0, 330.0, 320.0]);
    assert_close(moved.shift.0, 190.0);
    assert_close(moved.shift.1, 170.0);
    // And towards the origin.
    let moved = relocate_keepout(probe, (5.0, 5.0), bed);
    assert_rect(moved.keepout, [0.0, 0.0, 40.0, 50.0]);
    assert_close(moved.shift.0, -100.0);
    assert_close(moved.shift.1, -100.0);
}

#[test]
fn relocated_tower_must_not_overlap_a_fixed_keepout() {
    let tower = Rect::new(145.0, 135.0, 185.0, 185.0);
    // Exclusion corner and a purge strip elsewhere: clear. Edge contact is not an overlap.
    let clear = [Rect::new(0.0, 0.0, 20.0, 20.0), Rect::new(185.0, 0.0, 300.0, 5.0)];
    assert!(ensure_relocated_tower_clear(tower, &clear).is_ok());
    let blocked = [Rect::new(0.0, 0.0, 20.0, 20.0), Rect::new(180.0, 100.0, 200.0, 140.0)];
    let error = ensure_relocated_tower_clear(tower, &blocked).unwrap_err();
    assert_eq!(
        error,
        "relocated wipe tower would overlap a printer keep-out area; rerun with --no-centre"
    );
}

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
    let zones = a2l_zones();
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
    let mut zones = a2l_zones();
    zones.reverse();
    let zone = smallest_containing_zone(&zones, Rect::new(110.0, 100.0, 220.0, 220.0)).unwrap();
    assert_eq!(zone.name, "inner");
}

// ---- tower verification --------------------------------------------------------------------

#[test]
fn tower_corner_error_is_the_min_corner_distance() {
    let target = Rect::new(145.0, 135.0, 185.0, 185.0);
    assert_close(tower_corner_error_mm(target, target), 0.0);
    let measured = Rect::new(148.0, 139.0, 188.0, 189.0); // corner moved by (3, 4)
    assert_close(tower_corner_error_mm(measured, target), 5.0);
}

#[test]
fn tower_corner_error_ignores_one_sided_growth() {
    // wipe_tower_x/y anchors the min corner; a full plate's tower grows away from it (here 30 mm
    // in y). The centre moves 15 mm, the anchor does not move at all.
    let target = Rect::new(145.0, 135.0, 185.0, 185.0);
    let grown = Rect::new(145.0, 135.0, 185.0, 215.0);
    assert_close(tower_corner_error_mm(grown, target), 0.0);
}

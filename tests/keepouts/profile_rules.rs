//! A profile's bed rules as plate rules (design §7.4, §7.5): the legacy table for profiles
//! without `bed_rules`, declared rules, the G-code frame and the GUI's own band.

use goat_packer::colored_packer::PrinterProfile;
use goat_packer::colored_packer::bed_rules::{
    LEGACY_EXCLUSION_GROWTH_MM, ProjectBed, old_field_mismatches, plate_rules,
};
use goat_packer::keepout::{BodyRule, Keepout, KeepoutFrame, KeepoutShape, Permission, ZoneClass};
use goat_packer::keepout::{Gap, permission};
use goat_packer::{KeepoutClass, PrintKind, Rect};
use serde_json::{Value, json};

/// A generic 256 mm test bed with `fields` on top.
fn profile(fields: Value) -> PrinterProfile {
    let mut base = json!({
        "id": "test_bed", "printer_name": "Test Bed", "slicer_kind": "bambu", "slicer_path": "",
        "blank_start_gcode_for_cli": false, "no_check_for_final_tower_probe": false,
        "printable_area": [0.0, 0.0, 256.0, 256.0], "edge_margin_mm": 0.0,
        "exclusion_rectangles": [], "purge_lines": [], "tower_marker_style": "wipe_tower",
        "tower_line_width_mm": 0.5
    });
    for (key, value) in fields.as_object().unwrap() {
        base[key] = value.clone();
    }
    serde_json::from_value(base).unwrap()
}

/// A project whose G-code is shifted like a P1S one, first layer 0.2 mm.
const SHIFTED: ProjectBed = ProjectBed {
    gcode_offset: (0.0, 2.0),
    first_layer_height_mm: Some(0.2),
};

fn keepout(id: &str, class: KeepoutClass, shape: KeepoutShape, z: Option<[f64; 2]>) -> Keepout {
    Keepout {
        id: id.to_owned(),
        class,
        shape,
        z_range_mm: z,
        frame: KeepoutFrame::Plate,
    }
}

/// Design §7.5, row by row: old fields → body band, print reserve, keepouts (plate frame).
#[test]
fn legacy_profile_fields_map_through_the_design_table() {
    let grow = LEGACY_EXCLUSION_GROWTH_MM;
    let table: Vec<(&str, Value, f64, f64, Vec<Keepout>)> = vec![
        ("no bed fields", json!({}), 0.0, 0.0, vec![]),
        (
            "an edge margin alone binds everything",
            json!({"edge_margin_mm": 5.0}),
            0.0,
            0.0,
            vec![keepout(
                "edge_margin",
                KeepoutClass::Hard,
                KeepoutShape::EdgeBand(5.0),
                None,
            )],
        ),
        (
            "with a support margin it binds bodies, the support margin is the reserve",
            json!({"edge_margin_mm": 4.0, "support_edge_margin_mm": 0.7}),
            4.0,
            0.7,
            vec![],
        ),
        (
            "exclusions are hard and keep the legacy growth",
            json!({"exclusion_rectangles": [[0.0, 0.0, 18.0, 28.0]]}),
            0.0,
            0.0,
            vec![keepout(
                "exclusion_0",
                KeepoutClass::Hard,
                KeepoutShape::Rect(Rect::new(-grow, -grow, 18.0 + grow, 28.0 + grow)),
                None,
            )],
        ),
        (
            "purge lines are soft G-code strips up to the first layer",
            json!({"purge_lines": [[18.0, 1.0, 240.0, 1.0, 0.81]]}),
            0.0,
            0.0,
            vec![keepout(
                "purge",
                KeepoutClass::Soft,
                KeepoutShape::Strips(vec![[18.0, 3.0, 240.0, 3.0, 0.81]]),
                Some([0.0, 0.2]),
            )],
        ),
    ];
    assert_eq!(LEGACY_EXCLUSION_GROWTH_MM, 4.0);
    for (case, fields, band, reserve, keepouts) in table {
        let rules = plate_rules(&profile(fields), &SHIFTED).unwrap();
        assert_eq!(
            rules.printable_area,
            Rect::new(0.0, 0.0, 256.0, 256.0),
            "{case}"
        );
        assert_eq!(rules.body_edge_band_mm, band, "{case}");
        assert_eq!(rules.print_edge_reserve_mm, reserve, "{case}");
        assert_eq!(rules.keepouts, keepouts, "{case}");
        assert_eq!(rules.body_rule, BodyRule::Silhouette, "{case}");
    }
}

#[test]
fn a_legacy_purge_without_a_known_first_layer_holds_at_every_height() {
    let fields = json!({"purge_lines": [[18.0, 1.0, 240.0, 1.0, 0.81]]});
    let rules = plate_rules(&profile(fields), &ProjectBed::default()).unwrap();
    assert_eq!(rules.keepouts[0].z_range_mm, None);
    assert_eq!(
        rules.keepouts[0].shape,
        KeepoutShape::Strips(vec![[18.0, 1.0, 240.0, 1.0, 0.81]])
    );
}

/// Declared rules replace the old fields entirely: exact exclusions, G-code strips shifted
/// into the plate frame, plate-frame zones left alone.
#[test]
fn declared_rules_win_and_only_gcode_zones_move() {
    let declared = profile(json!({
        "edge_margin_mm": 4.0,
        "exclusion_rectangles": [[0.0, 0.0, 18.0, 28.0]],
        "bed_rules": {
            "body_edge_band_mm": 3.5,
            "keepouts": [
                {"id": "cutter", "class": "hard", "rect": [0.0, 0.0, 18.0, 28.0]},
                {"id": "purge", "class": "soft", "frame": "gcode", "z_range_mm": [0.0, 0.2],
                 "strips": [[18.0, 1.0, 240.0, 1.0, 0.81]]},
                {"id": "ring", "class": "restricted", "frame": "gcode",
                 "polygon": [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]}
            ]
        }
    }));
    let rules = plate_rules(&declared, &SHIFTED).unwrap();
    assert_eq!(rules.body_edge_band_mm, 3.5);
    assert_eq!(rules.print_edge_reserve_mm, 0.0);
    assert_eq!(
        rules.keepouts[0].shape,
        KeepoutShape::Rect(Rect::new(0.0, 0.0, 18.0, 28.0))
    );
    assert_eq!(
        rules.keepouts[1].shape,
        KeepoutShape::Strips(vec![[18.0, 3.0, 240.0, 3.0, 0.81]])
    );
    let KeepoutShape::Polygon(ring) = &rules.keepouts[2].shape else {
        panic!("polygon expected");
    };
    assert_eq!((ring[2].x, ring[2].y), (10.0, 12.0));
    assert!(
        rules
            .keepouts
            .iter()
            .all(|k| k.frame == KeepoutFrame::Plate)
    );
}

#[test]
fn a_snapmaker_orca_gui_widens_the_body_band_to_its_own() {
    let narrow = profile(json!({"gui_flavour": "snapmaker-orca-2.4", "bed_rules": {}}));
    assert_eq!(
        plate_rules(&narrow, &SHIFTED).unwrap().body_edge_band_mm,
        3.5
    );
    let wide = profile(json!({"gui_flavour": "snapmaker-orca-2.4",
                              "bed_rules": {"body_edge_band_mm": 4.0}}));
    assert_eq!(plate_rules(&wide, &SHIFTED).unwrap().body_edge_band_mm, 4.0);
    let bambu = profile(json!({"gui_flavour": "bambu-02.08", "bed_rules": {}}));
    assert_eq!(
        plate_rules(&bambu, &SHIFTED).unwrap().body_edge_band_mm,
        0.0
    );
    // The GUI band applies to old profiles too.
    let legacy = profile(json!({"gui_flavour": "snapmaker-orca-2.4"}));
    assert_eq!(
        plate_rules(&legacy, &SHIFTED).unwrap().body_edge_band_mm,
        3.5
    );
}

#[test]
fn project_bed_reads_the_extruder_offset_and_the_first_layer() {
    let settings = json!({"extruder_offset": ["0x2"], "initial_layer_print_height": "0.25"});
    let project = ProjectBed::from_settings(&settings).unwrap();
    assert_eq!(project.gcode_offset, (0.0, 2.0));
    assert_eq!(project.first_layer_height_mm, Some(0.25));
    let bare = ProjectBed::from_settings(&json!({})).unwrap();
    assert_eq!(bare, ProjectBed::default());
    let listed = json!({"initial_layer_print_height": ["0.2"]});
    assert_eq!(
        ProjectBed::from_settings(&listed)
            .unwrap()
            .first_layer_height_mm,
        Some(0.2)
    );
}

/// The old fields are what the webapp and `source_project` read: they must agree with the
/// declared rules, and may only be stricter where the legacy path needs its own reserve.
#[test]
fn old_fields_must_say_what_the_rules_say() {
    let consistent = profile(json!({
        "edge_margin_mm": 3.5, "support_edge_margin_mm": 0.7,
        "exclusion_rectangles": [[0.0, 0.0, 18.0, 28.0]],
        "purge_lines": [[18.0, 1.0, 240.0, 1.0, 0.81]],
        "gui_flavour": "snapmaker-orca-2.4",
        "bed_rules": {"keepouts": [
            {"id": "cutter", "class": "hard", "rect": [0.0, 0.0, 18.0, 28.0]},
            {"id": "purge", "class": "soft", "frame": "gcode",
             "strips": [[18.0, 1.0, 240.0, 1.0, 0.81]]}
        ]}
    }));
    assert_eq!(old_field_mismatches(&consistent), Vec::<String>::new());

    let mut stale_margin = consistent.clone();
    stale_margin.edge_margin_mm = 4.0;
    let mut stale_purge = consistent.clone();
    stale_purge.purge_lines[0][4] = 0.5;
    let mut stale_exclusions = consistent.clone();
    stale_exclusions.exclusion_rectangles.clear();
    let mut loose_support = consistent.clone();
    if let Some(rules) = loose_support.bed_rules.as_mut() {
        rules.print_edge_reserve_mm = 1.0;
    }
    for stale in [stale_margin, stale_purge, stale_exclusions, loose_support] {
        assert_eq!(old_field_mismatches(&stale).len(), 1, "{stale:?}");
    }
    // Profiles without declared rules have nothing to disagree with.
    assert!(old_field_mismatches(&profile(json!({"edge_margin_mm": 5.0}))).is_empty());
}

#[test]
fn malformed_rules_are_refused() {
    let two_shapes = profile(json!({"bed_rules": {"keepouts": [
        {"id": "x", "class": "hard", "rect": [0.0, 0.0, 1.0, 1.0], "edge_band_mm": 2.0}
    ]}}));
    assert!(
        plate_rules(&two_shapes, &SHIFTED)
            .unwrap_err()
            .contains("exactly one")
    );
    let no_shape = profile(json!({"bed_rules": {"keepouts": [{"id": "x", "class": "soft"}]}}));
    assert!(plate_rules(&no_shape, &SHIFTED).is_err());
    let duplicate = profile(json!({"bed_rules": {"keepouts": [
        {"id": "x", "class": "hard", "rect": [0.0, 0.0, 1.0, 1.0]},
        {"id": "x", "class": "soft", "rect": [2.0, 0.0, 3.0, 1.0]}
    ]}}));
    assert!(
        plate_rules(&duplicate, &SHIFTED)
            .unwrap_err()
            .contains("duplicate")
    );
    let inverted = profile(json!({"bed_rules": {"keepouts": [
        {"id": "x", "class": "hard", "rect": [5.0, 0.0, 1.0, 1.0]}
    ]}}));
    assert!(plate_rules(&inverted, &SHIFTED).is_err());
    let shifted_band = profile(json!({"bed_rules": {"keepouts": [
        {"id": "x", "class": "hard", "frame": "gcode", "edge_band_mm": 5.0}
    ]}}));
    assert!(plate_rules(&shifted_band, &SHIFTED).is_err());
    let negative = profile(json!({"bed_rules": {"print_edge_reserve_mm": -1.0}}));
    assert!(plate_rules(&negative, &SHIFTED).is_err());
}

/// The zone check (and the packing core) only ever reads a gap of zero against profile zones:
/// the clearance and support gaps live in the tower columns.
#[test]
fn profile_zone_cells_forbid_with_a_zero_gap_only() {
    let profile_columns = [
        ZoneClass::Boundary,
        ZoneClass::Hard,
        ZoneClass::Restricted,
        ZoneClass::Soft,
    ];
    for kind in PrintKind::ALL {
        for zone in profile_columns {
            if let Permission::Forbidden { gap } = permission(kind, zone) {
                assert_eq!(gap, Gap::Zero, "{kind:?} against {zone:?}");
            }
        }
    }
}

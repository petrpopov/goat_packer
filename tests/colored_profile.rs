use goat_packer::colored_packer::load_printer_profile;

#[test]
fn p1s_profile_contains_blind_corner_and_purge_u_line() {
    let profile = load_printer_profile(std::path::Path::new("profiles/p1s.json")).unwrap();
    assert_eq!(profile.printer_name, "Bambu Lab P1S");
    assert_eq!(profile.edge_margin_mm, 0.0);
    assert_eq!(profile.exclusion_rectangles.len(), 1);
    assert_eq!(profile.purge_lines.len(), 4);
    assert!(!profile.blank_start_gcode_for_cli);
    // BambuStudio's CLI can report a false-positive "gcode path conflicts" error on dense
    // multi-color plates that our own validate_result/verify_outputs.py checks confirm have no
    // real overlap; --no-check (gated by this flag) skips that internal heuristic.
    assert!(profile.no_check_for_final_tower_probe);
}

#[test]
fn u1_profile_has_edge_margin_and_cli_workaround() {
    let profile = load_printer_profile(std::path::Path::new("profiles/snapmaker_u1.json")).unwrap();
    assert_eq!(profile.printer_name, "Snapmaker U1");
    // 4.0 (было 3.5): Orca Snapmaker при spiral lifting требует зазор строго больше 3.5 мм,
    // и пакер с margin=3.5 упирался в границу точно по порогу предупреждения.
    assert_eq!(profile.edge_margin_mm, 4.0);
    assert_eq!(profile.purge_lines.len(), 1);
    assert!(profile.blank_start_gcode_for_cli);
    assert!(profile.no_check_for_final_tower_probe);
    let usable = profile.usable_bed().unwrap();
    assert_eq!((usable.min_x, usable.min_y, usable.max_x, usable.max_y), (4.5, 5.0, 266.5, 267.0));
}

#[test]
fn detects_slicer_kind_from_known_printer_models() {
    use goat_packer::colored_packer::detect_slicer_kind_from_printer_model;
    assert_eq!(
        detect_slicer_kind_from_printer_model("Bambu Lab P1S"),
        Some("bambu")
    );
    assert_eq!(
        detect_slicer_kind_from_printer_model("Snapmaker U1"),
        Some("orca")
    );
    assert_eq!(detect_slicer_kind_from_printer_model("Prusa MK4"), None);
}

#[test]
fn profiles_carry_real_filament_change_duration() {
    let load = |path: &str| load_printer_profile(std::path::Path::new(path)).unwrap();
    // gen-1 AMS measured on koza-mini: A2L 23 + 72, P1S 57 + 34 (spec 2026-10-26).
    assert_eq!(load("profiles/bambu_a2l.json").filament_change_seconds, Some(95.0));
    assert_eq!(load("profiles/p1s.json").filament_change_seconds, Some(91.0));
    // U1 is a tool changer: no AMS change correction.
    assert_eq!(load("profiles/snapmaker_u1.json").filament_change_seconds, None);
}

mod cli_settings {
    use goat_packer::colored_packer::{
        apply_cli_settings_workarounds, load_printer_profile, normalize_flush_volumes_matrix,
    };
    use serde_json::json;

    #[test]
    fn normalize_flush_volumes_matrix_shrinks_a_stale_larger_matrix() {
        let mut settings = json!({
            "filament_colour": ["#C6001A", "#CBC6B8"],
            "flush_volumes_matrix": ["0", "214", "268", "381", "613", "0", "323", "558", "601",
                "222", "0", "442", "592", "280", "298", "0"],
        });
        assert!(normalize_flush_volumes_matrix(&mut settings));
        assert_eq!(settings["flush_volumes_matrix"], json!(["0", "214", "613", "0"]));
    }

    #[test]
    fn normalize_flush_volumes_matrix_grows_a_stale_smaller_matrix_with_zeros() {
        let mut settings = json!({
            "filament_colour": ["#C6001A", "#CBC6B8", "#FFFFFF"],
            "flush_volumes_matrix": ["0", "214", "613", "0"],
        });
        assert!(normalize_flush_volumes_matrix(&mut settings));
        assert_eq!(
            settings["flush_volumes_matrix"],
            json!(["0", "214", "0", "613", "0", "0", "0", "0", "0"])
        );
    }

    #[test]
    fn normalize_flush_volumes_matrix_is_a_noop_when_already_consistent() {
        let mut settings = json!({
            "filament_colour": ["#C6001A"],
            "flush_volumes_matrix": ["0"],
        });
        assert!(!normalize_flush_volumes_matrix(&mut settings));
    }

    #[test]
    fn normalize_flush_volumes_matrix_is_a_noop_without_filament_colour() {
        let mut settings = json!({
            "flush_volumes_matrix": ["0", "214", "268", "381", "613", "0", "323", "558", "601",
                "222", "0", "442", "592", "280", "298", "0"],
        });
        assert!(!normalize_flush_volumes_matrix(&mut settings));
    }

    #[test]
    fn cli_workarounds_blank_start_gcode_only_when_the_profile_asks_and_always_fix_the_matrix() {
        let p1s = load_printer_profile(std::path::Path::new("profiles/p1s.json")).unwrap();
        let u1 = load_printer_profile(std::path::Path::new("profiles/snapmaker_u1.json")).unwrap();
        let stale = json!({
            "machine_start_gcode": "G28",
            "filament_colour": ["#C6001A"],
            "flush_volumes_matrix": ["0", "1", "2", "0"],
        });

        let mut settings = stale.clone();
        assert!(apply_cli_settings_workarounds(&p1s, &mut settings));
        assert_eq!(settings["machine_start_gcode"], json!("G28"));
        assert_eq!(settings["flush_volumes_matrix"], json!(["0"]));

        let mut settings = stale.clone();
        assert!(apply_cli_settings_workarounds(&u1, &mut settings));
        assert_eq!(settings["machine_start_gcode"], json!(""));
        assert_eq!(settings["flush_volumes_matrix"], json!(["0"]));

        let mut consistent = json!({ "filament_colour": ["#C6001A"], "flush_volumes_matrix": ["0"] });
        assert!(!apply_cli_settings_workarounds(&p1s, &mut consistent));
    }
}

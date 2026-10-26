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

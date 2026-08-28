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
    assert_eq!(profile.edge_margin_mm, 3.5);
    assert_eq!(profile.purge_lines.len(), 1);
    assert!(profile.blank_start_gcode_for_cli);
    assert!(profile.no_check_for_final_tower_probe);
    let usable = profile.usable_bed().unwrap();
    assert_eq!((usable.min_x, usable.min_y, usable.max_x, usable.max_y), (4.0, 4.5, 267.0, 267.5));
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

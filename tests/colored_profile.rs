use goat_packer::colored_packer::load_printer_profile;

fn fixture(name: &str) -> goat_packer::colored_packer::PrinterProfile {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profiles").join(name);
    load_printer_profile(&path).unwrap()
}

#[test]
fn profile_loads_its_exclusion_corner_and_purge_line() {
    let profile = fixture("corner_bed.json");
    assert_eq!(profile.edge_margin_mm, 0.0);
    assert_eq!(profile.exclusion_rectangles.len(), 1);
    assert_eq!(profile.purge_lines.len(), 4);
    assert!(!profile.blank_start_gcode_for_cli);
    assert!(profile.no_check_for_final_tower_probe);
    assert_eq!(profile.filament_change_seconds, Some(91.0));
    assert_eq!(profile.support_bed().unwrap(), profile.usable_bed().unwrap());
}

#[test]
fn edge_margin_shrinks_the_usable_bed_and_a_support_margin_only_the_support_bed() {
    let profile = fixture("margin_bed.json");
    let usable = profile.usable_bed().unwrap();
    assert_eq!((usable.min_x, usable.min_y, usable.max_x, usable.max_y), (4.5, 5.0, 266.5, 267.0));
    let support = profile.support_bed().unwrap();
    assert_eq!((support.min_x, support.min_y, support.max_x, support.max_y), (1.2, 1.7, 269.8, 270.3));
    // A tool changer: no AMS change correction.
    assert_eq!(profile.filament_change_seconds, None);
}

#[test]
fn detects_slicer_kind_from_the_vendor_in_printer_model() {
    use goat_packer::colored_packer::detect_slicer_kind_from_printer_model;
    assert_eq!(detect_slicer_kind_from_printer_model("Bambu Lab X"), Some("bambu"));
    assert_eq!(detect_slicer_kind_from_printer_model("Unknown Printer"), None);
}

/// Every `profiles/*.json` (hand-made and catalog-generated) must load, and ids / printer names
/// must be unique: the webapp detects a project's profile by `printer_name`, so two profiles for
/// one printer would make detection ambiguous.
#[test]
fn every_repo_profile_loads_with_unique_id_and_printer_name() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles");
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let profile = load_printer_profile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let id = raw["id"].as_str().unwrap_or_else(|| panic!("{}: no id", path.display())).to_owned();
        assert!(ids.insert(id.clone()), "duplicate id {id}");
        assert!(names.insert(profile.printer_name.clone()), "duplicate printer_name {}", profile.printer_name);
        count += 1;
    }
    assert!(count > 0, "no profiles in {}", dir.display());
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
        let fixture = |name: &str| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profiles").join(name);
            load_printer_profile(&path).unwrap()
        };
        let keeps_start_gcode = fixture("corner_bed.json");
        let blanks_start_gcode = fixture("margin_bed.json");
        let stale = json!({
            "machine_start_gcode": "G28",
            "filament_colour": ["#C6001A"],
            "flush_volumes_matrix": ["0", "1", "2", "0"],
        });

        let mut settings = stale.clone();
        assert!(apply_cli_settings_workarounds(&keeps_start_gcode, &mut settings));
        assert_eq!(settings["machine_start_gcode"], json!("G28"));
        assert_eq!(settings["flush_volumes_matrix"], json!(["0"]));

        let mut settings = stale.clone();
        assert!(apply_cli_settings_workarounds(&blanks_start_gcode, &mut settings));
        assert_eq!(settings["machine_start_gcode"], json!(""));
        assert_eq!(settings["flush_volumes_matrix"], json!(["0"]));

        let mut consistent = json!({ "filament_colour": ["#C6001A"], "flush_volumes_matrix": ["0"] });
        assert!(!apply_cli_settings_workarounds(&keeps_start_gcode, &mut consistent));
    }
}

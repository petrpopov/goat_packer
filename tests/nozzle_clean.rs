use goat_packer::{
    colored_packer::PrinterProfile,
    nozzle_clean::{NozzleCleanSpec, compose_layer_change_gcode, inject_nozzle_clean},
};
use std::fs;
use std::io::{Read, Write};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

fn test_profile(nozzle_clean_gcode: Option<&str>) -> PrinterProfile {
    PrinterProfile {
        id: "test_printer".to_owned(),
        printer_name: "Test Printer".to_owned(),
        slicer_kind: "bambu".to_owned(),
        slicer_path: "/usr/bin/true".to_owned(),
        blank_start_gcode_for_cli: false,
        no_check_for_final_tower_probe: false,
        printable_area: [0.0, 0.0, 256.0, 256.0],
        edge_margin_mm: 0.0,
        exclusion_rectangles: vec![],
        purge_lines: vec![],
        tower_marker_style: "wipe_tower".to_owned(),
        tower_line_width_mm: 0.5,
        nozzle_clean_gcode: nozzle_clean_gcode.map(str::to_owned),
        filament_change_seconds: None,
    }
}

#[test]
fn parses_valid_specs() {
    assert_eq!(NozzleCleanSpec::parse("global").unwrap(), NozzleCleanSpec::Global { every: 10 });
    assert_eq!(
        NozzleCleanSpec::parse("global,every=5").unwrap(),
        NozzleCleanSpec::Global { every: 5 }
    );
    assert_eq!(
        NozzleCleanSpec::parse("from-z=6.6").unwrap(),
        NozzleCleanSpec::FromZ { z_mm: 6.6, every: 1 }
    );
    assert_eq!(
        NozzleCleanSpec::parse("from-z=6.6,every=2").unwrap(),
        NozzleCleanSpec::FromZ { z_mm: 6.6, every: 2 }
    );
    assert_eq!(
        NozzleCleanSpec::parse(" from-z=12 , every=3 ").unwrap(),
        NozzleCleanSpec::FromZ { z_mm: 12.0, every: 3 }
    );
}

#[test]
fn rejects_invalid_specs_with_usage() {
    for bad in [
        "",
        "banana",
        "global,every=0",
        "global,every=x",
        "global,foo=1",
        "from-z=-1",
        "from-z=abc",
        "from-z=",
    ] {
        let error = NozzleCleanSpec::parse(bad).unwrap_err();
        assert!(
            error.contains("nozzle-clean"),
            "error for {bad:?} should name the flag: {error}"
        );
        assert!(
            error.contains("expected"),
            "error for {bad:?} should list valid forms: {error}"
        );
    }
}

#[test]
fn composes_from_zone_every_layer() {
    let existing = "M73 L{layer_num+1}\nM991 S0 P{layer_num}";
    let composed = compose_layer_change_gcode(
        existing,
        &NozzleCleanSpec::FromZ { z_mm: 6.6, every: 1 },
        "WIPE_BODY",
    );
    assert!(composed.starts_with(";### START: NOZZLE CLEAN ###\n"));
    assert!(composed.contains("{if layer_num > 0}\n"));
    // every == 1 collapses the modulo condition entirely.
    assert!(!composed.contains('%'));
    // 0.01 float epsilon below the requested height.
    assert!(composed.contains("{if layer_z >= 6.59}\n"));
    assert!(composed.contains("WIPE_BODY\n"));
    assert_eq!(composed.matches("{endif}").count(), 2);
    // The printer-native tail survives, after the wipe block.
    let end = composed.rfind(";### END: NOZZLE CLEAN ###").unwrap();
    assert!(composed[end..].contains("M73 L{layer_num+1}"));
    assert!(composed[end..].contains("M991 S0 P{layer_num}"));
}

#[test]
fn composes_global_every_ten() {
    let composed = compose_layer_change_gcode(
        "M73 L{layer_num+1}",
        &NozzleCleanSpec::Global { every: 10 },
        "WIPE_BODY",
    );
    assert!(composed.contains("{if (layer_num + 1) % 10 == 0}\n"));
    assert!(!composed.contains("layer_z"));
    assert_eq!(composed.matches("{endif}").count(), 2);
}

#[test]
fn composes_zone_with_stride_and_trims_numbers() {
    let composed = compose_layer_change_gcode(
        "",
        &NozzleCleanSpec::FromZ { z_mm: 7.0, every: 3 },
        "WIPE_BODY",
    );
    assert!(composed.contains("{if (layer_num + 1) % 3 == 0}\n"));
    // 7.0 - 0.01 formats without trailing zeros.
    assert!(composed.contains("{if layer_z >= 6.99}\n"));
    assert_eq!(composed.matches("{endif}").count(), 3);
    // No existing content: nothing is appended after the END marker.
    assert!(composed.ends_with(";### END: NOZZLE CLEAN ###"));
}

fn write_synthetic_3mf(path: &std::path::Path, layer_change: Option<&str>) {
    let file = fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    let mut settings = String::from(r#"{"printer_model":"Bambu Lab P1S""#);
    if let Some(gcode) = layer_change {
        settings.push_str(&format!(r#","layer_change_gcode":{}"#, serde_json::json!(gcode)));
    }
    settings.push('}');
    zip.start_file("Metadata/project_settings.config", options).unwrap();
    zip.write_all(settings.as_bytes()).unwrap();
    zip.start_file("3D/3dmodel.model", options).unwrap();
    zip.write_all(b"<model/> untouched bytes").unwrap();
    zip.finish().unwrap();
}

fn read_entry(path: &std::path::Path, name: &str) -> Vec<u8> {
    let file = fs::File::open(path).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    let mut entry = archive.by_name(name).unwrap();
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).unwrap();
    bytes
}

#[test]
fn injects_and_preserves_other_entries() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.3mf");
    let output = dir.path().join("output.3mf");
    write_synthetic_3mf(&source, Some("M73 L{layer_num+1}"));

    let profile = test_profile(Some("M400\nG1 Z{layer_z} F1200"));
    inject_nozzle_clean(&profile, &source, &output, &NozzleCleanSpec::FromZ { z_mm: 6.6, every: 1 })
        .unwrap();

    let settings: serde_json::Value =
        serde_json::from_slice(&read_entry(&output, "Metadata/project_settings.config")).unwrap();
    let gcode = settings["layer_change_gcode"].as_str().unwrap();
    assert!(gcode.contains("M400\nG1 Z{layer_z} F1200"));
    assert!(gcode.contains("{if layer_z >= 6.59}"));
    assert!(gcode.ends_with("M73 L{layer_num+1}"));
    // Unrelated settings survive the serde_json round-trip.
    assert_eq!(settings["printer_model"].as_str().unwrap(), "Bambu Lab P1S");
    // Non-settings entries pass through byte-for-byte.
    assert_eq!(
        read_entry(&output, "3D/3dmodel.model"),
        b"<model/> untouched bytes"
    );
}

#[test]
fn injects_in_place_and_creates_missing_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inplace.3mf");
    write_synthetic_3mf(&path, None);

    let profile = test_profile(Some("M400"));
    inject_nozzle_clean(&profile, &path, &path, &NozzleCleanSpec::Global { every: 10 }).unwrap();

    let settings: serde_json::Value =
        serde_json::from_slice(&read_entry(&path, "Metadata/project_settings.config")).unwrap();
    assert!(settings["layer_change_gcode"]
        .as_str()
        .unwrap()
        .contains("{if (layer_num + 1) % 10 == 0}"));
}

#[test]
fn errors_when_profile_lacks_wipe_body() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.3mf");
    let output = dir.path().join("output.3mf");
    write_synthetic_3mf(&source, Some("M73"));

    let error = inject_nozzle_clean(
        &test_profile(None),
        &source,
        &output,
        &NozzleCleanSpec::Global { every: 10 },
    )
    .unwrap_err();
    assert!(error.contains("test_printer"), "error names the profile: {error}");
    assert!(error.contains("nozzle_clean_gcode"), "error names the missing field: {error}");
}

#[test]
fn errors_when_project_has_no_settings_entry() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.3mf");
    let file = fs::File::create(&source).unwrap();
    let mut zip = ZipWriter::new(file);
    zip.start_file("3D/3dmodel.model", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"<model/>").unwrap();
    zip.finish().unwrap();

    let error = inject_nozzle_clean(
        &test_profile(Some("M400")),
        &source,
        &dir.path().join("output.3mf"),
        &NozzleCleanSpec::Global { every: 10 },
    )
    .unwrap_err();
    assert!(error.contains("project_settings.config"), "error names the missing entry: {error}");
}

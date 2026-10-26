use std::{io::Read, path::Path};

use goat_packer::{
    ams_calibration::{
        BandPlan, CUBE_SIDE_MM, TWO_FILAMENTS_ERROR, Template, Variant, band_plan,
        bambu_filament_change_times, cube_footprint, cube_mesh, custom_gcode_per_layer_xml,
        instructions_markdown, model_settings_xml, model_xml, parse_template_settings, place_cube, plan_projects, write_calibration_3mf,
    },
    colored_packer::{PrinterProfile, load_printer_profile},
    print_time::orca_toolchange_count,
};

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

fn assert_all_close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
    for (a, e) in actual.iter().zip(expected) {
        assert_close(*a, *e);
    }
}

fn profile(name: &str) -> PrinterProfile {
    load_printer_profile(&Path::new("profiles").join(name)).unwrap()
}

fn settings_json(extra: serde_json::Value) -> Vec<u8> {
    let mut settings = serde_json::json!({
        "printer_model": "Bambu Lab P1S",
        "layer_height": "0.28",
        "initial_layer_print_height": "0.2",
        "filament_colour": ["#FFFFFF", "#000000"],
        "enable_prime_tower": "1",
        "wipe_tower_x": ["221", "221"],
        "wipe_tower_y": ["207.25", "207.25"],
        "prime_tower_width": "20",
    });
    for (key, value) in extra.as_object().unwrap() {
        settings[key] = value.clone();
    }
    serde_json::to_vec(&settings).unwrap()
}

#[test]
fn variants_parse_and_carry_counts() {
    assert_eq!(Variant::parse("normal").unwrap(), Variant::Normal);
    assert_eq!(Variant::parse("perfectionist").unwrap(), Variant::Perfectionist);
    assert!(Variant::parse("fast").is_err());
    assert_eq!(Variant::Normal.changes(), 10);
    assert_eq!(Variant::Normal.layers_per_band(), 4);
    assert_eq!(Variant::Perfectionist.changes(), 30);
    assert_eq!(Variant::Perfectionist.layers_per_band(), 2);
}

#[test]
fn band_plan_uniform_layers() {
    let normal = band_plan(0.2, 0.2, Variant::Normal).unwrap();
    // first layer + 10 bands x 4 layers x 0.2
    assert_close(normal.height_mm, 8.2);
    assert_eq!(normal.change_z.len(), 10);
    // Each change sits at the print_z of the first layer of its band.
    assert_all_close(
        &normal.change_z,
        &[0.4, 1.2, 2.0, 2.8, 3.6, 4.4, 5.2, 6.0, 6.8, 7.6],
    );

    let perfectionist = band_plan(0.2, 0.2, Variant::Perfectionist).unwrap();
    assert_close(perfectionist.height_mm, 12.2);
    assert_eq!(perfectionist.change_z.len(), 30);
    assert_close(perfectionist.change_z[0], 0.4);
    assert_close(perfectionist.change_z[1], 0.8);
    assert_close(perfectionist.change_z[29], 12.0);
}

#[test]
fn band_plan_thicker_layers_with_thin_first_layer() {
    let normal = band_plan(0.2, 0.28, Variant::Normal).unwrap();
    assert_close(normal.height_mm, 11.4);
    assert_all_close(
        &normal.change_z,
        &[0.48, 1.6, 2.72, 3.84, 4.96, 6.08, 7.2, 8.32, 9.44, 10.56],
    );

    let perfectionist = band_plan(0.2, 0.28, Variant::Perfectionist).unwrap();
    assert_close(perfectionist.height_mm, 17.0);
    assert_eq!(perfectionist.change_z.len(), 30);
    assert_close(perfectionist.change_z[0], 0.48);
    assert_close(perfectionist.change_z[1], 1.04);
    // The last band (2 layers) starts one band below the top: 17.0 - 0.56 + 0.28.
    assert_close(perfectionist.change_z[29], 16.72);
    // Every change lies strictly inside the cube.
    assert!(perfectionist.change_z.iter().all(|z| *z < perfectionist.height_mm));
}

#[test]
fn band_plan_rejects_non_positive_layers() {
    assert!(band_plan(0.0, 0.2, Variant::Normal).is_err());
    assert!(band_plan(0.2, -0.1, Variant::Normal).is_err());
}

fn plan_for(z: &[f64]) -> BandPlan {
    BandPlan {
        first_layer_mm: 0.2,
        layer_height_mm: 0.2,
        layers_per_band: 4,
        height_mm: 10.0,
        change_z: z.to_vec(),
    }
}

#[test]
fn custom_gcode_xml_alternates_extruders_with_exact_count() {
    for variant in [Variant::Normal, Variant::Perfectionist] {
        let plan = band_plan(0.2, 0.28, variant).unwrap();
        let colours = vec!["#FFFFFF".to_owned(), "#000000".to_owned(), "#C12E1F".to_owned()];
        let xml = custom_gcode_per_layer_xml(&plan, &colours);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<custom_gcodes_per_layer>"));
        assert!(xml.contains("<plate_info id=\"1\"/>"));
        assert!(xml.contains("<mode value=\"MultiAsSingle\"/>"));
        let layers: Vec<&str> = xml.lines().filter(|line| line.starts_with("<layer ")).collect();
        assert_eq!(layers.len(), variant.changes());
        for (index, line) in layers.iter().enumerate() {
            let (extruder, colour) = if index % 2 == 0 { (2, "#000000") } else { (1, "#FFFFFF") };
            assert!(line.contains(&format!("extruder=\"{extruder}\"")), "{line}");
            assert!(line.contains(&format!("color=\"{colour}\"")), "{line}");
            assert!(line.contains("type=\"2\""), "{line}");
            assert!(line.contains("gcode=\"tool_change\""), "{line}");
        }
        assert!(layers[0].contains("top_z=\"0.48\""), "{}", layers[0]);
    }
}

#[test]
fn custom_gcode_xml_prints_short_decimals() {
    let xml = custom_gcode_per_layer_xml(&plan_for(&[0.1 + 0.2, 14.599999999999998]), &[
        "#FFFFFF".to_owned(),
        "#000000".to_owned(),
    ]);
    assert!(xml.contains("top_z=\"0.3\""), "{xml}");
    assert!(xml.contains("top_z=\"14.6\""), "{xml}");
}

#[test]
fn cube_mesh_has_expected_bbox_and_outward_faces() {
    let (vertices, triangles) = cube_mesh(CUBE_SIDE_MM, 11.4);
    assert_eq!(vertices.len(), 8);
    assert_eq!(triangles.len(), 12);
    let min = |axis: usize| vertices.iter().map(|v| v[axis]).fold(f64::INFINITY, f64::min);
    let max = |axis: usize| vertices.iter().map(|v| v[axis]).fold(f64::NEG_INFINITY, f64::max);
    assert_eq!((min(0), max(0)), (-10.0, 10.0));
    assert_eq!((min(1), max(1)), (-10.0, 10.0));
    assert_eq!((min(2), max(2)), (0.0, 11.4));
    // Signed volume (divergence theorem) is positive iff every face points outward.
    let volume: f64 = triangles
        .iter()
        .map(|[a, b, c]| {
            let (a, b, c) = (vertices[*a], vertices[*b], vertices[*c]);
            a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0])
        })
        .sum::<f64>()
        / 6.0;
    assert_close(volume, 20.0 * 20.0 * 11.4);
}

#[test]
fn model_xml_contains_mesh_and_build_item() {
    let xml = model_xml("BambuStudio-02.08.02.61", 11.4, (128.0, 128.0));
    assert!(xml.contains("<metadata name=\"Application\">BambuStudio-02.08.02.61</metadata>"));
    assert!(xml.contains("<metadata name=\"BambuStudio:3mfVersion\">1</metadata>"));
    assert_eq!(xml.matches("<vertex ").count(), 8);
    assert_eq!(xml.matches("<triangle ").count(), 12);
    assert!(xml.contains("z=\"11.4\""));
    assert!(xml.contains("<item objectid=\"1\" transform=\"1 0 0 0 1 0 0 0 1 128 128 0\" printable=\"1\"/>"));
    let settings = model_settings_xml((128.0, 128.0));
    assert!(settings.contains("<metadata key=\"extruder\" value=\"1\"/>"));
    assert!(settings.contains("transform=\"1 0 0 0 1 0 0 0 1 128 128 0\""));
}

#[test]
fn template_with_one_filament_is_rejected() {
    let bytes = settings_json(serde_json::json!({ "filament_colour": ["#FFFFFF"] }));
    let error = parse_template_settings(&bytes).unwrap_err();
    assert_eq!(error.to_string(), TWO_FILAMENTS_ERROR);
    let bytes = settings_json(serde_json::json!({ "filament_colour": null }));
    assert_eq!(
        parse_template_settings(&bytes).unwrap_err().to_string(),
        TWO_FILAMENTS_ERROR
    );
}

#[test]
fn template_settings_read_layers_and_tower() {
    let settings = parse_template_settings(&settings_json(serde_json::json!({}))).unwrap();
    assert_close(settings.layer_height_mm, 0.28);
    assert_close(settings.first_layer_mm, 0.2);
    assert_eq!(settings.filament_colours.len(), 2);
    let tower = settings.tower.unwrap();
    assert_close(tower.min_x, 221.0);
    assert_close(tower.min_y, 207.25);
    assert_close(tower.max_x, 241.0);
    let disabled = settings_json(serde_json::json!({ "enable_prime_tower": "0" }));
    assert!(parse_template_settings(&disabled).unwrap().tower.is_none());
}

#[test]
fn template_settings_slicer_change_seconds_branches() {
    let change = |extra: serde_json::Value| {
        parse_template_settings(&settings_json(extra))
            .unwrap()
            .slicer_change_s
    };
    let ams = serde_json::json!({
        "ams_filament_load_time_ams": "23",
        "ams_filament_unload_time_ams": "21",
        "ams_filament_load_time_ams_lite": "11",
        "ams_filament_unload_time_ams_lite": ["12"],
        "ams_filament_load_time_n3f_s": "18",
        "ams_filament_unload_time_n3f_s": "19",
        "machine_load_filament_time": "29",
        "machine_unload_filament_time": "28",
    });
    let with_type = |ams_type: serde_json::Value| {
        let mut extra = ams.clone();
        extra["default_ams_type"] = ams_type;
        change(extra)
    };
    assert_close(with_type(serde_json::json!("1")), 44.0);
    assert_close(with_type(serde_json::json!(["2"])), 23.0);
    assert_close(with_type(serde_json::json!("3")), 37.0);
    // P1S: default_ams_type -1 (or absent) falls back to the machine load/unload pair.
    assert_close(with_type(serde_json::json!("-1")), 57.0);
    assert_close(change(ams.clone()), 57.0);
    assert_close(change(serde_json::json!({})), 0.0);
}

#[test]
fn cube_is_centred_when_tower_is_clear() {
    let p1s = profile("p1s.json");
    let settings = parse_template_settings(&settings_json(serde_json::json!({}))).unwrap();
    let centre = place_cube(&p1s, settings.tower).unwrap();
    assert_eq!(centre, (128.0, 128.0));
}

#[test]
fn cube_moves_away_from_overlapping_tower() {
    let a2l = profile("bambu_a2l.json");
    // A2L template: tower corner at the bed centre, 35 mm wide.
    let bytes = settings_json(serde_json::json!({
        "wipe_tower_x": ["164.898", "153"],
        "wipe_tower_y": ["141.172", "160"],
        "prime_tower_width": "35",
    }));
    let settings = parse_template_settings(&bytes).unwrap();
    let centre = place_cube(&a2l, settings.tower).unwrap();
    // Tower lies right of the centre, so the cube moves left by 35 + 10 mm.
    assert_eq!(centre, (120.0, 160.0));
    let cube = cube_footprint(centre);
    assert!(cube.max_x < settings.tower.unwrap().min_x);
}

#[test]
fn instructions_mention_variant_accuracy() {
    let plan = band_plan(0.2, 0.28, Variant::Normal).unwrap();
    let normal = instructions_markdown("Bambu Lab P1S", Variant::Normal, &plan);
    assert!(normal.contains("±12 с"), "{normal}");
    assert!(normal.contains("10 смен"));
    assert!(normal.contains("Bambu Lab P1S"));
    assert!(normal.contains("11.4 мм"));
    assert!(normal.contains("«Печать завершена»"));
    let plan = band_plan(0.2, 0.28, Variant::Perfectionist).unwrap();
    let perfectionist = instructions_markdown("Bambu Lab P1S", Variant::Perfectionist, &plan);
    assert!(perfectionist.contains("±4 с"));
    assert!(perfectionist.contains("30 смен"));
}

fn zip_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (entry.name().to_owned(), bytes)
        })
        .collect()
}

#[test]
fn calibration_projects_are_written_with_verbatim_settings() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_json(serde_json::json!({}));
    let template = Template {
        project_settings: settings.clone(),
        application: "BambuStudio-02.08.02.61".to_owned(),
    };
    let projects = plan_projects(&template, &profile("p1s.json"), Variant::Normal).unwrap();
    let a = dir.path().join("calib_A.3mf");
    let b = dir.path().join("calib_B.3mf");
    write_calibration_3mf(&a, &settings, &projects.model, &projects.model_settings, None).unwrap();
    write_calibration_3mf(
        &b,
        &settings,
        &projects.model,
        &projects.model_settings,
        Some(&projects.custom_gcode),
    )
    .unwrap();
    let names = |entries: &[(String, Vec<u8>)]| {
        entries.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>()
    };
    let a_entries = zip_entries(&a);
    let b_entries = zip_entries(&b);
    assert_eq!(
        names(&a_entries),
        [
            "[Content_Types].xml",
            "_rels/.rels",
            "3D/3dmodel.model",
            "Metadata/project_settings.config",
            "Metadata/model_settings.config",
        ]
    );
    assert_eq!(names(&b_entries).last().unwrap(), "Metadata/custom_gcode_per_layer.xml");
    for entries in [&a_entries, &b_entries] {
        let (_, bytes) = entries
            .iter()
            .find(|(name, _)| name == "Metadata/project_settings.config")
            .unwrap();
        assert_eq!(bytes, &settings);
    }
    // A and B carry the same cube.
    let model = |entries: &[(String, Vec<u8>)]| {
        entries.iter().find(|(name, _)| name == "3D/3dmodel.model").unwrap().1.clone()
    };
    assert_eq!(model(&a_entries), model(&b_entries));
}

#[test]
fn change_counts_are_read_from_slicer_output() {
    let result = br#"{"error_string":"Success.","sliced_plates":[{"filament_change_times":10,"total_predication":1783.3}]}"#;
    assert_eq!(bambu_filament_change_times(result).unwrap(), 10);
    assert!(bambu_filament_change_times(br#"{"sliced_plates":[]}"#).is_err());
    let gcode = "; start\n; toolchange #1\nT1\nG1 X1\n; toolchange #2\nT0\n;toolchange without space\n";
    assert_eq!(orca_toolchange_count(gcode), 2);
}

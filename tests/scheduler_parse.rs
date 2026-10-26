use goat_packer::scheduler::{parse_bambu_result, parse_duration_text, parse_orca_gcode};

#[test]
fn parses_bambu_native_result_times_and_material() {
    let input = br#"{
      "sliced_plates": [{
        "main_predication": 13387.5,
        "total_predication": 13762.8,
        "objects": [{}, {}, {}],
        "filaments": [{"total_used_g": 26.75}]
      }]
    }"#;
    let estimate = parse_bambu_result(input).unwrap();
    assert_eq!(estimate.object_count, 3);
    assert_eq!(estimate.model_seconds, 13_388);
    assert_eq!(estimate.slicer_total_seconds, 13_763);
    assert!((estimate.filament_grams - 26.75).abs() < 1e-9);
}

#[test]
fn parses_orca_gcode_estimate_and_material_from_binary_safe_text() {
    let input = b"\0PNG\0; estimated printing time (normal mode) = 3h 43m 9s\n\
                  ; filament used [g] = 168.42\n";
    let estimate = parse_orca_gcode(input, 19).unwrap();
    assert_eq!(estimate.object_count, 19);
    assert_eq!(estimate.model_seconds, 13_389);
    assert_eq!(estimate.slicer_total_seconds, 13_389);
    assert!((estimate.filament_grams - 168.42).abs() < 1e-9);
}

#[test]
fn parses_mixed_hour_minute_second_duration_text() {
    assert_eq!(parse_duration_text("3h 43m 9s").unwrap(), 13_389);
    assert_eq!(parse_duration_text("42m 5s").unwrap(), 2_525);
    assert_eq!(parse_duration_text("17s").unwrap(), 17);
}

#[test]
fn parses_bambu_filament_change_count() {
    let input = br#"{"sliced_plates": [{
        "main_predication": 100.0, "total_predication": 120.4,
        "filament_change_times": 227, "objects": [{}]
    }]}"#;
    assert_eq!(parse_bambu_result(input).unwrap().filament_changes, 227);
}

#[test]
fn bambu_result_without_change_count_means_zero_changes() {
    let input = br#"{"sliced_plates": [{
        "main_predication": 100.0, "total_predication": 120.4, "objects": [{}]
    }]}"#;
    assert_eq!(parse_bambu_result(input).unwrap().filament_changes, 0);
}

#[test]
fn counts_orca_toolchange_comments() {
    let input = b"; estimated printing time (normal mode) = 1h 0m 0s\n\
                  ; toolchange #1\nT1\n; toolchange #2\nT0\n; toolchange #3\nT1\n";
    assert_eq!(parse_orca_gcode(input, 1).unwrap().filament_changes, 3);
}

#[test]
fn orca_change_count_ignores_the_marker_inside_a_config_line() {
    // Orca appends its config block as `; key = value` lines; a change G-code template that
    // mentions the marker must not count as a change.
    let input = b"; estimated printing time (normal mode) = 1h 0m 0s\n\
                  ; toolchange #1\nT1\n\
                  ; change_filament_gcode = M620 S[next_extruder]A\\n; toolchange #2\\nT[next_extruder]\n";
    assert_eq!(parse_orca_gcode(input, 1).unwrap().filament_changes, 1);
}

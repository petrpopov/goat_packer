use goat_packer::colored_packer::parse_prime_tower_bounds;

#[test]
fn parses_brim_and_tower_extrusions_with_absolute_and_relative_moves() {
    let gcode = b"
G90
G1 X10 Y10 F1000
; WIPE_TOWER_BRIM_START
; LINE_WIDTH: 0.50
G1 X12 Y10 E0.2
G1 Y20 E0.2
G1 X10 E0.2
; WIPE_TOWER_BRIM_END
G91
; WIPE_TOWER_START
G1 X2 Y0 E0.1
G1 Y2 E0.1
G1 X99 Y99 F1000
; WIPE_TOWER_END
";
    let bounds = parse_prime_tower_bounds(gcode, 0.4, 0.7).unwrap();
    assert_eq!(bounds.centerline.min_x, 10.0);
    assert_eq!(bounds.centerline.min_y, 10.0);
    assert_eq!(bounds.centerline.max_x, 12.0);
    assert_eq!(bounds.centerline.max_y, 22.0);
    assert!((bounds.keepout.min_x - 5.05).abs() < 1e-9);
    assert!((bounds.keepout.max_y - 26.95).abs() < 1e-9);
}

#[test]
fn returns_none_without_prime_tower_markers() {
    assert!(parse_prime_tower_bounds(b"G90\nG1 X10 Y10 E1\n", 0.5, 0.7).is_none());
}

#[test]
fn keepout_adds_a_fixed_safety_margin_beyond_line_width_and_clearance() {
    let gcode = b"
G90
; WIPE_TOWER_START
G1 X0 Y0 E0.1
G1 X10 Y0 E0.1
G1 X10 Y5 E0.1
; WIPE_TOWER_END
";
    let bounds = parse_prime_tower_bounds(gcode, 0.5, 0.7).unwrap();
    assert_eq!(bounds.centerline.min_x, 0.0);
    assert_eq!(bounds.centerline.max_x, 10.0);
    assert_eq!(bounds.centerline.min_y, 0.0);
    assert_eq!(bounds.centerline.max_y, 5.0);
    // 0.5 * 0.5 (half line width) + 0.7 (clearance) + 4.0 (fixed safety margin) = 4.95
    let expected_expansion = 4.95;
    assert!((bounds.keepout.min_x - (0.0 - expected_expansion)).abs() < 1e-9);
    assert!((bounds.keepout.max_x - (10.0 + expected_expansion)).abs() < 1e-9);
    assert!((bounds.keepout.min_y - (0.0 - expected_expansion)).abs() < 1e-9);
    assert!((bounds.keepout.max_y - (5.0 + expected_expansion)).abs() < 1e-9);
}

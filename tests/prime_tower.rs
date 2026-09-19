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
    // 0.5 (parsed LINE_WIDTH) * 0.5 + 0.7 (clearance) + 1.0 (tower safety) = 1.95
    assert!((bounds.keepout.min_x - 8.05).abs() < 1e-9);
    assert!((bounds.keepout.max_y - 23.95).abs() < 1e-9);
}

#[test]
fn returns_none_without_prime_tower_markers() {
    assert!(parse_prime_tower_bounds(b"G90\nG1 X10 Y10 E1\n", 0.5, 0.7).is_none());
}

/// OrcaSlicer on IDEX printers (Snapmaker U1) wraps a full tool change inside the tower
/// section: the nozzle unload/wipe (`;WIPE_START` .. `;WIPE_END`) runs at a garbage point far
/// from the tower (half a plate away on a real lie_c.3mf plate), then the tower wipe zigzags
/// next to the tower itself. The bounds must come from the tower alone — including the wipe
/// points made the "tower" the size of the whole plate and forced a bogus re-pack.
#[test]
fn nozzle_wipe_inside_tool_change_is_not_part_of_the_tower() {
    let gcode = b"
G90
G1 X45 Y210 F30000
; WIPE_TOWER_START
; CP TOOLCHANGE UNLOAD
G1 E-9.85 F1800
;WIPE_START
G1 F12000
G1 X39.047 Y9.647 E-.07376
G1 X39.514 Y9.845 E-.06725
;WIPE_END
G1 F30000
G1 X49.750 Y218.250
G1 E10 F1800
; CP TOOLCHANGE WIPE
G1 X49.000 Y216.200 E0.0269
G1 X41.000 Y216.200 E0.2237
G1 X41.000 Y214.400 E0.0269
; WIPE_TOWER_END
";
    let bounds = parse_prime_tower_bounds(gcode, 0.5, 0.7).unwrap();
    assert_eq!(bounds.centerline.min_x, 41.0);
    assert_eq!(bounds.centerline.max_x, 49.0);
    assert_eq!(bounds.centerline.min_y, 214.4);
    assert_eq!(bounds.centerline.max_y, 216.2);
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
    // 0.5 * 0.5 (half line width) + 0.7 (clearance) + 1.0 (tower safety) = 1.95
    let expected_expansion = 1.95;
    assert!((bounds.keepout.min_x - (0.0 - expected_expansion)).abs() < 1e-9);
    assert!((bounds.keepout.max_x - (10.0 + expected_expansion)).abs() < 1e-9);
    assert!((bounds.keepout.min_y - (0.0 - expected_expansion)).abs() < 1e-9);
    assert!((bounds.keepout.max_y - (5.0 + expected_expansion)).abs() < 1e-9);
}

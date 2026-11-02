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

/// BambuStudio (A2L, 2026-10-26) spells the nozzle wipe marker with a space, `; WIPE_START`,
/// and runs the post-change retract-wipe at the OBJECT just printed, right after
/// `; WIPE_TOWER_START`. Counting those `G1 X.. Y.. E-1.9` points stretched
/// a ~27x24 mm koza tower (brim included) to an 84x92 mm keepout over the object (probe),
/// and across a 61-object plate (final) — a "tower" that moved with the objects.
#[test]
fn bambu_spaced_wipe_marker_inside_tower_section_is_not_part_of_the_tower() {
    let gcode = b"
G90
G1 X113.446 Y127.443
; FEATURE: Prime tower
; WIPE_TOWER_START
; WIPE_START
G1 F12000
G1 X113.446 Y127.443 E-1.9
G1 X118.0 Y128.0 E-0.2
; WIPE_END
G1 E-.1 F1800
G1 X172 Y182 F30000
G1 X170 Y180 E1.0
G1 X190 Y180 E1.0
G1 X190 Y198 E1.0
; WIPE_TOWER_END
";
    let bounds = parse_prime_tower_bounds(gcode, 0.5, 0.7).unwrap();
    assert_eq!(bounds.centerline.min_x, 170.0);
    assert_eq!(bounds.centerline.min_y, 180.0);
    assert_eq!(bounds.centerline.max_x, 190.0);
    assert_eq!(bounds.centerline.max_y, 198.0);
}

/// Controller repro (real A2L koza probe G-code): a retract-wipe `G1 X125.282 Y116.54 E-.4256`
/// right after `WIPE_TOWER_START`, at the object, while the tower sits at x=170..205 — here
/// without any wipe marker. Only moves that really extrude are the tower: relative E (M83,
/// the Bambu/Orca default) needs E > 0.
#[test]
fn retract_moves_with_negative_e_are_not_part_of_the_tower() {
    let gcode = b"
G90
M83
G1 X125 Y116 F30000
; WIPE_TOWER_START
G1 X125.282 Y116.54 E-.4256
G1 X170 Y180 F30000
G1 X170 Y181 E0.2
G1 X205 Y181 E1.0
G1 X205 Y200 E1.0
; WIPE_TOWER_END
";
    let bounds = parse_prime_tower_bounds(gcode, 0.5, 0.7).unwrap();
    assert_eq!(bounds.centerline.min_x, 170.0);
    assert_eq!(bounds.centerline.min_y, 181.0);
    assert_eq!(bounds.centerline.max_x, 205.0);
    assert_eq!(bounds.centerline.max_y, 200.0);
}

/// Absolute E (M82): a move extrudes when E rises past the previous value; `G92 E` resets the
/// reference. A retract (E falls) and a move right after a reset to the same value do not.
#[test]
fn absolute_e_mode_counts_only_rising_e_and_honours_g92() {
    let gcode = b"
G90
M82
G1 X10 Y10 E5.0
; WIPE_TOWER_START
G1 X125 Y116 E4.2
G1 X170 Y180 E4.2
G1 X190 Y180 E5.2
G92 E0
G1 X190 Y199 E0.8
G1 X60 Y60 E0.8
; WIPE_TOWER_END
";
    let bounds = parse_prime_tower_bounds(gcode, 0.5, 0.7).unwrap();
    assert_eq!(bounds.centerline.min_x, 190.0);
    assert_eq!(bounds.centerline.min_y, 180.0);
    assert_eq!(bounds.centerline.max_x, 190.0);
    assert_eq!(bounds.centerline.max_y, 199.0);
}

/// BambuStudio draws its rib-wall tower with G2/G3 fillet arcs. Reading G0/G1 only kept the
/// head at each arc's start and missed every bulge (a real P1S tower measured 16.3x14.6 mm
/// instead of 24.9x24.0). Here the tower is two half circles of radius 5 around (20, 15): one
/// in the I/J form, one in the R form; their ends sit at x = 20, their bulges reach 15 and 25.
#[test]
fn tower_arcs_count_with_their_bulges_in_both_forms() {
    let gcode = b"
G90
M83
G1 X20 Y10 F30000
; WIPE_TOWER_START
G3 X20 Y20 I0 J5 E0.5
G3 X20 Y10 R5 E0.5
; WIPE_TOWER_END
";
    let bounds = parse_prime_tower_bounds(gcode, 0.5, 0.7).unwrap();
    let centre = bounds.centerline;
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert!(
        near(centre.min_x, 15.0) && near(centre.max_x, 25.0),
        "{centre:?}"
    );
    assert!(
        near(centre.min_y, 10.0) && near(centre.max_y, 20.0),
        "{centre:?}"
    );
}

/// The R form picks the side of the centre from the direction and the sign: a clockwise arc
/// from (0, 0) to (10, 0) with R = 5 is the upper half circle (centre on the chord); with
/// R = -10 it is the long way round a centre above the chord.
#[test]
fn radius_form_arcs_take_the_side_their_direction_and_sign_give() {
    let tower = |arc: &str| {
        let gcode = format!("G90\nM83\nG1 X0 Y0\n; WIPE_TOWER_START\n{arc}\n; WIPE_TOWER_END\n");
        parse_prime_tower_bounds(gcode.as_bytes(), 0.5, 0.7)
            .unwrap()
            .centerline
    };
    let short = tower("G2 X10 Y0 R5 E1");
    assert!(
        (short.max_y - 5.0).abs() < 1e-9 && short.min_y.abs() < 1e-9,
        "{short:?}"
    );
    // Long clockwise arc, radius 10: the centre sits left of the travel direction at
    // (5, 8.66), and the 300-degree sweep passes the circle's left, top and right extremes.
    let long = tower("G2 X10 Y0 R-10 E1");
    assert!((long.max_y - 10.0 - 75f64.sqrt()).abs() < 1e-9, "{long:?}");
    assert!(
        (long.min_x + 5.0).abs() < 1e-9 && (long.max_x - 15.0).abs() < 1e-9,
        "{long:?}"
    );
    assert!(long.min_y.abs() < 1e-9, "{long:?}");
}

/// Relative positioning moves an arc's end like any other move; I/J stay relative to the start.
#[test]
fn relative_arcs_end_where_the_offsets_say() {
    let gcode = b"
G90
M83
G1 X30 Y30
G91
; WIPE_TOWER_START
G2 X10 Y0 I5 J0 E1
G1 X0 Y5 E1
; WIPE_TOWER_END
";
    let centre = parse_prime_tower_bounds(gcode, 0.5, 0.7)
        .unwrap()
        .centerline;
    // Clockwise from (30, 30) to (40, 30) around (35, 30): over the top to y = 35, then the
    // straight move to (40, 35).
    assert!((centre.max_y - 35.0).abs() < 1e-9, "{centre:?}");
    assert!((centre.min_x - 30.0).abs() < 1e-9 && (centre.max_x - 40.0).abs() < 1e-9);
    assert!((centre.min_y - 30.0).abs() < 1e-9, "{centre:?}");
}

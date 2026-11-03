//! Reading sliced plates the way the slicer GUIs judge them: object labels in both dialects
//! (including BambuStudio's unlabelled single-object plates), the tower, exclusions, the
//! centre-line bed tolerance, path conflicts and the support realization of one object.
//! Every G-code here is synthetic.

use goat_packer::gcode_collisions::gui_checks::{
    GUI_BED_EPSILON_MM, GuiRules, PartEdgeIssue, Party, check_plate, part_edge_hits,
    placed_part_boxes,
};
use goat_packer::gcode_collisions::{ConflictKind, analyze_plate};
use goat_packer::{
    ObjectPrint, Point, Rect, realization_from_gcode,
    realizations_from_gcode,
};

const BED: Rect = Rect {
    min_x: 0.0,
    min_y: 0.0,
    max_x: 256.0,
    max_y: 256.0,
};

fn rules(exclusions: Vec<Vec<Point>>) -> GuiRules {
    GuiRules {
        printable: BED,
        exclusions,
    }
}

/// A closed square walked with G1 moves, `E` 1 per move.
fn square_moves(cx: f64, cy: f64, half: f64) -> String {
    format!(
        "G1 X{} Y{} F600\nG1 X{} Y{} E1\nG1 X{} Y{} E1\nG1 X{} Y{} E1\nG1 X{} Y{} E1\n",
        cx - half,
        cy - half,
        cx + half,
        cy - half,
        cx + half,
        cy + half,
        cx - half,
        cy + half,
        cx - half,
        cy - half
    )
}

/// One layer of a BambuStudio single-object plate: the object only under `; OBJECT_ID:` (no
/// start/stop labels), a tower section in the middle, and object lines after the tower that
/// come before the next `; OBJECT_ID:`.
fn bambu_single_layer(z: f64, support_x: f64) -> String {
    format!(
        "; CHANGE_LAYER\n; Z_HEIGHT: {z}\n; OBJECT_ID: 77\n\
         ; FEATURE: Outer wall\n; LINE_WIDTH: 0.45\n{}\
         ; FEATURE: Prime tower\n; WIPE_TOWER_START\n{}; WIPE_TOWER_END\n\
         ; FEATURE: Support\nG1 X{support_x} Y95 F600\nG1 X{support_x} Y105 E1\n",
        square_moves(100.0, 100.0, 5.0),
        square_moves(200.0, 200.0, 8.0),
    )
}

/// Two layers, a start purge before them and a skirt around everything on the first one.
fn bambu_single_plate() -> String {
    let start = "; FEATURE: Custom\nG1 X18 Y1 F600\nG1 X240 Y1 E15\n";
    let skirt = format!("; FEATURE: Skirt\n{}", square_moves(150.0, 150.0, 70.0));
    format!(
        "{start}{}{skirt}{}",
        bambu_single_layer(0.2, 92.0),
        bambu_single_layer(2.4, 92.0)
    )
}

#[test]
fn bambu_single_object_without_start_labels_is_one_object() {
    let analysis = analyze_plate(bambu_single_plate().as_bytes());
    assert!(analysis.require_object_coverage(1).is_ok(), "{analysis:?}");
    assert_eq!(analysis.extents.len(), 1);
    let (label, extent) = &analysis.extents[0];
    assert_eq!(label, "77");
    // The support printed after the tower, before any new `; OBJECT_ID:`, is the object's;
    // the tower (x 192..208), the start purge (y = 1) and the skirt (80..220) are not.
    assert!((extent.min_x - (92.0 - 0.225)).abs() < 1e-9, "{extent:?}");
    assert!(extent.max_x < 106.0 && extent.min_y > 90.0, "{extent:?}");
    let first = &analysis.first_layer[0];
    assert!(first.strips.iter().any(|strip| strip.support));
    assert!(first.strips.iter().any(|strip| !strip.support));
}

#[test]
fn orca_comment_and_klipper_labels_name_one_object() {
    let gcode = format!(
        ";TYPE:Custom\nG1 X85 Y1\nG1 X185 Y1 E15\n;LAYER_CHANGE\n;Z:0.2\n\
         ; printing object goat.stl id:0 copy 0\nEXCLUDE_OBJECT_START NAME=goat.stl_id_0_copy_0\n\
         ;TYPE:Outer wall\n;WIDTH:0.42\n{}\
         ; stop printing object goat.stl id:0 copy 0\nEXCLUDE_OBJECT_END NAME=goat.stl_id_0_copy_0\n\
         ;TYPE:Prime tower\n; WIPE_TOWER_START\n{}; WIPE_TOWER_END\n",
        square_moves(50.0, 50.0, 5.0),
        square_moves(200.0, 200.0, 8.0),
    );
    let analysis = analyze_plate(gcode.as_bytes());
    assert!(analysis.require_object_coverage(1).is_ok());
    assert_eq!(analysis.extents[0].0, "goat.stl id:0 copy 0");
    assert_eq!(analysis.first_layer.len(), 1);
}

/// On a multi-object BambuStudio plate `; OBJECT_ID:` announces the next object a few lines
/// before the current one stops: the explicit labels decide.
#[test]
fn object_id_announcements_do_not_move_lines_between_objects() {
    let gcode = "; CHANGE_LAYER\n; Z_HEIGHT: 0.2\n; OBJECT_ID: 11\n\
                 ; start printing object, unique label id: 11\n; FEATURE: Outer wall\n\
                 G1 X0 Y0\nG1 X10 Y0 E1\n; OBJECT_ID: 22\nG1 X10 Y3 E1\n\
                 ; stop printing object, unique label id: 11\n\
                 ; start printing object, unique label id: 22\n\
                 G1 X50 Y0\nG1 X60 Y0 E1\n; stop printing object, unique label id: 22\n";
    let analysis = analyze_plate(gcode.as_bytes());
    let extent_of = |label: &str| {
        analysis
            .extents
            .iter()
            .find(|(name, _)| name == label)
            .map(|(_, extent)| *extent)
            .unwrap()
    };
    assert!(extent_of("11").max_y > 3.0);
    assert!(extent_of("22").min_x > 49.0);
    assert!(analysis.require_object_coverage(2).is_ok());
}

#[test]
fn the_toolpath_extent_takes_the_centre_line_and_skips_machine_gcode() {
    let plate = |x: f64| {
        format!(
            "; FEATURE: Custom\nG1 X-5 Y-1\nG1 X300 Y-1 E15\n; CHANGE_LAYER\n; OBJECT_ID: 1\n\
             ; FEATURE: Support\n; LINE_WIDTH: 0.5\nG1 X200 Y100\nG1 X{x} Y100 E1\n"
        )
    };
    // The bead's outer half may hang over the edge; only the centre line counts.
    let inside = check_plate(
        plate(256.0 + GUI_BED_EPSILON_MM * 0.5).as_bytes(),
        &rules(vec![]),
    );
    assert!(!inside.outside_plate, "{inside:?}");
    assert!(inside.errors().is_empty());
    let outside = check_plate(
        plate(256.0 + GUI_BED_EPSILON_MM * 2.0).as_bytes(),
        &rules(vec![]),
    );
    assert!(outside.outside_plate);
    assert!(outside.require_accepted().is_err());
}

/// The tower counts toward the toolpath extent, arc bulges included: a G3 half circle whose
/// ends sit at x = 250 bulges to x = 258.
#[test]
fn tower_arcs_count_toward_the_toolpath_extent() {
    let gcode = "; CHANGE_LAYER\n; FEATURE: Prime tower\n; WIPE_TOWER_START\n\
                 G1 X250 Y100 F600\nG3 X250 Y116 I0 J8 E1\n; WIPE_TOWER_END\n";
    let report = check_plate(gcode.as_bytes(), &rules(vec![]));
    let extent = report.toolpath_extent.unwrap();
    assert!((extent.max_x - 258.0).abs() < 1e-9, "{extent:?}");
    assert!(report.outside_plate);
}

#[test]
fn every_segment_is_tested_against_the_exact_exclusion_polygon() {
    // A triangle cutter: its bbox covers (15, 15), the polygon does not.
    let triangle: Vec<Point> = [(0.0, 0.0), (18.0, 0.0), (0.0, 28.0)]
        .into_iter()
        .map(|(x, y)| Point { x, y })
        .collect();
    let plate = |from: (f64, f64), to: (f64, f64)| {
        format!(
            "; CHANGE_LAYER\n; OBJECT_ID: 5\n; FEATURE: Support\nG1 X{} Y{}\nG1 X{} Y{} E1\n",
            from.0, from.1, to.0, to.1
        )
    };
    let clear = check_plate(
        plate((15.0, 15.0), (16.0, 25.0)).as_bytes(),
        &rules(vec![triangle.clone()]),
    );
    assert!(clear.exclusion_hits.is_empty(), "{clear:?}");
    // Both ends outside, the segment cuts through the corner at the origin.
    let crossing = check_plate(
        plate((-2.0, 5.0), (10.0, -2.0)).as_bytes(),
        &rules(vec![triangle.clone()]),
    );
    assert_eq!(crossing.exclusion_hits.len(), 1);
    assert_eq!(crossing.exclusion_hits[0].party, Party::Object("5".into()));
    assert_eq!(crossing.exclusion_hits[0].first_layer, 1);
    // Touching the edge is a hit too.
    let touching = check_plate(
        plate((18.0, 0.0), (30.0, 0.0)).as_bytes(),
        &rules(vec![triangle]),
    );
    assert_eq!(touching.exclusion_hits.len(), 1);
    assert!(touching.require_accepted().is_err());
}

/// Two objects on one layer: `a` (label 1) and `b` (label 2), each a list of `(feature,
/// from, to)` lines.
fn two_objects(
    a: &[(&str, (f64, f64), (f64, f64))],
    b: &[(&str, (f64, f64), (f64, f64))],
) -> String {
    let object = |id: u32, lines: &[(&str, (f64, f64), (f64, f64))]| {
        let mut text = format!("; start printing object, unique label id: {id}\n");
        for (feature, from, to) in lines {
            text += &format!(
                "; FEATURE: {feature}\nG1 X{} Y{}\nG1 X{} Y{} E1\n",
                from.0, from.1, to.0, to.1
            );
        }
        text + &format!("; stop printing object, unique label id: {id}\n")
    };
    format!("; CHANGE_LAYER\n{}{}", object(1, a), object(2, b))
}

fn conflicts_of(gcode: &str) -> Vec<(ConflictKind, bool)> {
    check_plate(gcode.as_bytes(), &rules(vec![]))
        .conflicts
        .iter()
        .map(|conflict| (conflict.kind, conflict.allowed))
        .collect()
}

#[test]
fn crossing_centre_lines_of_two_objects_conflict() {
    let wall = [("Outer wall", (0.0, 5.0), (10.0, 5.0))];
    let crossing = [("Inner wall", (5.0, 0.0), (5.0, 10.0))];
    let gcode = two_objects(&wall, &crossing);
    let report = check_plate(gcode.as_bytes(), &rules(vec![]));
    assert_eq!(report.conflicts.len(), 1);
    let conflict = &report.conflicts[0];
    assert_eq!(conflict.kind, ConflictKind::Crossing);
    assert!(!conflict.allowed);
    assert_eq!(
        (&conflict.first, &conflict.second),
        (&Party::Object("1".into()), &Party::Object("2".into()))
    );
    assert!((conflict.at.0 - 5.0).abs() < 1e-9 && (conflict.at.1 - 5.0).abs() < 1e-9);
    assert!(report.require_accepted().is_err());
}

#[test]
fn a_crossing_at_a_segment_end_does_not_count() {
    // The second line ends 0.005 mm past the first: inside the 0.01 mm end tolerance.
    let wall = [("Outer wall", (0.0, 5.0), (10.0, 5.0))];
    let touching = [("Outer wall", (5.0, 0.0), (5.0, 5.005))];
    assert!(conflicts_of(&two_objects(&wall, &touching)).is_empty());
    let deeper = [("Outer wall", (5.0, 0.0), (5.0, 5.02))];
    assert_eq!(
        conflicts_of(&two_objects(&wall, &deeper)),
        vec![(ConflictKind::Crossing, false)]
    );
}

#[test]
fn collinear_overlap_conflicts_and_vertical_lines_are_not_exempt() {
    let wall = [("Outer wall", (0.0, 5.0), (10.0, 5.0))];
    let overlap = [("Sparse infill", (9.98, 5.0), (20.0, 5.0))];
    assert_eq!(
        conflicts_of(&two_objects(&wall, &overlap)),
        vec![(ConflictKind::CollinearOverlap, false)]
    );
    let end_to_end = [("Sparse infill", (10.0, 5.0), (20.0, 5.0))];
    assert!(conflicts_of(&two_objects(&wall, &end_to_end)).is_empty());
    let vertical = [("Outer wall", (5.0, 0.0), (5.0, 10.0))];
    let vertical_overlap = [("Outer wall", (5.0, 9.0), (5.0, 20.0))];
    assert_eq!(
        conflicts_of(&two_objects(&vertical, &vertical_overlap)),
        vec![(ConflictKind::CollinearOverlap, false)]
    );
}

#[test]
fn supports_of_two_objects_may_cross() {
    let a = [("Support", (0.0, 5.0), (10.0, 5.0))];
    let b = [("Support interface", (5.0, 0.0), (5.0, 10.0))];
    assert!(conflicts_of(&two_objects(&a, &b)).is_empty());
    // A support over a foreign wall is a conflict.
    let wall = [("Outer wall", (5.0, 0.0), (5.0, 10.0))];
    assert_eq!(
        conflicts_of(&two_objects(&a, &wall)),
        vec![(ConflictKind::Crossing, false)]
    );
}

/// A BambuStudio tower: the body square x/y 100..110 on every layer, plus an unmarked brim
/// ring 2 mm out on the first layer. One object crosses the brim with a support line on layer
/// 1 and with `feature` on layer `layer`.
fn tower_plate(feature: &str, from: (f64, f64), to: (f64, f64), layer: usize) -> String {
    let mut gcode = String::new();
    for index in 1..=6 {
        gcode += "; CHANGE_LAYER\n; FEATURE: Prime tower\n; WIPE_TOWER_START\n";
        gcode += &square_moves(105.0, 105.0, 5.0);
        if index == 1 {
            gcode += &square_moves(105.0, 105.0, 7.0);
        }
        gcode += "; WIPE_TOWER_END\n";
        if index == layer {
            gcode += &format!(
                "; start printing object, unique label id: 3\n; FEATURE: {feature}\n\
                 G1 X{} Y{}\nG1 X{} Y{} E1\n; stop printing object, unique label id: 3\n",
                from.0, from.1, to.0, to.1
            );
        }
    }
    gcode
}

#[test]
fn a_support_over_the_tower_brim_is_reported_but_allowed() {
    // Crosses the brim ring (x = 98) but stops 1 mm short of the body (x = 100).
    let gcode = tower_plate("Support", (95.0, 105.0), (99.0, 105.0), 1);
    let report = check_plate(gcode.as_bytes(), &rules(vec![]));
    assert_eq!(report.conflicts.len(), 1, "{report:?}");
    let conflict = &report.conflicts[0];
    assert!(conflict.allowed);
    assert_eq!(conflict.second, Party::Tower);
    assert!(report.require_accepted().is_ok());
}

#[test]
fn walls_over_the_tower_brim_and_supports_into_the_tower_body_are_refused() {
    let wall = tower_plate("Outer wall", (95.0, 105.0), (99.0, 105.0), 1);
    assert_eq!(conflicts_of(&wall), vec![(ConflictKind::Crossing, false)]);
    let into_body = tower_plate("Support", (95.0, 103.0), (101.0, 103.0), 4);
    assert_eq!(
        conflicts_of(&into_body),
        vec![(ConflictKind::Crossing, false)]
    );
}

#[test]
fn the_snapmaker_band_uses_part_boxes_and_equality_passes() {
    let print = ObjectPrint {
        part_boxes: vec![
            Rect::new(-10.0, -10.0, 10.0, 10.0),
            Rect::new(-2.0, 8.0, 2.0, 12.0),
        ],
        ..ObjectPrint::default()
    };
    let band = 3.5;
    let at_band = placed_part_boxes(&print, (13.5, 100.0));
    let inside_band = placed_part_boxes(&print, (13.4, 100.0));
    let hits = part_edge_hits(&[at_band, inside_band.clone()], BED, band);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!((hits[0].object, hits[0].part), (1, 0));
    match hits[0].issue {
        PartEdgeIssue::InsideBand { distance_mm } => assert!((distance_mm - 3.4).abs() < 1e-9),
        other => panic!("{other:?}"),
    }
    assert!(part_edge_hits(&[inside_band], BED, 0.0).is_empty());
    // Only the pompom part (y up to 257) leaves the plate.
    let over_edge = placed_part_boxes(&print, (100.0, 245.0));
    let outside = part_edge_hits(&[over_edge], BED, 0.0);
    assert_eq!(outside.len(), 1, "{outside:?}");
    assert_eq!((outside[0].object, outside[0].part), (0, 1));
    assert_eq!(outside[0].issue, PartEdgeIssue::OutsidePlate);
}

/// A single object (BambuStudio `; OBJECT_ID:`) with a first-layer support foot, a support at
/// layer topped at z 2.4 (bands 0-1) and an interface layer topped at z 4.2 (bands 1-2), placed
/// at (100, 100).
fn supported_plate() -> String {
    "; CHANGE_LAYER\n; Z_HEIGHT: 0.2\n; OBJECT_ID: 9\n; FEATURE: Outer wall\n; LINE_WIDTH: 0.4\n\
     G1 X95 Y95\nG1 X105 Y95 E1\n; FEATURE: Support\nG1 X110 Y90\nG1 X110 Y110 E1\n\
     ; CHANGE_LAYER\n; Z_HEIGHT: 2.4\n; OBJECT_ID: 9\n; FEATURE: Support\nG1 X108 Y95\nG1 X108 Y105 E1\n\
     ; CHANGE_LAYER\n; Z_HEIGHT: 4.2\n; OBJECT_ID: 9\n; FEATURE: Support interface\n\
     G1 X106 Y96\nG1 X106 Y104 E1\n"
        .to_owned()
}

#[test]
fn realization_from_gcode_reads_base_bands_and_interfaces_in_the_object_frame() {
    let realization =
        realization_from_gcode(supported_plate().as_bytes(), None, (100.0, 100.0), 7).unwrap();
    assert_eq!(realization.member, 7);
    let base = Rect::of_points(realization.base.iter().flatten());
    // The foot x = 110, y 90..110, 0.4 wide, moved by -(100, 100); strips carry no end caps.
    // The polygon union snaps to a fixed grid, hence the micron tolerance.
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    assert!(near(base.min_x, 9.8) && near(base.max_x, 10.2), "{base:?}");
    assert!(
        near(base.min_y, -10.0) && near(base.max_y, 10.0),
        "{base:?}"
    );
    // The band rule (design §4.1): a layer occupying (z_bottom, z] feeds every band it meets.
    // Layer 2 here spans (0.2, 2.4], layer 3 spans (2.4, 4.2], so bands 0-1 and 1-2.
    assert_eq!(realization.support.len(), 3);
    assert_eq!(realization.above_base.len(), 3);
    assert_eq!(realization.interface.len(), 3);
    // Band 0 holds the base foot (layer 0 is in band 0 too) and layer 2's strip; the support
    // above the base leaves the foot out.
    let band_0 = Rect::of_points(realization.support[0].iter().flatten());
    assert!(near(band_0.min_x, 7.8) && near(band_0.max_x, 10.2), "{band_0:?}");
    let above = Rect::of_points(realization.above_base[0].iter().flatten());
    assert!(near(above.min_x, 7.8) && near(above.max_x, 8.2), "{above:?}");
    assert!(realization.interface[0].is_empty());
    assert!(!realization.interface[1].is_empty() && !realization.interface[2].is_empty());
    let band_2 = Rect::of_points(realization.interface[2].iter().flatten());
    assert!(
        near(band_2.min_x, 5.8) && near(band_2.max_y, 4.0),
        "{band_2:?}"
    );
}

#[test]
fn realizations_need_a_label_when_the_plate_holds_several_objects() {
    let gcode = two_objects(
        &[("Support", (0.0, 0.0), (5.0, 0.0))],
        &[("Support", (50.0, 0.0), (55.0, 0.0))],
    );
    assert!(realization_from_gcode(gcode.as_bytes(), None, (0.0, 0.0), 0).is_err());
    assert!(realization_from_gcode(gcode.as_bytes(), Some("3"), (0.0, 0.0), 0).is_err());
    let objects = [
        goat_packer::SlicedObject {
            label: Some("2".into()),
            origin: (50.0, 0.0),
        },
        goat_packer::SlicedObject {
            label: Some("1".into()),
            origin: (0.0, 0.0),
        },
    ];
    let both = realizations_from_gcode(gcode.as_bytes(), &objects, 0).unwrap();
    // Each object's first-layer support lands at the same place in its own frame.
    let bounds: Vec<Rect> = both
        .iter()
        .map(|realization| Rect::of_points(realization.base.iter().flatten()))
        .collect();
    assert!((bounds[0].min_x - bounds[1].min_x).abs() < 1e-6);
    assert!((bounds[0].max_x - 5.0).abs() < 1e-6, "{:?}", bounds[0]);
}

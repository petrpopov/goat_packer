use super::*;
use crate::geometry::Rect;

fn bambu_plate(support_x: f64) -> String {
    // Object 1: a body wall along x = 0..10 at y = 0 on layers 1 and 2. Object 2: a support
    // line parallel to it at y = support_x on layer 2 only, plus its own body far away.
    format!(
        "; CHANGE_LAYER\n\
         ; start printing object, unique label id: 11\n\
         ; FEATURE: Outer wall\n; LINE_WIDTH: 0.45\n\
         G1 X0 Y0 F600\nG1 X10 Y0 E0.5\n\
         ; stop printing object, unique label id: 11\n\
         ; start printing object, unique label id: 22\n\
         ; FEATURE: Outer wall\nG1 X0 Y20\nG1 X10 Y20 E0.5\n\
         ; stop printing object, unique label id: 22\n\
         ; CHANGE_LAYER\n\
         ; start printing object, unique label id: 11\n\
         ; FEATURE: Outer wall\nG1 X0 Y0\nG1 X10 Y0 E0.5\n\
         ; stop printing object, unique label id: 11\n\
         ; start printing object, unique label id: 22\n\
         ; FEATURE: Support\n; LINE_WIDTH: 0.4\n\
         G1 X0 Y{support_x}\nG1 X10 Y{support_x} E0.5\n\
         ; stop printing object, unique label id: 22\n"
    )
}

#[test]
fn touching_support_on_a_foreign_body_is_reported_with_layer_and_gap() {
    // Centre lines 0.3 apart, half widths 0.225 + 0.2: overlap by 0.125.
    let analysis = analyze_plate(bambu_plate(0.3).as_bytes());
    assert_eq!(analysis.layers, 2);
    assert_eq!(analysis.collisions.len(), 1);
    let collision = &analysis.collisions[0];
    assert_eq!(collision.support_object, "22");
    assert_eq!(collision.body_object, "11");
    assert_eq!(
        (
            collision.first_layer,
            collision.last_layer,
            collision.layers
        ),
        (2, 2, 1)
    );
    assert!(
        (collision.min_gap_mm + 0.125).abs() < 1e-9,
        "{}",
        collision.min_gap_mm
    );
}

#[test]
fn a_support_clear_of_foreign_bodies_is_not_reported() {
    let analysis = analyze_plate(bambu_plate(0.5).as_bytes());
    assert!(analysis.collisions.is_empty());
    assert!(analysis.labelled_lines > 0);
}

#[test]
fn own_support_touching_own_body_is_fine() {
    let gcode = "; CHANGE_LAYER\n; start printing object, unique label id: 1\n\
                 ; FEATURE: Outer wall\nG1 X0 Y0\nG1 X10 Y0 E0.5\n\
                 ; FEATURE: Support\nG1 X0 Y0.1\nG1 X10 Y0.1 E0.5\n\
                 ; stop printing object, unique label id: 1\n";
    assert!(analyze_plate(gcode.as_bytes()).collisions.is_empty());
}

#[test]
fn orca_labels_types_and_arcs_are_parsed() {
    // A G3 quarter arc of object A's support sweeping across object B's straight wall.
    let gcode = ";LAYER_CHANGE\n\
                 ; printing object b.stl id:1 copy 0\n;TYPE:Outer wall\n;WIDTH:0.42\n\
                 G1 X0 Y5\nG1 X10 Y5 E0.4\n\
                 ; stop printing object b.stl id:1 copy 0\n\
                 ; printing object a.stl id:0 copy 0\n;TYPE:Support\n;WIDTH:0.42\n\
                 G1 X0 Y0\nG3 X5 Y5 I0 J5 E0.3\n\
                 ; stop printing object a.stl id:0 copy 0\n";
    let analysis = analyze_plate(gcode.as_bytes());
    assert_eq!(analysis.collisions.len(), 1);
    assert_eq!(analysis.collisions[0].support_object, "a.stl id:0 copy 0");
    assert_eq!(analysis.collisions[0].body_object, "b.stl id:1 copy 0");
    assert_eq!(analysis.first_layer.len(), 2);
    assert!(
        analysis.first_layer[1]
            .strips
            .iter()
            .all(|strip| strip.support)
    );
}

#[test]
fn an_arc_bulging_past_the_plate_edge_is_off_plate() {
    // The P1S support base: a G3 whose end points sit at x = 255.6 but whose sweep bulges
    // to x ≈ 256.3 — BambuStudio's viewer samples the arc and refuses the plate.
    let gcode = "; CHANGE_LAYER\n\
                 ; start printing object, unique label id: 7\n; FEATURE: Support\n\
                 G1 X255.6 Y170\nG3 X255.6 Y180 I-17.5 J5 E0.9\n\
                 ; stop printing object, unique label id: 7\n\
                 ; start printing object, unique label id: 8\n; FEATURE: Outer wall\n\
                 G1 X100 Y100\nG1 X150 Y100 E1.0\n\
                 ; stop printing object, unique label id: 8\n";
    let analysis = analyze_plate(gcode.as_bytes());
    let off = analysis.off_plate(Rect::new(0.0, 0.0, 256.0, 256.0));
    assert_eq!(off.len(), 1, "{off:?}");
    assert_eq!(off[0].0, "7");
    assert!(
        off[0].1.max_x > 256.4 && off[0].1.max_x < 256.7,
        "{:?}",
        off[0].1
    );
    assert!(
        analysis
            .off_plate(Rect::new(0.0, 0.0, 257.0, 256.0))
            .is_empty()
    );
}

#[test]
fn unretractions_and_unlabelled_lines_are_ignored() {
    let gcode = "; CHANGE_LAYER\nG1 X0 Y0\nG1 X10 Y0 E0.5\n\
                 ; start printing object, unique label id: 1\n; FEATURE: Support\n\
                 G1 X0 Y0.1\nG1 E0.8\n\
                 ; stop printing object, unique label id: 1\n";
    let analysis = analyze_plate(gcode.as_bytes());
    assert!(analysis.collisions.is_empty());
    assert_eq!(analysis.labelled_lines, 0);
}

#[test]
fn first_layer_body_center_ignores_supports() {
    let analysis = analyze_plate(bambu_plate(0.3).as_bytes());
    let object = &analysis.first_layer[0];
    assert_eq!(object.label, "11");
    assert_eq!(object.body_center(), Some((5.0, 0.0)));
}

#[test]
fn bed_check_includes_extrusion_width_not_just_the_path_centre() {
    let gcode = "; CHANGE_LAYER\n; start printing object, unique label id: 7\n; FEATURE: Outer wall\n; LINE_WIDTH: 0.4\nG1 X9.9 Y2\nG1 X9.9 Y8 E1\n";
    let analysis = analyze_plate(gcode.as_bytes());
    let off = analysis.off_plate(Rect::new(0.0, 0.0, 10.0, 10.0));
    assert_eq!(off.len(), 1);
    assert!((off[0].1.max_x - 10.1).abs() < 1e-6);
}

#[test]
fn incomplete_or_unlabelled_slices_cannot_certify_the_plate() {
    let analysis = analyze_plate(bambu_plate(2.0).as_bytes());
    assert!(analysis.require_object_coverage(2).is_ok());
    assert!(analysis.require_object_coverage(3).is_err());
    assert!(
        analyze_plate(b"G1 X1 Y1 E1")
            .require_object_coverage(1)
            .is_err()
    );
}

#[test]
fn informational_orca_labels_do_not_count_as_extra_printed_objects() {
    // Orca can emit both a comment and Klipper's actual exclusion label before any ink.
    let gcode = b";LAYER_CHANGE\n; printing object Cube id:0 copy 0\nEXCLUDE_OBJECT_START NAME=Cube_id_0_copy_0\n;TYPE:Outer wall\nG1 X5 Y5\nG1 X8 Y5 E1\nEXCLUDE_OBJECT_END NAME=Cube_id_0_copy_0\n";
    let analysis = analyze_plate(gcode);
    assert_eq!(analysis.extents.len(), 1);
    assert!(analysis.require_object_coverage(1).is_ok());
}

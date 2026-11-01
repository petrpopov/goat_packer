use super::learning::{layer_zones_from_sample, learn_support_patch};
use super::nudge::{NUDGE_MARGIN_MM, nudge_apart};
use super::touching::TOUCH_EPSILON_MM;
use super::*;
use crate::footprint::Footprint;
use crate::geometry::Rect;
use crate::options::PackedPlacement;

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

fn at(x: f64, y: f64) -> PackedPlacement {
    PackedPlacement {
        x_mm: x,
        y_mm: y,
        angle_deg: 0.0,
    }
}

#[test]
fn nudge_moves_the_body_object_away_along_the_pair_line() {
    let placements = [at(0.0, 0.0), at(40.0, 0.0)];
    let moved = nudge_apart(&placements, &[(0, 1, -0.2)], |_| true).unwrap();
    assert_eq!((moved[0].x_mm, moved[0].y_mm), (0.0, 0.0));
    let expected = 40.0 + 0.2 + TOUCH_EPSILON_MM + NUDGE_MARGIN_MM;
    assert!((moved[1].x_mm - expected).abs() < 1e-9, "{:?}", moved[1]);
    assert!(moved[1].y_mm.abs() < 1e-9);
}

#[test]
fn nudge_moves_the_support_object_when_the_other_cannot_move() {
    // Object 1 sits against the bed edge at x = 40: only object 0 may move.
    let placements = [at(0.0, 0.0), at(40.0, 0.0)];
    let moved = nudge_apart(&placements, &[(0, 1, -0.1)], |layout| {
        layout[1].x_mm <= 40.0
    })
    .unwrap();
    assert_eq!(moved[1].x_mm, 40.0);
    assert!(moved[0].x_mm < -0.1, "{:?}", moved[0]);
}

#[test]
fn nudge_handles_a_pair_once_and_gives_up_without_a_valid_move() {
    let placements = [at(0.0, 0.0), at(40.0, 0.0)];
    let moved = nudge_apart(&placements, &[(0, 1, -0.1), (1, 0, -0.3)], |_| true).unwrap();
    let expected = 40.0 + 0.3 + TOUCH_EPSILON_MM + NUDGE_MARGIN_MM;
    assert!((moved[1].x_mm - expected).abs() < 1e-9, "{:?}", moved[1]);
    assert!(nudge_apart(&placements, &[(0, 1, -0.1)], |_| false).is_none());
}

#[test]
fn nudge_pushes_a_chain_when_a_lone_object_would_hit_the_next() {
    // A row 0 - 1 - 2 along x, 40 mm apart; 0 touches 1, and 1 may not come within 39.9 of
    // anything: only shifting 1 and 2 together keeps the row valid.
    let placements = [at(0.0, 0.0), at(40.0, 0.0), at(80.0, 0.0)];
    let valid = |layout: &[PackedPlacement]| {
        let far_enough = |i: usize, j: usize| {
            (layout[i].x_mm - layout[j].x_mm).hypot(layout[i].y_mm - layout[j].y_mm) >= 39.9
        };
        far_enough(0, 1)
            && far_enough(1, 2)
            && far_enough(0, 2)
            && layout[0].x_mm.abs() < 1e-9
            && layout.iter().all(|p| p.y_mm.abs() < 1e-9)
    };
    let moved = nudge_apart(&placements, &[(0, 1, -0.2)], valid).unwrap();
    let need = 0.2 + TOUCH_EPSILON_MM + NUDGE_MARGIN_MM;
    assert!((moved[1].x_mm - (40.0 + need)).abs() < 1e-9, "{moved:?}");
    assert!((moved[2].x_mm - (80.0 + need)).abs() < 1e-9, "{moved:?}");
}

#[test]
fn nudge_skips_a_pair_an_earlier_move_already_separated() {
    // 0-1 and 0-2 touch; shifting the side beyond 1 also carries 2 away from 0.
    let placements = [at(0.0, 0.0), at(40.0, 0.0), at(80.0, 0.0)];
    let moved = nudge_apart(&placements, &[(0, 1, -0.2), (0, 2, -0.1)], |layout| {
        layout[1].x_mm > 40.0 && (layout[2].x_mm - layout[1].x_mm) >= 39.99
    })
    .unwrap();
    assert!(
        (moved[2].x_mm - moved[1].x_mm - 40.0).abs() < 1e-9,
        "{moved:?}"
    );
}

#[test]
fn layer_zones_come_from_the_sample_in_the_footprint_frame() {
    // Two objects at x = 0 and x = 50 (footprint frame centred on the body). Layer 1
    // (z 0.2) bodies only; layer 2 (z 2.4, band 1) a wall and a support 4..5 mm to the right (strips carry no end caps).
    let object = |id: u32, x: f64, support: bool| {
        let mut text = format!(
            "; start printing object, unique label id: {id}\n; FEATURE: Outer wall\n\
             ; LINE_WIDTH: 0.4\nG1 X{} Y-1\nG1 X{} Y-1 E1\nG1 X{} Y1 E1\nG1 X{} Y1 E1\n",
            x - 1.0,
            x + 1.0,
            x + 1.0,
            x - 1.0
        );
        if support {
            text += &format!(
                "; FEATURE: Support interface\nG1 X{} Y0\nG1 X{} Y0 E1\n",
                x + 4.0,
                x + 5.0
            );
        }
        text + &format!("; stop printing object, unique label id: {id}\n")
    };
    let gcode = format!(
        "; CHANGE_LAYER\n; Z_HEIGHT: 0.2\n{}{}; CHANGE_LAYER\n; Z_HEIGHT: 2.4\n{}{}",
        object(7, 0.0, false),
        object(8, 50.0, false),
        object(7, 0.0, true),
        object(8, 50.0, true)
    );
    let square = |half: f64| {
        vec![vec![
            crate::Point { x: -half, y: -half },
            crate::Point { x: half, y: -half },
            crate::Point { x: half, y: half },
            crate::Point { x: -half, y: half },
            crate::Point { x: -half, y: -half },
        ]]
    };
    let footprint = Footprint {
        loops: square(1.0),
        body_loops: Vec::new(),
        support_landing_loops: Vec::new(),
        bed_loops: None,
        layer_zones: Vec::new(),
        width: 2.0,
        height: 2.0,
        min_z: 0.0,
        max_z: 3.0,
        triangle_count: 0,
    };
    let placements = [
        PackedPlacement {
            x_mm: 0.0,
            y_mm: 0.0,
            angle_deg: 0.0,
        },
        PackedPlacement {
            x_mm: 50.0,
            y_mm: 0.0,
            angle_deg: 0.0,
        },
    ];
    let (zones, analysis) =
        layer_zones_from_sample(&[gcode.as_bytes()], &footprint, &placements).unwrap();
    assert_eq!(analysis.layers, 2);
    // Band 0 has no support; band 1 has both.
    assert_eq!(zones.len(), 1);
    let support = crate::plate_centre::loops_bounds(&zones[0].support_loops).unwrap();
    assert!(
        (support.min_x - 4.0).abs() < 1e-6 && (support.max_x - 5.0).abs() < 1e-6,
        "{support:?}"
    );
    let body = crate::plate_centre::loops_bounds(&zones[0].body_loops).unwrap();
    // Three sides of the square: bottom, right, top (no closing left side).
    assert!(
        (body.min_x + 1.0).abs() < 1e-6 && (body.max_x - 1.2).abs() < 1e-6,
        "{body:?}"
    );
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
        off[0].1.max_x > 256.2 && off[0].1.max_x < 256.4,
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

fn square(half: f64) -> Vec<crate::Point> {
    let mut ring: Vec<crate::Point> = [(-half, -half), (half, -half), (half, half), (-half, half)]
        .iter()
        .map(|&(x, y)| crate::Point { x, y })
        .collect();
    ring.push(ring[0]);
    ring
}

#[test]
fn learned_patch_reaches_the_printed_support_of_the_matched_object() {
    let footprint = Footprint {
        loops: vec![square(2.0)],
        body_loops: vec![square(1.5)],
        support_landing_loops: vec![square(1.0)],
        layer_zones: Vec::new(),
        bed_loops: Some(vec![square(2.0)]),
        width: 4.0,
        height: 4.0,
        min_z: 0.0,
        max_z: 1.0,
        triangle_count: 2,
    };
    let placements = [
        PackedPlacement {
            x_mm: 10.0,
            y_mm: 10.0,
            angle_deg: 0.0,
        },
        PackedPlacement {
            x_mm: 40.0,
            y_mm: 10.0,
            angle_deg: 0.0,
        },
    ];
    let object = ObjectFirstLayer {
        label: "7".into(),
        strips: vec![
            Strip {
                from: (9.0, 10.0),
                to: (11.0, 10.0),
                width: 0.4,
                support: false,
            },
            Strip {
                from: (10.0, 13.0),
                to: (12.0, 13.0),
                width: 0.4,
                support: true,
            },
        ],
    };
    let learned = learn_support_patch(&footprint, &placements, &[object]).unwrap();
    let bounds = crate::plate_centre::loops_bounds(&learned.loops).unwrap();
    assert!((bounds.max_y - 3.2).abs() < 1e-6, "{bounds:?}");
    assert!((bounds.min_y + 2.0).abs() < 1e-6, "{bounds:?}");
    assert_eq!(
        learned.support_landing_loops,
        footprint.support_landing_loops
    );
    assert_eq!(learned.body_loops, footprint.body_loops);
    assert_eq!(learned.bed_loops.as_ref(), Some(&learned.loops));
    // Nothing within reach of a placement: the G-code is not this layout.
    let stray = ObjectFirstLayer {
        label: "8".into(),
        strips: vec![Strip {
            from: (99.0, 99.0),
            to: (100.0, 99.0),
            width: 0.4,
            support: false,
        }],
    };
    assert!(learn_support_patch(&footprint, &placements, &[stray]).is_err());
}

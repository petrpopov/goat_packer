//! Paint codes of work package S1 (spec 01 §2): bit stream, split geometry, extended states
//! per flavour, errors, and the projection of painted down-facing facets onto layers.

use goat_packer::embedded_mesh::{ObjectMesh, Part};
use goat_packer::paint::{
    Facing, PaintError, PaintState, PaintedLeaf, StateCoding, decode_leaves, facing, project_paint,
};
use goat_packer::polygon_area;

const TRIANGLE: [[f64; 3]; 3] = [[0.0, 0.0, 0.0], [8.0, 0.0, 0.0], [0.0, 4.0, 2.0]];

fn mid(p: [f64; 3], q: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|axis| (p[axis] + q[axis]) / 2.0)
}

fn area(corners: &[[f64; 3]; 3]) -> f64 {
    let [a, b, c] = corners;
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
}

/// A hand-built paint tree for the encoder below.
enum Node {
    Leaf(u32),
    Split {
        sides: u8,
        special: u8,
        children: Vec<Node>,
    },
}

/// Codes in reading order: a split, then its children last first; a leaf state of 3 or more
/// continues per flavour.
fn codes(node: &Node, coding: StateCoding, out: &mut Vec<u8>) {
    match node {
        Node::Leaf(state) if *state < 3 => out.push((*state as u8) << 2),
        Node::Leaf(state) => {
            out.push(0b1100);
            let mut rest = state - 3;
            if coding == StateCoding::Bambu {
                while rest >= 15 {
                    out.push(15);
                    rest -= 15;
                }
            }
            out.push(rest as u8);
        }
        Node::Split {
            sides,
            special,
            children,
        } => {
            out.push(sides | special << 2);
            for child in children.iter().rev() {
                codes(child, coding, out);
            }
        }
    }
}

/// The stored string: the first code is the last character.
fn encode(node: &Node, coding: StateCoding) -> String {
    let mut stream = Vec::new();
    codes(node, coding, &mut stream);
    stream
        .iter()
        .rev()
        .map(|code| format!("{code:X}"))
        .collect()
}

fn leaf_states(leaves: &[PaintedLeaf]) -> Vec<PaintState> {
    leaves.iter().map(|leaf| leaf.state).collect()
}

#[test]
fn unsplit_codes_give_one_leaf() {
    for (code, state) in [
        ("0", PaintState::None),
        ("4", PaintState::Enforcer),
        ("8", PaintState::Blocker),
    ] {
        let leaves = decode_leaves(code, TRIANGLE, StateCoding::Orca).unwrap();
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].state, state);
        assert_eq!(leaves[0].corners, TRIANGLE);
        assert_eq!(leaves[0].depth, 0);
    }
}

#[test]
fn spec_example_4002_paints_the_corner_child_at_the_special_vertex() {
    let leaves = decode_leaves("4002", TRIANGLE, StateCoding::Bambu).unwrap();
    // Stream order: last child (none), middle (none), first (enforcer).
    assert_eq!(
        leaf_states(&leaves),
        vec![PaintState::None, PaintState::None, PaintState::Enforcer]
    );
    let [a, b, c] = TRIANGLE;
    assert_eq!(leaves[2].corners, [a, mid(a, b), mid(a, c)]);
    assert_eq!(leaves[1].corners, [mid(a, b), b, mid(a, c)]);
    assert_eq!(leaves[0].corners, [b, c, mid(a, c)]);
}

#[test]
fn split_rules_turn_the_triangle_to_the_special_vertex() {
    let [v0, v1, v2] = TRIANGLE;
    // One side split, special vertex 1: a = v1, b = v2, c = v0, m = mid(b, c).
    let one = Node::Split {
        sides: 1,
        special: 1,
        children: vec![Node::Leaf(1), Node::Leaf(2)],
    };
    let leaves = decode_leaves(
        &encode(&one, StateCoding::Orca),
        TRIANGLE,
        StateCoding::Orca,
    )
    .unwrap();
    let m = mid(v2, v0);
    assert_eq!(leaves[0].corners, [m, v0, v1]);
    assert_eq!(leaves[0].state, PaintState::Blocker);
    assert_eq!(leaves[1].corners, [v1, v2, m]);
    assert_eq!(leaves[1].state, PaintState::Enforcer);
    // Three sides split: four children, the last one in the middle.
    let three = Node::Split {
        sides: 3,
        special: 0,
        children: vec![Node::Leaf(0), Node::Leaf(0), Node::Leaf(0), Node::Leaf(1)],
    };
    let leaves = decode_leaves(
        &encode(&three, StateCoding::Orca),
        TRIANGLE,
        StateCoding::Orca,
    )
    .unwrap();
    assert_eq!(leaves[0].corners, [mid(v0, v1), mid(v1, v2), mid(v2, v0)]);
    assert_eq!(leaves[0].state, PaintState::Enforcer);
    assert!(leaves.iter().all(|leaf| leaf.depth == 1));
}

#[test]
fn nested_splits_cover_the_parent_exactly() {
    let tree = Node::Split {
        sides: 3,
        special: 0,
        children: vec![
            Node::Split {
                sides: 2,
                special: 2,
                children: vec![
                    Node::Leaf(1),
                    Node::Split {
                        sides: 1,
                        special: 0,
                        children: vec![Node::Leaf(1), Node::Leaf(0)],
                    },
                    Node::Leaf(2),
                ],
            },
            Node::Leaf(0),
            Node::Leaf(1),
            Node::Split {
                sides: 2,
                special: 1,
                children: vec![Node::Leaf(2), Node::Leaf(1), Node::Leaf(0)],
            },
        ],
    };
    let code = encode(&tree, StateCoding::Bambu);
    let leaves = decode_leaves(&code, TRIANGLE, StateCoding::Bambu).unwrap();
    assert_eq!(leaves.len(), 9);
    assert_eq!(leaves.iter().map(|leaf| leaf.depth).max(), Some(3));
    let total: f64 = leaves.iter().map(|leaf| area(&leaf.corners)).sum();
    assert!((total - area(&TRIANGLE)).abs() < 1e-12);
    let enforced: f64 = leaves
        .iter()
        .filter(|leaf| leaf.state == PaintState::Enforcer)
        .map(|leaf| area(&leaf.corners))
        .sum();
    // Child 0: its corner quarter plus half of its middle quarter (3/8 of 1/4); child 2: 1/4;
    // child 3: its middle quarter (1/4 of 1/4). Total 13/32.
    assert!((enforced / area(&TRIANGLE) - 13.0 / 32.0).abs() < 1e-12);
}

#[test]
fn extended_states_differ_between_flavours_only_from_18() {
    // 'C' = state 3 continues; then '1' gives 4 in both flavours.
    for coding in [StateCoding::Bambu, StateCoding::Orca] {
        assert_eq!(
            decode_leaves("1C", TRIANGLE, coding).unwrap()[0].state,
            PaintState::Colour(4)
        );
    }
    // "1FC": Bambu reads 15 + 1 more (state 19); Orca stops after the F (state 18).
    assert_eq!(
        decode_leaves("1FC", TRIANGLE, StateCoding::Bambu).unwrap()[0].state,
        PaintState::Colour(19)
    );
    assert_eq!(
        decode_leaves("1FC", TRIANGLE, StateCoding::Orca).unwrap()[0].state,
        PaintState::Colour(18)
    );
    assert_eq!(PaintState::from_raw(19).raw(), 19);
    assert_eq!(PaintState::Blocker.raw(), 2);
    let deep = Node::Leaf(40);
    let leaves = decode_leaves(
        &encode(&deep, StateCoding::Bambu),
        TRIANGLE,
        StateCoding::Bambu,
    )
    .unwrap();
    assert_eq!(leaves[0].state, PaintState::Colour(40));
}

#[test]
fn corrupt_codes_are_errors() {
    assert_eq!(
        decode_leaves("4G", TRIANGLE, StateCoding::Orca),
        Err(PaintError::NotHex('G'))
    );
    assert_eq!(
        decode_leaves("2", TRIANGLE, StateCoding::Orca),
        Err(PaintError::Truncated)
    );
    assert_eq!(
        decode_leaves("C", TRIANGLE, StateCoding::Orca),
        Err(PaintError::Truncated)
    );
    assert_eq!(
        decode_leaves("7", TRIANGLE, StateCoding::Orca),
        Err(PaintError::BadSplit)
    );
    assert_eq!(
        decode_leaves("D", TRIANGLE, StateCoding::Orca),
        Err(PaintError::BadSplit)
    );
    let mut too_deep = Node::Leaf(1);
    for _ in 0..30 {
        too_deep = Node::Split {
            sides: 1,
            special: 0,
            children: vec![too_deep, Node::Leaf(0)],
        };
    }
    let code = encode(&too_deep, StateCoding::Orca);
    assert_eq!(
        decode_leaves(&code, TRIANGLE, StateCoding::Orca),
        Err(PaintError::TooDeep)
    );
}

// ---------------------------------------------------------------------------------------
// Projection.

/// The P1S grid of spec 01 §1.2.2: first layer 0.2, then 0.28.
fn slice_heights(count: usize) -> Vec<f64> {
    (0..count)
        .map(|k| {
            if k == 0 {
                0.1
            } else {
                0.34 + 0.28 * (k - 1) as f64
            }
        })
        .collect()
}

/// One part holding the given triangles (open surface is enough for paint) plus a vertex at
/// the origin so the frame keeps plate heights and XY.
fn painted_part(triangles: &[([[f64; 3]; 3], &str)]) -> ObjectMesh {
    let mut part = Part {
        vertices: vec![[-20.0, -20.0, 0.0], [20.0, 20.0, 0.0]],
        ..Part::default()
    };
    for (corners, code) in triangles {
        let base = part.vertices.len() as u32;
        part.vertices.extend(corners);
        let index = part.triangles.len() as u32;
        part.triangles.push([base, base + 1, base + 2]);
        if !code.is_empty() {
            part.paint_supports.insert(index, (*code).to_owned());
        }
    }
    ObjectMesh::from_plate_parts(vec![part]).unwrap()
}

/// Down-facing ramp from z = 2 (along x = 0) to z = 3 at (10, 0).
const RAMP: [[f64; 3]; 3] = [[0.0, 0.0, 2.0], [0.0, 10.0, 2.0], [10.0, 0.0, 3.0]];

#[test]
fn painted_ramp_splits_over_the_layers_it_spans() {
    let mesh = painted_part(&[(RAMP, "4")]);
    let slice_z = slice_heights(40);
    let projection =
        project_paint(&mesh, PaintState::Enforcer, &slice_z, StateCoding::Bambu).unwrap();
    let filled: Vec<usize> = (0..slice_z.len())
        .filter(|&k| !projection.layers[k].is_empty())
        .collect();
    // z_6 = 1.74 < 2 <= z_7 = 2.02, ..., z_10 = 2.86 < 3 <= z_11 = 3.14.
    assert_eq!(filled, vec![7, 8, 9, 10, 11]);
    let total: f64 = projection
        .layers
        .iter()
        .flatten()
        .map(|ring| polygon_area(ring))
        .sum();
    assert!((total - 50.0).abs() < 1e-6, "pieces tile the ramp: {total}");
    assert!(projection.vertical_points.is_empty());
    assert!(
        project_paint(&mesh, PaintState::Blocker, &slice_z, StateCoding::Bambu)
            .unwrap()
            .layers
            .iter()
            .all(Vec::is_empty)
    );
}

#[test]
fn only_down_facing_leaves_project_and_vertical_enforcers_become_points() {
    let up = [RAMP[0], RAMP[2], RAMP[1]];
    let wall = [[0.0, 0.0, 1.0], [10.0, 0.0, 1.0], [5.0, 0.0, 4.0]];
    let mesh = painted_part(&[(up, "4"), (wall, "4"), (RAMP, "8")]);
    let slice_z = slice_heights(40);
    let enforcers =
        project_paint(&mesh, PaintState::Enforcer, &slice_z, StateCoding::Bambu).unwrap();
    assert!(enforcers.layers.iter().all(Vec::is_empty));
    assert_eq!(enforcers.vertical_points, vec![[5.0, 0.0, 2.0]]);
    let blockers = project_paint(&mesh, PaintState::Blocker, &slice_z, StateCoding::Bambu).unwrap();
    assert!(
        blockers.vertical_points.is_empty(),
        "vertical points are for enforcers only"
    );
    assert_eq!(
        blockers
            .layers
            .iter()
            .filter(|layer| !layer.is_empty())
            .count(),
        5
    );
}

#[test]
fn mirrored_parts_flip_the_facing() {
    let mesh = painted_part(&[(RAMP, "4")]);
    let leaf = decode_leaves("4", mesh.parts[0].corners(0), StateCoding::Orca).unwrap()[0];
    assert_eq!(facing(&leaf, false), Facing::Down);
    assert_eq!(facing(&leaf, true), Facing::Up);
    let mut mirrored = mesh.clone();
    mirrored.parts[0].mirrored = true;
    let slice_z = slice_heights(40);
    let projection =
        project_paint(&mirrored, PaintState::Enforcer, &slice_z, StateCoding::Orca).unwrap();
    assert!(projection.layers.iter().all(Vec::is_empty));
}

#[test]
fn horizontal_leaves_on_a_plane_go_one_layer_up_and_above_the_top_drop() {
    let flat = |z: f64| [[0.0, 0.0, z], [0.0, 4.0, z], [4.0, 0.0, z]];
    let mesh = painted_part(&[(flat(3.0), "4"), (flat(3.5), "4"), (flat(6.0), "4")]);
    let slice_z = [1.0, 2.0, 3.0, 4.0, 5.0];
    let projection =
        project_paint(&mesh, PaintState::Enforcer, &slice_z, StateCoding::Orca).unwrap();
    let areas: Vec<f64> = projection
        .layers
        .iter()
        .map(|layer| layer.iter().map(|ring| polygon_area(ring)).sum())
        .collect();
    // The leaf on plane 3.0 (layer 2) goes to layer 3 together with the one at 3.5.
    assert_eq!(
        areas
            .iter()
            .map(|a| (a * 1e6).round() / 1e6)
            .collect::<Vec<_>>(),
        vec![0.0, 0.0, 0.0, 8.0, 0.0]
    );
}

#[test]
fn partly_painted_triangles_project_only_their_painted_leaves() {
    // Two sides split at vertex 0: only the corner child (a quarter) is painted.
    let mesh = painted_part(&[(RAMP, "4002")]);
    let slice_z = slice_heights(40);
    let projection =
        project_paint(&mesh, PaintState::Enforcer, &slice_z, StateCoding::Bambu).unwrap();
    let total: f64 = projection
        .layers
        .iter()
        .flatten()
        .map(|ring| polygon_area(ring))
        .sum();
    assert!(
        (total - 12.5).abs() < 1e-6,
        "a quarter of the 50 mm2 ramp: {total}"
    );
    let again = project_paint(&mesh, PaintState::Enforcer, &slice_z, StateCoding::Bambu).unwrap();
    assert_eq!(again, projection, "deterministic");
}

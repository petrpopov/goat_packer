//! Real-data check of the S1 paint decoder against spec 01 §2.4/§2.6:
//! `cargo run --release --example paint_check -- <3mf> <first_layer_mm> <layer_mm>`
use goat_packer::embedded_mesh::load_object_mesh;
use goat_packer::paint::{Facing, PaintState, StateCoding, decode_leaves, facing, project_paint};
use goat_packer::polygon_area;
use std::path::Path;

fn area(c: &[[f64; 3]; 3]) -> f64 {
    let u = [c[1][0] - c[0][0], c[1][1] - c[0][1], c[1][2] - c[0][2]];
    let v = [c[2][0] - c[0][0], c[2][1] - c[0][1], c[2][2] - c[0][2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mesh = load_object_mesh(Path::new(&args[1])).unwrap();
    let first: f64 = args[2].parse().unwrap();
    let layer: f64 = args[3].parse().unwrap();
    let height = mesh.height();
    // Layer grid of spec 01 §1.2 for a fixed profile: emit while mid-height < H.
    let mut slice_z = vec![first * 0.5];
    let mut top = first;
    while top + layer * 0.5 < height {
        slice_z.push(top + layer * 0.5);
        top += layer;
    }
    let (mut painted, mut whole, mut split, mut max_depth) = (0, 0, 0, 0u8);
    let (mut enforcer_leaves, mut none_leaves, mut blocker_leaves) = (0, 0, 0);
    let (mut enforcer_area, mut unpainted_in_split, mut worst_ratio) = (0.0, 0.0, 0.0f64);
    let (mut down, mut other, mut zlo, mut zhi) = (0, 0, f64::INFINITY, f64::NEG_INFINITY);
    for part in mesh.normal_parts() {
        for (&index, code) in &part.paint_supports {
            painted += 1;
            if code == "4" {
                whole += 1;
            }
            let corners = part.corners(index as usize);
            let leaves = decode_leaves(code, corners, StateCoding::Bambu).unwrap();
            if leaves.len() > 1 {
                split += 1;
            }
            let leaf_area: f64 = leaves.iter().map(|l| area(&l.corners)).sum();
            let parent = area(&corners);
            if parent > 0.0 {
                worst_ratio = worst_ratio.max((leaf_area / parent - 1.0).abs());
            }
            for leaf in &leaves {
                max_depth = max_depth.max(leaf.depth);
                match leaf.state {
                    PaintState::Enforcer => {
                        enforcer_leaves += 1;
                        enforcer_area += area(&leaf.corners);
                        if facing(leaf, part.mirrored) == Facing::Down {
                            down += 1;
                            for c in leaf.corners {
                                zlo = f64::min(zlo, c[2]);
                                zhi = f64::max(zhi, c[2]);
                            }
                        } else {
                            other += 1;
                        }
                    }
                    PaintState::None => {
                        none_leaves += 1;
                        if leaves.len() > 1 {
                            unpainted_in_split += area(&leaf.corners);
                        }
                    }
                    PaintState::Blocker => blocker_leaves += 1,
                    PaintState::Colour(_) => {}
                }
            }
        }
    }
    println!(
        "layers {} first slice {:.3} last slice {:.3}",
        slice_z.len(),
        slice_z[0],
        slice_z[slice_z.len() - 1]
    );
    println!("painted {painted} (\"4\": {whole}, split {split}) max depth {max_depth}");
    println!("leaves enforcer {enforcer_leaves} none {none_leaves} blocker {blocker_leaves}");
    println!(
        "enforcer area {enforcer_area:.2} unpainted in split {unpainted_in_split:.2} worst child/parent area deviation {worst_ratio:.2e}"
    );
    println!("down-facing enforcer leaves {down} (others {other}), z {zlo:.2}..{zhi:.2}");
    let projection =
        project_paint(&mesh, PaintState::Enforcer, &slice_z, StateCoding::Bambu).unwrap();
    let nonempty: Vec<usize> = projection
        .layers
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.is_empty())
        .map(|(k, _)| k)
        .collect();
    println!("E_k non-empty on {} layers: {:?}", nonempty.len(), nonempty);
    let total: f64 = projection
        .layers
        .iter()
        .flatten()
        .map(|l| polygon_area(l))
        .sum();
    println!(
        "projected enforcer area summed over layers {total:.2}; vertical points {}",
        projection.vertical_points.len()
    );
    for coding in [StateCoding::Bambu, StateCoding::Orca] {
        let mut states = std::collections::BTreeMap::new();
        let mut errors = 0;
        for part in mesh.normal_parts() {
            for (&index, code) in &part.paint_color {
                match decode_leaves(code, part.corners(index as usize), coding) {
                    Ok(leaves) => leaves.iter().for_each(|leaf| {
                        *states.entry(format!("{:?}", leaf.state)).or_insert(0usize) += 1
                    }),
                    Err(_) => errors += 1,
                }
            }
        }
        println!("paint_color {coding:?}: leaves by state {states:?}, undecodable {errors}");
    }
}

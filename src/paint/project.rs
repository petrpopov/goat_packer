//! Painted facets onto the layer grid (spec 01 §2.6): the down-facing leaves of one state,
//! each piece assigned to the first layer whose slice plane is at or above it, projected
//! and united per layer; vertical enforcer leaves become points.

use i_overlay::core::fill_rule::FillRule;

use super::decode::decode_leaves;
use super::{PaintError, PaintState, PaintedLeaf, StateCoding};
use crate::embedded_mesh::ObjectMesh;
use crate::mesh_section::area::{
    IntRings, counter_clockwise, int_point, loops_from_rings, resolve,
};
use crate::mesh_section::slab_polygon;
use crate::object_print::Loops;

/// What the painted facets of one state put on each layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaintLayers {
    /// Index = layer (aligned with the `slice_z` given): the projected area, even-odd loops.
    pub layers: Vec<Loops>,
    /// Centroids of vertical leaves (enforcers only), object frame; the overhang stage turns
    /// each into a 0.5 mm disc on the first layer at or above it (spec 01 §2.6 step 6).
    pub vertical_points: Vec<[f64; 3]>,
}

/// How a leaf faces, from the sign of its XY area on the truncated nm grid (spec 01 §2.6
/// step 2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Facing {
    Up,
    Down,
    Vertical,
    Degenerate,
}

/// Projects the leaves in state `wanted` of every normal part's `paint_supports` onto the
/// layers whose slice heights are `slice_z` (ascending, object frame). Only normal parts
/// carry support paint the slicer reads (spec 01 §2.1).
pub fn project_paint(
    mesh: &ObjectMesh,
    wanted: PaintState,
    slice_z: &[f64],
    coding: StateCoding,
) -> Result<PaintLayers, PaintError> {
    let mut rings = vec![IntRings::new(); slice_z.len()];
    let mut vertical_points = Vec::new();
    for part in mesh.normal_parts() {
        for (&index, code) in &part.paint_supports {
            let leaves = decode_leaves(code, part.corners(index as usize), coding)?;
            for leaf in leaves.iter().filter(|leaf| leaf.state == wanted) {
                match facing(leaf, part.mirrored) {
                    Facing::Down | Facing::Degenerate => add_to_layers(&mut rings, leaf, slice_z),
                    Facing::Vertical if wanted == PaintState::Enforcer => {
                        vertical_points.push(centroid(leaf));
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(PaintLayers {
        layers: rings
            .iter()
            .map(|layer| loops_from_rings(&resolve(layer, FillRule::Positive)))
            .collect(),
        vertical_points,
    })
}

/// The facing of a leaf. The slicer truncates the XY coordinates to whole nm before the
/// cross product, so near-vertical leaves are classified on that grid.
pub fn facing(leaf: &PaintedLeaf, mirrored: bool) -> Facing {
    let [a, b, c] = leaf
        .corners
        .map(|corner| [(corner[0] * 1.0e6) as i64, (corner[1] * 1.0e6) as i64]);
    let cross = i128::from(b[0] - a[0]) * i128::from(c[1] - b[1])
        - i128::from(b[1] - a[1]) * i128::from(c[0] - b[0]);
    let cross = if mirrored { -cross } else { cross };
    match cross.signum() {
        1 => Facing::Up,
        -1 => Facing::Down,
        _ => {
            let [p, q, r] = leaf.corners;
            if p == q || p == r || q == r {
                Facing::Degenerate
            } else {
                Facing::Vertical
            }
        }
    }
}

/// Splits a leaf over the layers: the piece with `z_(k-1) < z <= z_k` goes to layer k (no
/// lower bound for layer 0); a horizontal leaf exactly on plane j goes to layer j + 1; what
/// lies above the last plane is dropped (spec 01 §2.6 step 4).
fn add_to_layers(rings: &mut [IntRings], leaf: &PaintedLeaf, slice_z: &[f64]) {
    let zs = leaf.corners.map(|corner| corner[2]);
    let low = zs[0].min(zs[1]).min(zs[2]);
    let high = zs[0].max(zs[1]).max(zs[2]);
    let first = slice_z.partition_point(|&z| z < low);
    if first == slice_z.len() {
        return;
    }
    if low == high {
        let layer = if slice_z[first] == low {
            first + 1
        } else {
            first
        };
        if layer < slice_z.len() {
            push_piece(&mut rings[layer], leaf, f64::NEG_INFINITY, f64::INFINITY);
        }
        return;
    }
    let last = slice_z
        .partition_point(|&z| z < high)
        .min(slice_z.len() - 1);
    for layer in first..=last {
        let below = if layer == 0 {
            f64::NEG_INFINITY
        } else {
            slice_z[layer - 1]
        };
        push_piece(&mut rings[layer], leaf, below, slice_z[layer]);
    }
}

fn push_piece(rings: &mut IntRings, leaf: &PaintedLeaf, lo: f64, hi: f64) {
    let polygon = slab_polygon(&leaf.corners, lo, hi);
    let ring = polygon
        .iter()
        .map(|point| int_point(point[0], point[1]))
        .collect();
    rings.extend(counter_clockwise(ring));
}

fn centroid(leaf: &PaintedLeaf) -> [f64; 3] {
    let [a, b, c] = leaf.corners;
    [0, 1, 2].map(|axis| (a[axis] + b[axis] + c[axis]) / 3.0)
}

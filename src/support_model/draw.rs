//! Stage B6: the branches as per-layer polygons (spec 03 §3). Layer l is sliced at the
//! middle of its own height, so a tube between the nodes of layers l − 1 and l shows as an
//! ellipse centred between them (minor semi-axis the mean radius, major that over the cosine
//! of the tilt); a node where branches meet is a ball; a root on the bed is a hemisphere
//! (Bambu, Snapmaker) or a flat cylinder (Orca). Every slice loses the object's own radius-0
//! collision area, and the layer's union is closed over notches narrower than a line.

mod shapes;

use shapes::{CHORD_TOLERANCE_MM, disc, slice_cylinder};

use super::flavour::RootCap;
use super::node::Node;
use super::stage_a::StageA;
use crate::polygon_ops::{
    FillRule, IntPoint, Join, Ring, Rings, difference, offset, resolve, simplify,
};

/// Notches narrower than a support line are filled (spec 03 §3.7 step 2).
const NOTCH_CLOSING_MM: f64 = 0.42;

/// A bend whose sine is above this (about 10°) gets a ball at its joint.
const SHARP_BEND_SINE: f64 = 0.17;

/// Simplification tolerance of the layer outline, mm (spec 03 §3.7 step 3).
const SIMPLIFY_MM: f64 = 0.03;

/// The tube slices of every layer, trimmed and closed, in the object frame.
pub fn draw(stage: &StageA, layers: &[Vec<Node>], centres: &[Vec<IntPoint>]) -> Vec<Rings> {
    let pieces = Pieces::new(stage, layers, centres);
    let mut slices: Vec<Vec<Ring>> = vec![Vec::new(); layers.len()];
    for (layer, nodes) in layers.iter().enumerate() {
        for (index, node) in nodes.iter().enumerate() {
            pieces.add_node(&mut slices, layer, index, node);
        }
    }
    slices
        .into_iter()
        .enumerate()
        .map(|(layer, rings)| finish_layer(stage, layer, rings))
        .collect()
}

/// Everything the slices of one drawing need.
struct Pieces<'a> {
    stage: &'a StageA,
    layers: &'a [Vec<Node>],
    centres: &'a [Vec<IntPoint>],
    /// `child[l][i]`: the node of layer l − 1 below node i of layer l.
    child: Vec<Vec<Option<usize>>>,
    /// Slice height of every layer and the layer's print height, units.
    slice_z: Vec<f64>,
    top_z: Vec<f64>,
    tolerance: f64,
}

impl<'a> Pieces<'a> {
    fn new(stage: &'a StageA, layers: &'a [Vec<Node>], centres: &'a [Vec<IntPoint>]) -> Pieces<'a> {
        let grid = stage.derived.grid;
        let mut child: Vec<Vec<Option<usize>>> =
            layers.iter().map(|nodes| vec![None; nodes.len()]).collect();
        for layer in 0..layers.len().saturating_sub(1) {
            for (index, node) in layers[layer].iter().enumerate() {
                for &parent in &node.parents {
                    child[layer + 1][parent] = Some(index);
                }
            }
        }
        let top_z: Vec<f64> = stage
            .grid
            .layers
            .iter()
            .map(|layer| grid.scaled(layer.top))
            .collect();
        let slice_z = (0..top_z.len())
            .map(|layer| {
                let below = if layer == 0 { 0.0 } else { top_z[layer - 1] };
                0.5 * (below + top_z[layer])
            })
            .collect();
        Pieces {
            stage,
            layers,
            centres,
            child,
            slice_z,
            top_z,
            tolerance: grid.scaled(CHORD_TOLERANCE_MM),
        }
    }

    fn position(&self, layer: usize, index: usize) -> [f64; 2] {
        let point = self.centres[layer][index];
        [f64::from(point.x), f64::from(point.y)]
    }

    fn add_node(&self, slices: &mut [Vec<Ring>], layer: usize, index: usize, node: &Node) {
        let derived = &self.stage.derived;
        let radius = node.real_radius(derived) as f64;
        let here = self.position(layer, index);
        if layer == 0 {
            slices[0].push(self.root_disc(here, radius));
        }
        if let Some(below) = self.child[layer][index] {
            self.add_tube(slices, layer, here, radius, below);
        }
        // A ball where branches meet or a branch bends sharply closes the wedge gaps between
        // straight pieces (a gentle bend leaves none worth a polygon); a tip has no upper half
        // to draw, and its piece already ends at it.
        let bends =
            node.parents.len() == 1 && self.bend(layer, index, node.parents[0]) > SHARP_BEND_SINE;
        if node.parents.len() >= 2 || bends {
            self.add_ball(slices, layer, here, radius);
        }
    }

    /// Sine of the angle between the piece below the node and the piece above it (0 when the
    /// node has no piece on one side).
    fn bend(&self, layer: usize, index: usize, parent: usize) -> f64 {
        let Some(below) = self.child[layer][index] else {
            return 0.0;
        };
        let (a, b, c) = (
            self.position(layer - 1, below),
            self.position(layer, index),
            self.position(layer + 1, parent),
        );
        let (z0, z1, z2) = (
            self.top_z[layer - 1],
            self.top_z[layer],
            self.top_z[layer + 1],
        );
        let first = [b[0] - a[0], b[1] - a[1], z1 - z0];
        let second = [c[0] - b[0], c[1] - b[1], z2 - z1];
        let cross = [
            first[1] * second[2] - first[2] * second[1],
            first[2] * second[0] - first[0] * second[2],
            first[0] * second[1] - first[1] * second[0],
        ];
        let norm = |v: [f64; 3]| libm::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
        norm(cross) / (norm(first) * norm(second)).max(f64::MIN_POSITIVE)
    }

    /// The root's slice at the middle of layer 0 (spec 03 §3.3).
    fn root_disc(&self, at: [f64; 2], radius: f64) -> Ring {
        let reach = match self.stage.flavour.root_cap() {
            RootCap::FlatFoot => radius,
            RootCap::Hemisphere => {
                let drop = self.top_z[0] - self.slice_z[0];
                libm::sqrt((radius * radius - drop * drop).max(0.0))
            }
        };
        disc(at, reach, self.tolerance)
    }

    /// The straight piece between node `below` of layer l − 1 and the node at `here`, sliced
    /// at every layer whose plane it can reach (a tilted piece reaches r·sin φ above and below
    /// its ends).
    fn add_tube(
        &self,
        slices: &mut [Vec<Ring>],
        layer: usize,
        here: [f64; 2],
        radius: f64,
        below: usize,
    ) {
        let derived = &self.stage.derived;
        let other = &self.layers[layer - 1][below];
        let other_at = self.position(layer - 1, below);
        let mean = 0.5 * (radius + other.real_radius(derived) as f64);
        let (z_from, z_to) = (self.top_z[layer - 1], self.top_z[layer]);
        let sideways = libm::hypot(here[0] - other_at[0], here[1] - other_at[1]);
        let tilt = sideways / libm::hypot(z_to - z_from, sideways);
        let (low, high) = (z_from - mean * tilt, z_to + mean * tilt);
        let first = self.slice_z.partition_point(|&z| z < low);
        let last = self.slice_z.partition_point(|&z| z <= high);
        for index in first..last.min(slices.len()) {
            let piece = slice_cylinder(
                other_at,
                z_from,
                here,
                z_to,
                mean,
                self.slice_z[index],
                self.tolerance,
            );
            slices[index].extend(piece);
        }
    }

    fn add_ball(&self, slices: &mut [Vec<Ring>], layer: usize, at: [f64; 2], radius: f64) {
        let centre_z = self.top_z[layer];
        for (index, slice) in slices.iter_mut().enumerate() {
            let offset = (self.slice_z[index] - centre_z).abs();
            if offset < radius {
                let reach = libm::sqrt(radius * radius - offset * offset);
                slice.push(disc(at, reach, self.tolerance));
            }
        }
    }
}

fn finish_layer(stage: &StageA, layer: usize, rings: Vec<Ring>) -> Rings {
    if rings.is_empty() {
        return Rings::new();
    }
    let grid = stage.derived.grid;
    let united = resolve(&rings, FillRule::NonZero);
    let collision = stage.clearance.collision(0, layer, true);
    let trimmed = difference(&united, &collision);
    let half = 0.5 * grid.scaled(NOTCH_CLOSING_MM);
    let round = Join::Round(grid.scaled(shapes::CHORD_TOLERANCE_MM));
    let closed = offset(&offset(&trimmed, half, round), -half, round);
    let filled = difference(
        &resolve(&[trimmed, closed].concat(), FillRule::NonZero),
        &collision,
    );
    simplify(
        &filled,
        grid.scaled(SIMPLIFY_MM),
        stage.flavour.strictly_simple(),
    )
}

//! The sphere graph organic smoothing runs on (spec 03 §2.1, §2.3): one sphere per placed
//! node, linked to its parents and its child, with the layers each sphere has to test
//! against the object and the spheres that never move.

use crate::support_model::memo::Fingerprint;
use crate::support_model::node::Node;
use crate::support_model::stage_a::StageA;

#[derive(Clone)]
pub struct Sphere {
    pub layer: usize,
    /// Position in units.
    pub position: [f64; 2],
    /// Radius in units.
    pub radius: f64,
    /// Height of the sphere's layer, units.
    pub z: f64,
    pub locked: bool,
    pub parents: Vec<usize>,
    pub child: Option<usize>,
    /// First and last layer whose obstacle the sphere tests.
    pub span: (usize, usize),
}

/// All spheres, layer by layer; `offsets[l]` is the id of layer l's first sphere.
pub struct Spheres {
    pub list: Vec<Sphere>,
    pub offsets: Vec<usize>,
}

impl Spheres {
    pub fn new(stage: &StageA, layers: &[Vec<Node>]) -> Spheres {
        let derived = &stage.derived;
        let grid = derived.grid;
        let mut offsets = Vec::with_capacity(layers.len() + 1);
        let mut total = 0;
        for layer in layers {
            offsets.push(total);
            total += layer.len();
        }
        offsets.push(total);
        let heights: Vec<f64> = stage
            .grid
            .layers
            .iter()
            .map(|layer| grid.scaled(layer.top))
            .collect();
        let mut list = Vec::with_capacity(total);
        for (layer, nodes) in layers.iter().enumerate() {
            for node in nodes {
                let centre = node.centre.expect("a placed node has a centre");
                list.push(Sphere {
                    layer,
                    position: [f64::from(centre.x), f64::from(centre.y)],
                    radius: node.real_radius(derived) as f64,
                    z: heights[layer],
                    locked: false,
                    parents: node
                        .parents
                        .iter()
                        .map(|&p| offsets[layer + 1] + p)
                        .collect(),
                    child: None,
                    span: (layer, layer),
                });
            }
        }
        let mut spheres = Spheres { list, offsets };
        spheres.link_children();
        spheres.lock();
        spheres.spans(&heights);
        spheres
    }

    fn link_children(&mut self) {
        for id in 0..self.list.len() {
            for parent in self.list[id].parents.clone() {
                self.list[parent].child = Some(id);
            }
        }
    }

    /// Tips never move, nor do nodes above layer 0 without a child (spec 03 §2.1).
    fn lock(&mut self) {
        for sphere in &mut self.list {
            sphere.locked =
                sphere.parents.is_empty() || (sphere.layer > 0 && sphere.child.is_none());
        }
    }

    /// The layers each sphere tests (spec 03 §2.3): bounded by the sphere itself and by its
    /// tree: not below the tree's bottom node, not above the lowest tip among its ancestors.
    fn spans(&mut self, heights: &[f64]) {
        let count = self.list.len();
        // Children have smaller ids than their parents: bottoms fill upward, tips downward.
        let mut bottom = vec![0.0; count];
        for id in 0..count {
            bottom[id] = match self.list[id].child {
                Some(child) => bottom[child],
                None => self.list[id].z,
            };
        }
        let mut lowest_tip = vec![0.0; count];
        for id in (0..count).rev() {
            lowest_tip[id] = if self.list[id].parents.is_empty() {
                self.list[id].z
            } else {
                self.list[id]
                    .parents
                    .iter()
                    .map(|&parent| lowest_tip[parent])
                    .fold(f64::MAX, f64::min)
            };
        }
        for id in 0..count {
            let sphere = &self.list[id];
            let low_z = bottom[id].max(sphere.z - sphere.radius);
            let high_z = lowest_tip[id].min(sphere.z + sphere.radius);
            let first = heights
                .iter()
                .position(|&z| z >= low_z)
                .unwrap_or(sphere.layer);
            let last = heights
                .iter()
                .rposition(|&z| z <= high_z)
                .unwrap_or(sphere.layer);
            self.list[id].span = (first.min(sphere.layer), last.max(sphere.layer));
        }
    }
}

impl Spheres {
    /// The trees (spheres linked through parents and children), each as its sphere ids in
    /// ascending order, in the order of their first sphere.
    pub fn trees(&self) -> Vec<Vec<usize>> {
        let mut tree_of = vec![usize::MAX; self.list.len()];
        let mut trees: Vec<Vec<usize>> = Vec::new();
        for start in 0..self.list.len() {
            if tree_of[start] != usize::MAX {
                continue;
            }
            tree_of[start] = trees.len();
            let mut members = vec![start];
            let mut stack = vec![start];
            while let Some(id) = stack.pop() {
                let sphere = &self.list[id];
                for &linked in sphere.parents.iter().chain(&sphere.child) {
                    if tree_of[linked] == usize::MAX {
                        tree_of[linked] = trees.len();
                        members.push(linked);
                        stack.push(linked);
                    }
                }
            }
            members.sort_unstable();
            trees.push(members);
        }
        trees
    }

    /// The spheres of `tree` renumbered from 0 in its order, and the fingerprint of all an
    /// update reads of them.
    pub fn alone(&self, tree: &[usize]) -> (u128, Vec<Sphere>) {
        let local = |id: usize| tree.binary_search(&id).expect("a linked sphere is in its tree");
        let mut key = Fingerprint::new(TREE_KEY);
        let spheres: Vec<Sphere> = tree
            .iter()
            .map(|&id| {
                let sphere = &self.list[id];
                let alone = Sphere {
                    parents: sphere.parents.iter().map(|&p| local(p)).collect(),
                    child: sphere.child.map(local),
                    ..sphere.clone()
                };
                key.int(alone.layer as i64)
                    .int(alone.position[0].to_bits() as i64)
                    .int(alone.position[1].to_bits() as i64)
                    .int(alone.radius.to_bits() as i64)
                    .int(alone.z.to_bits() as i64)
                    .int(i64::from(alone.locked))
                    .int(alone.child.map_or(-1, |child| child as i64))
                    .int(alone.span.0 as i64)
                    .int(alone.span.1 as i64)
                    .int(alone.parents.len() as i64);
                for &parent in &alone.parents {
                    key.int(parent as i64);
                }
                alone
            })
            .collect();
        (key.finish(), spheres)
    }
}

/// The memo kind of a smoothed tree.
const TREE_KEY: u8 = 8;

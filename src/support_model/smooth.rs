//! Stage B5: organic smoothing (spec 03 §2). Every node is a sphere that is pushed away from
//! the object's radius-0 collision boundary where it digs in, and pulled toward the mean of
//! its neighbours' previous positions (half toward the parents, half toward the child);
//! Orca builds also keep each link within about one slow step. The update reads only the
//! previous iteration's positions, so the result does not depend on evaluation order.
//!
//! Constants (spec 03 §2.4): extra gap 0.1 mm, push cap 0.5 mm applied squared, smoothing
//! cap 0.2 mm at factor ½, stop when no sphere digs in deeper than 1e-4 mm or after 100
//! iterations.

mod obstacle;
mod spheres;

use std::collections::BTreeMap;

use obstacle::SegmentGrid;
use spheres::{Sphere, Spheres};

use super::node::Node;
use super::stage_a::StageA;
use crate::polygon_ops::IntPoint;

const EXTRA_GAP_MM: f64 = 0.1;
const PUSH_CAP_MM: f64 = 0.5;
const SMOOTHING_CAP_MM: f64 = 0.2;
const SMOOTHING_FACTOR: f64 = 0.5;
const ENOUGH_MM: f64 = 1e-4;
const MAX_ITERATIONS: usize = 100;
const CELL_MM: f64 = 2.0;

/// Smoothed centres per layer and node (same indexing as the graph) and the iterations run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Smoothed {
    pub centres: Vec<Vec<IntPoint>>,
    pub iterations: usize,
}

/// Runs the pass over the placed `layers`.
pub fn smooth(stage: &StageA, layers: &[Vec<Node>]) -> Smoothed {
    let mut spheres = Spheres::new(stage, layers);
    let mut obstacles: BTreeMap<usize, SegmentGrid> = BTreeMap::new();
    let cell = stage.derived.grid.scaled(CELL_MM);
    for sphere in &spheres.list {
        for layer in sphere.span.0..=sphere.span.1 {
            obstacles.entry(layer).or_insert_with(|| {
                let rings = stage.clearance.collision(0, layer, true);
                SegmentGrid::new(&rings, cell)
            });
        }
    }
    let rule = Rule::new(stage);
    let mut iterations = 0;
    for _ in 0..MAX_ITERATIONS {
        iterations += 1;
        let previous: Vec<[f64; 2]> = spheres.list.iter().map(|s| s.position).collect();
        let mut deepest: f64 = 0.0;
        for id in 0..spheres.list.len() {
            let sphere = &spheres.list[id];
            if sphere.locked {
                continue;
            }
            let (next, depth) =
                rule.update(sphere, previous[id], &spheres.list, &previous, &obstacles);
            deepest = deepest.max(depth);
            spheres.list[id].position = next;
        }
        if deepest <= rule.enough {
            break;
        }
    }
    let centres = layers
        .iter()
        .enumerate()
        .map(|(layer, nodes)| {
            (0..nodes.len())
                .map(|index| {
                    let sphere = &spheres.list[spheres.offsets[layer] + index];
                    IntPoint::new(
                        libm::round(sphere.position[0]) as i32,
                        libm::round(sphere.position[1]) as i32,
                    )
                })
                .collect()
        })
        .collect();
    Smoothed {
        centres,
        iterations,
    }
}

/// The update rule's constants on the flavour grid.
struct Rule {
    layer: f64,
    extra_gap: f64,
    push_cap: f64,
    smoothing_cap: f64,
    enough: f64,
    slow_step: f64,
    anchored: bool,
    mm: f64,
}

impl Rule {
    fn new(stage: &StageA) -> Rule {
        let grid = stage.derived.grid;
        Rule {
            layer: stage.derived.layer_height as f64,
            extra_gap: grid.scaled(EXTRA_GAP_MM),
            push_cap: grid.scaled(PUSH_CAP_MM),
            smoothing_cap: grid.scaled(SMOOTHING_CAP_MM),
            enough: grid.scaled(ENOUGH_MM),
            slow_step: stage.derived.move_slow as f64,
            anchored: stage.flavour.smoothing_anchored(),
            mm: grid.scaled(1.0),
        }
    }

    /// The sphere's next position and how deep it dug in at its previous one.
    fn update(
        &self,
        sphere: &Sphere,
        at: [f64; 2],
        all: &[Sphere],
        previous: &[[f64; 2]],
        obstacles: &BTreeMap<usize, SegmentGrid>,
    ) -> ([f64; 2], f64) {
        let (depth, away) = self.collision(sphere, at, obstacles);
        let mut pushed = at;
        if depth > 0.0 {
            let reach = (depth + self.extra_gap).min(self.push_cap);
            // The push is the square of the clamped value, in mm (spec 03 §2.4).
            let step = reach / self.mm * reach;
            pushed = [at[0] + away[0] * step, at[1] + away[1] * step];
        }
        if self.anchored {
            pushed = self.anchor_limit(sphere, all, previous, pushed, at);
        }
        let target = self.target(sphere, previous);
        let pull = limit_length(
            [
                SMOOTHING_FACTOR * (target[0] - pushed[0]),
                SMOOTHING_FACTOR * (target[1] - pushed[1]),
            ],
            self.smoothing_cap,
        );
        let mut next = [pushed[0] + pull[0], pushed[1] + pull[1]];
        if self.anchored {
            next = self.anchor_limit(sphere, all, previous, next, at);
        }
        (next, depth)
    }

    /// Deepest dig-in over the tested layers and the unit direction away from the boundary.
    fn collision(
        &self,
        sphere: &Sphere,
        at: [f64; 2],
        obstacles: &BTreeMap<usize, SegmentGrid>,
    ) -> (f64, [f64; 2]) {
        let mut best = (0.0, [0.0, 0.0]);
        for layer in sphere.span.0..=sphere.span.1 {
            let offset = (layer as f64 - sphere.layer as f64) * self.layer;
            let rho2 = sphere.radius * sphere.radius - offset * offset;
            if rho2 <= 0.0 {
                continue;
            }
            let rho = libm::sqrt(rho2);
            let Some(grid) = obstacles.get(&layer) else {
                continue;
            };
            let Some((point, distance)) = grid.nearest(at, rho) else {
                continue;
            };
            let depth = rho - distance;
            if depth > best.0 {
                let away = if distance > 0.0 {
                    [(at[0] - point[0]) / distance, (at[1] - point[1]) / distance]
                } else {
                    [0.0, 0.0]
                };
                best = (depth, away);
            }
        }
        best
    }

    /// Mean of the parents (no child), or half parents and half child.
    fn target(&self, sphere: &Sphere, previous: &[[f64; 2]]) -> [f64; 2] {
        let count = sphere.parents.len().max(1) as f64;
        let mean = sphere.parents.iter().fold([0.0, 0.0], |sum, &p| {
            [sum[0] + previous[p][0], sum[1] + previous[p][1]]
        });
        let mean = [mean[0] / count, mean[1] / count];
        match sphere.child {
            Some(child) => [
                0.5 * mean[0] + 0.5 * previous[child][0],
                0.5 * mean[1] + 0.5 * previous[child][1],
            ],
            None => mean,
        }
    }

    /// Orca builds: a candidate may not leave the disc around a linked sphere's previous
    /// position whose radius is the larger of the allowed distance and the current one.
    fn anchor_limit(
        &self,
        sphere: &Sphere,
        all: &[Sphere],
        previous: &[[f64; 2]],
        mut candidate: [f64; 2],
        current: [f64; 2],
    ) -> [f64; 2] {
        if let Some(child) = sphere.child {
            let allowed = (all[child].radius - sphere.radius).max(0.0) + self.slow_step;
            candidate = project(candidate, previous[child], allowed, current);
        }
        for &parent in &sphere.parents {
            let allowed = (sphere.radius - all[parent].radius).max(0.0) + self.slow_step;
            candidate = project(candidate, previous[parent], allowed, current);
        }
        candidate
    }
}

fn limit_length(vector: [f64; 2], cap: f64) -> [f64; 2] {
    let length = libm::hypot(vector[0], vector[1]);
    if length > cap {
        [vector[0] / length * cap, vector[1] / length * cap]
    } else {
        vector
    }
}

fn project(candidate: [f64; 2], anchor: [f64; 2], allowed: f64, current: [f64; 2]) -> [f64; 2] {
    let reach = allowed.max(libm::hypot(current[0] - anchor[0], current[1] - anchor[1]));
    let offset = [candidate[0] - anchor[0], candidate[1] - anchor[1]];
    let length = libm::hypot(offset[0], offset[1]);
    if length > reach {
        [
            anchor[0] + offset[0] / length * reach,
            anchor[1] + offset[1] / length * reach,
        ]
    } else {
        candidate
    }
}

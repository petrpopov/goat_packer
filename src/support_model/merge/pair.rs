//! The pair merge test and its result (spec 02 §4.5): two branches merge when the area one
//! can reach, grown by the difference of their radii, overlaps the other's thickly enough;
//! the merged branch keeps the destination's identity, the larger radius and a hint at the
//! overlap closest to the destination's old hint.

use super::super::descend::{
    Moved,
    soi::{StepParams, grow_through},
};
use crate::polygon_ops::{
    IntBox, IntPoint, Join, Rings, area, bbox, intersection, nearest_inside, offset, union,
};
use crate::support_model::node::Node;
use crate::support_model::stage_a::StageA;

/// The overlap must survive shrinking by this (mm): thicker than about 0.05 mm.
const THICK_OVERLAP_MM: f64 = 0.025;

/// Mitre limit asked for by the overlap erosion (acts as 2).
const EROSION_MITER_LIMIT: f64 = 1.2;

/// The box padding of a merged candidate (mm).
const BOX_PADDING_MM: f64 = 0.0001;

/// A merge test bound to one layer step.
pub struct PairTest<'a> {
    pub stage: &'a StageA,
    /// The layer the candidates move to.
    pub layer: usize,
    epsilon: f64,
}

impl<'a> PairTest<'a> {
    pub fn new(stage: &'a StageA, layer: usize) -> PairTest<'a> {
        let tiny = stage.derived.grid.scaled(0.001);
        PairTest {
            stage,
            layer,
            epsilon: tiny * tiny,
        }
    }

    /// `destination` absorbs `source`: the merged candidate, or `None` when refused.
    pub fn try_merge(&self, destination: &Moved, source: &Moved) -> Option<Moved> {
        let derived = &self.stage.derived;
        let (dst, src) = (&destination.node, &source.node);
        if dst.graceful != src.graceful || dst.tight_xy != src.tight_xy {
            return None;
        }
        let (dst_radius, src_radius) =
            (dst.collision_radius(derived), src.collision_radius(derived));
        let destination_is_bigger = dst_radius > src_radius;
        let (bigger, smaller) = if destination_is_bigger {
            (destination, source)
        } else {
            (source, destination)
        };
        let delta = (bigger.node.real_radius(derived) - smaller.node.real_radius(derived)).abs();
        if !boxes_meet(smaller.bounds?, bigger.bounds?, delta as i32) {
            return None;
        }
        if !(dst.to_bed && src.to_bed) {
            return None;
        }
        if !bigger.node.past_holes && smaller.node.past_holes {
            return None;
        }
        let tight = bigger.node.tight_xy && smaller.node.tight_xy;
        let small_radius = smaller.node.collision_radius(derived);
        let walls = self
            .stage
            .clearance
            .collision(small_radius, self.layer, tight);
        let reach = |region: &Rings| self.grow(region, delta, &walls, small_radius);
        let overlap = intersection(&reach(&smaller.plate), &bigger.plate);
        if area(&overlap) <= self.epsilon
            || !thick(
                &overlap,
                derived.grid.scaled(THICK_OVERLAP_MM),
                self.epsilon,
            )
        {
            return None;
        }
        let hint = nearest_inside(&overlap, dst.hint);
        let node = self.merged_node(dst, src, hint);
        let stored = union(
            &intersection(&reach(&smaller.node.area), &bigger.node.area),
            &overlap,
        );
        let pad = derived.grid.scaled(BOX_PADDING_MM) as i32;
        let bounds = bbox(&stored).map(|found| padded(found, pad));
        Some(Moved {
            node: Node {
                area: stored,
                ..node
            },
            bypass: false,
            plate: overlap,
            bounds,
        })
    }

    /// grow(X): one round offset by Δ through the smaller branch's collision.
    fn grow(&self, region: &Rings, delta: i64, walls: &Rings, small_radius: i64) -> Rings {
        let derived = &self.stage.derived;
        let params = StepParams {
            grid: derived.grid,
            strict: self.stage.flavour.strictly_simple(),
            safe_step: 2 * (derived.xy + small_radius - 3),
            careless: 0,
            parts: 0,
        };
        grow_through(region, delta, walls, &params)
    }

    fn merged_node(&self, dst: &Node, src: &Node, hint: IntPoint) -> Node {
        let derived = &self.stage.derived;
        let higher = if dst.origin_layer > src.origin_layer {
            dst
        } else {
            src
        };
        let rdepth = dst.rdepth.max(src.rdepth);
        let radius = dst
            .collision_radius(derived)
            .max(src.collision_radius(derived));
        let foot = if derived.foot_increment > 0.0 {
            (radius - derived.radius(rdepth, 0.0)).abs() as f64 / derived.foot_increment
        } else {
            0.0
        };
        let mut parents = dst.parents.clone();
        parents.extend(&src.parents);
        Node {
            layer: dst.layer,
            depth: dst.depth.max(src.depth),
            rdepth,
            foot,
            origin_layer: higher.origin_layer,
            origin_xy: higher.origin_xy,
            hint,
            centre: None,
            to_bed: dst.to_bed && src.to_bed,
            graceful: dst.graceful && src.graceful,
            tight_xy: dst.tight_xy || src.tight_xy,
            under_roof: dst.under_roof || src.under_roof,
            past_holes: dst.past_holes || src.past_holes,
            keep_round: false,
            hold_until: dst.hold_until.max(src.hold_until),
            model_growth: dst.model_growth.max(src.model_growth),
            parents,
            area: Rings::new(),
            deleted: false,
            marked: false,
        }
    }
}

fn boxes_meet(small: IntBox, big: IntBox, grow: i32) -> bool {
    small.min.x - grow <= big.max.x
        && big.min.x <= small.max.x + grow
        && small.min.y - grow <= big.max.y
        && big.min.y <= small.max.y + grow
}

/// The overlap still has area after eroding by `radius` units.
fn thick(overlap: &Rings, radius: f64, epsilon: f64) -> bool {
    let eroded = offset(overlap, -radius, Join::Miter(EROSION_MITER_LIMIT));
    area(&eroded) > epsilon
}

fn padded(found: IntBox, pad: i32) -> IntBox {
    IntBox {
        min: IntPoint::new(found.min.x - pad, found.min.y - pad),
        max: IntPoint::new(found.max.x + pad, found.max.y + pad),
    }
}

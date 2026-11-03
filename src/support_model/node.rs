//! One branch node of the tree generator (spec 02 §3.1, glossary 02 §1.4): a position on a
//! layer with the radius bookkeeping, flags, the parents it grew from and the influence area
//! (IA) of positions it may still take. Tips are created by `tips`, moved down by `descend`,
//! merged by `merge` and given centres by `place`.

use crate::polygon_ops::{IntPoint, Rings};

use super::settings::Derived;

/// Only what later steps read is stored, not why a branch moved the way it did.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub layer: usize,
    /// Layers below the branch's highest tip.
    pub depth: i64,
    /// Radius height: normally `depth`, but may lag (a branch cannot widen where it would
    /// hit the model).
    pub rdepth: i64,
    /// Foot-growth count, fractional after merges.
    pub foot: f64,
    /// The tip this branch ultimately serves (the higher one after a merge).
    pub origin_layer: usize,
    pub origin_xy: IntPoint,
    /// Where placement tries to put the node: the tip, or the last merge point.
    pub hint: IntPoint,
    /// The placed position; `None` until placement (or a dont-move copy) sets it.
    pub centre: Option<IntPoint>,
    pub to_bed: bool,
    pub graceful: bool,
    /// Still allowed the reduced xy distance.
    pub tight_xy: bool,
    pub under_roof: bool,
    /// May use the hole-closed (slow, fast-safe) avoidances.
    pub past_holes: bool,
    pub keep_round: bool,
    /// The branch stays vertical while `depth` is below this.
    pub hold_until: i64,
    pub model_growth: i64,
    /// Indices into layer `layer + 1` (toward the tips).
    pub parents: Vec<usize>,
    pub area: Rings,
    pub deleted: bool,
    pub marked: bool,
}

impl Node {
    /// The collision radius c(e) = R(rdepth, foot) (spec 02 §1.2).
    pub fn collision_radius(&self, derived: &Derived) -> i64 {
        derived.radius(self.rdepth, self.foot)
    }

    /// The real radius ρ(e) = R(d_eff, foot): what the branch will be drawn with.
    pub fn real_radius(&self, derived: &Derived) -> i64 {
        derived.radius(self.effective_depth(derived), self.foot)
    }

    /// d_eff: the radius height drawing uses (02 §1.2).
    pub fn effective_depth(&self, derived: &Derived) -> i64 {
        if self.rdepth < derived.increase_until_layer {
            self.depth.min(derived.increase_until_layer)
        } else {
            self.rdepth
        }
    }
}

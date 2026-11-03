//! The exact predicates the refinement moves copies by: `VectorValidator`'s own pair and zone
//! tests, so a layout the oracle accepts is a layout the publication gate accepts.

use crate::geometry::{Point, Rect};
use crate::validation::VectorValidator;

/// Every pair gap is checked this much wider than the rule, mm. The search stops at the
/// exact contact otherwise, and a contact at float precision is a coin toss for any later
/// re-validation of the same numbers (the 3MF writes ten decimals).
pub(super) const PAD_MM: f64 = 2e-6;

pub(super) struct Oracle<'a> {
    validator: &'a VectorValidator,
    /// Per class: everything it prints plus half the widest gap, print frame.
    reach: Vec<Rect>,
}

impl<'a> Oracle<'a> {
    pub(super) fn new(validator: &'a VectorValidator, classes: usize) -> Oracle<'a> {
        Oracle {
            validator,
            reach: (0..classes).map(|c| validator.reach_box(c)).collect(),
        }
    }

    /// Whether copies of `a` and `b`, `b` at `shift` from `a`, can come close enough to
    /// matter (a box test; a far pair is clear without the polygon work).
    pub(super) fn may_meet(&self, a: usize, b: usize, shift: Point) -> bool {
        let (p, q) = (self.reach[a], self.reach[b].translated(shift.x, shift.y));
        p.min_x <= q.max_x && q.min_x <= p.max_x && p.min_y <= q.max_y && q.min_y <= p.max_y
    }

    /// The pair breaks no rule (with the pad).
    pub(super) fn pair_clear(&self, a: usize, b: usize, shift: Point) -> bool {
        !self.may_meet(a, b, shift) || self.validator.pair_clear_padded(a, b, shift, PAD_MM)
    }

    pub(super) fn zones_clear(&self, class: usize, at: Point) -> bool {
        self.validator.zones_clear(class, at)
    }
}

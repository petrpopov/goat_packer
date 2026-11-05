//! How far a branch may move in one layer (spec 02 §3.3): the preferred (slow) and the
//! fast speed from the angles, plus what the growing radius and the growing foot add to them
//! so a widening branch does not outrun its own circle.

use crate::support_model::node::Node;
use crate::support_model::stage_a::StageA;

/// Both speeds, grid units per layer, and the pieces that make them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speeds {
    pub fast: i64,
    pub slow: i64,
    /// Added to both speeds; the part of the slow speed beyond that is `slow_only`.
    pub both: i64,
    pub slow_only: i64,
}

/// The rounding guard both speeds start with (5 grid units, spec 02 §3.3).
const GUARD_UNITS: i64 = 5;

/// The speeds of `parent` on `layer`, moving down to `layer - 1`.
pub fn speeds(stage: &StageA, parent: &Node, layer: usize) -> Speeds {
    let derived = &stage.derived;
    let keys = stage.clearance.keys();
    let tight = parent.tight_xy;
    let mut both = GUARD_UNITS;
    let mut slow_only = 0;
    let current = parent.collision_radius(derived);
    let grown = derived.radius(parent.rdepth + 1, parent.foot);
    let growth = grown - current;
    if keys.key(grown, tight) == keys.key(current, tight) || grown < derived.increase_until_radius {
        both += growth;
    } else {
        slow_only += growth.min(derived.move_fast + both - derived.move_slow - slow_only);
    }
    let foot_step = derived.foot_growth as i64;
    let below = layer as i64 - 1;
    if derived.foot_in_descent
        && (layer as i64) < derived.foot_layers
        && derived.recommended_min_radius(below) < grown
    {
        let with_foot = derived.radius(parent.rdepth + 1, parent.foot + 1.0);
        if keys.key(with_foot, tight) == keys.key(current, tight) {
            both += foot_step;
        } else {
            slow_only += foot_step.min(derived.move_fast - derived.move_slow - slow_only);
        }
    }
    Speeds {
        fast: derived.move_fast + both,
        slow: derived.move_slow + both + slow_only,
        both,
        slow_only,
    }
}

//! Stage B7: the first-layer support base (spec 03 §4.2). The roots' discs grow outward by the
//! flange expansion in several small steps, each step cut back by the object's own uncompensated
//! first layer grown by the first-layer gap (square joins): a geodesic dilation that spreads
//! through free space and cannot jump over the gap around the model. That cut is the
//! slicer's own mask, not the packer's conservative landing, which would under-predict the
//! base.

use super::flavour::FirstLayerExpansion;
use super::stage_a::StageA;
use crate::polygon_ops::{DEFAULT_MITER_LIMIT, Join, Rings, difference, offset};

/// The automatic expansion Bambu reads from `raft_first_layer_expansion = -1`, mm.
const AUTOMATIC_EXPANSION_MM: f64 = 2.0;

/// An expansion not larger than this is none (spec 03 §4.2 step 1), mm.
const NEGLIGIBLE_MM: f64 = 0.0001;

/// At least this many growth steps.
const MIN_STEPS: i64 = 5;

/// The expansion distance of the flavour in mm (0 = none).
pub fn expansion_mm(stage: &StageA) -> f64 {
    let value = stage.derived.first_layer_expansion_mm;
    match stage.flavour.first_layer_expansion() {
        FirstLayerExpansion::AutoSteps if value < 0.0 => AUTOMATIC_EXPANSION_MM,
        FirstLayerExpansion::AutoSteps => value,
        FirstLayerExpansion::MinusFine => value.max(0.0),
    }
}

/// The base from the roots' trimmed first-layer discs.
pub fn first_layer_base(stage: &StageA, roots: &Rings) -> Rings {
    let derived = &stage.derived;
    let grid = derived.grid;
    let Some(first) = stage.sections.area.first() else {
        return roots.clone();
    };
    let mask = offset(first, grid.scaled(derived.first_layer_gap_mm), Join::Square);
    let expansion = expansion_mm(stage);
    if expansion <= NEGLIGIBLE_MM {
        return difference(roots, &mask);
    }
    let steps = MIN_STEPS.max(libm::ceil(expansion / derived.first_layer_width_mm) as i64);
    let step = grid.scaled(expansion / steps as f64);
    let mut grown = roots.clone();
    for _ in 0..steps {
        let wider = offset(&grown, step, Join::Miter(DEFAULT_MITER_LIMIT));
        grown = difference(&wider, &mask);
    }
    grown
}

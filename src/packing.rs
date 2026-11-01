//! The packing entry point: validates the input and runs the deterministic search.

use crate::footprint::Footprint;
use crate::options::{PackOptions, PackResult, check_pack_input};
use crate::packing_search;

pub fn pack_footprint(footprint: &Footprint, options: &PackOptions) -> PackResult {
    assert!(options.resolution.is_finite() && options.resolution > 0.0);
    assert!(
        options.angle_step_deg.is_finite()
            && options.angle_step_deg > 0.0
            && options.angle_step_deg <= 360.0
    );
    check_pack_input(footprint, options).expect("invalid packing input");
    packing_search::search(footprint, options)
}

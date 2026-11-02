//! The brick-row baseline as a public entry point: a thin wrapper over the packing
//! portfolio's legacy run with only the rows member (`pack::rows`).

use crate::footprint::Footprint;
use crate::options::{PackOptions, PackResult, check_pack_input};
use crate::pack::legacy::{Portfolio, pack_legacy};

/// Deterministic brick-row baseline in the same canonical frame as the search: copies on the
/// densest sheared row lattice at 0° and (for an oblong footprint that may turn) 90°, every
/// placed copy checked, the winner validated like any search result.
pub fn grid_layout(footprint: &Footprint, options: &PackOptions) -> PackResult {
    check_pack_input(footprint, options).expect("invalid packing input");
    pack_legacy(footprint, options, Portfolio::Rows)
}

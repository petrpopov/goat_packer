//! The one facade over polygon booleans, offsets and simplification for the support model
//! (design 2026-11-02 §5.2, lead decision Q6): integer coordinates on the slicer flavour's
//! grid, `i_overlay` underneath, and the Clipper-1 semantics the slicers rely on emulated
//! wherever they change a discrete decision downstream:
//!
//! - `grid`: the flavour grid (B 10 nm, O/R/S 1 nm) and the truncating mm → units rule;
//! - `boolean`: union, intersection, difference, xor and fill-rule resolution;
//! - `offset`: Clipper-1 offsets (mitre-limit floor 2, arc-tolerance rule, square joins,
//!   the 0.005·|d| input decimation, f32 deltas, one ring at a time);
//! - `simplify`: Douglas-Peucker and the ring self-union of the slicers' simplification;
//! - `query`: area, bounding box, point containment, nearest boundary point.
//!
//! Stages see only these types and functions, so a Clipper-1-compatible engine can replace
//! `i_overlay` behind the facade without touching them. Every call is single-threaded and
//! integer-exact, so results are bit-identical across machines (design §5.2 rules). What the
//! backend does not reproduce (ring start vertices, component order, vertex splitting of
//! touching rings) goes into the calibrated margin δ (spec 02 §6 item 2).

mod boolean;
mod grid;
mod offset;
mod query;
mod simplify;

pub use i_overlay::core::fill_rule::FillRule;
pub use i_overlay::i_float::int::point::IntPoint;

pub use boolean::{
    difference, difference_islands, intersection, intersection_islands, islands, overlaps, resolve,
    union, union_all, union_islands, union_many, xor,
};
pub use grid::{Grid, MAX_COORDINATE};
pub use offset::{DEFAULT_MITER_LIMIT, Join, offset};
pub use query::{
    Containment, IntBox, area, bbox, containment, contains, distance_to_boundary,
    inside_even_odd, nearest_boundary, nearest_inside, ring_doubled_area,
};
pub use simplify::{douglas_peucker, simplify};

/// One closed ring on the grid, without a repeated first point.
pub type Ring = Vec<IntPoint>;

/// Rings whose union under the non-zero rule is a region. Results of this module are
/// resolved: outer rings counter-clockwise, holes clockwise.
pub type Rings = Vec<Ring>;

/// One polygon with holes (the slicers' `ExPolygon`): the outer ring first, counter-clockwise,
/// then its holes, clockwise.
pub type Island = Vec<Ring>;

/// Every ring of `islands`, in island order.
pub fn flatten(islands: Vec<Island>) -> Rings {
    islands.into_iter().flatten().collect()
}

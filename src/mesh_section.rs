//! Mesh geometry of the source object (design 2026-11-02 §4.2, work package S1):
//!
//! - `section`: exact cross-sections at one height for the support model (spec 01 §1.3-1.4):
//!   triangle ∩ plane (`plane`), loop chaining (`chain`), the closing radius (`offset`).
//! - `slab`: the body per absolute band (projection of the material in the band's slab) and
//!   the silhouette.
//! - `hull`: per-part convex hulls and boxes.
//! - `print`: those kinds as an `ObjectPrint` for one rotation class.
//!
//! Booleans run on the 1 nm grid (`area`), single-threaded, so every result is bit-identical
//! across machines.

pub(crate) mod area;
mod chain;
pub(crate) mod hull;
mod offset;
mod plane;
mod print;
mod section;
mod slab;

pub use chain::MAX_GAP_MM;
pub use hull::{convex_hull, part_boxes, part_hulls};
pub use offset::CLOSING_MITER_LIMIT;
pub use print::body_print;
pub use section::{
    SECTION_SIMPLIFY_MM, SectionOptions, object_section, part_section, raw_section_loops,
};
pub use slab::{BodyBands, body_bands, silhouette, slab_polygon, wall_bulge_mm};

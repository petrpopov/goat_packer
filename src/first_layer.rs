//! The first-layer kinds of an object (design 2026-11-02 §6, work package I3):
//!
//! - `landing`: the first-layer printed body, the section at the middle of layer 0 grown by
//!   the wall overshoot (spec 05);
//! - `brim`: the object brim ring with the per-build auto-width rule (spec 03 §5).
//!
//! `project` holds what the first-layer and tower models share: the slicer build a prediction
//! follows, typed project settings and the `Unsupported` refusal (design §5.7). `grow` is the
//! conservative round offset both kinds are built with.

mod brim;
mod brim_groups;
mod brim_width;
mod grow;
mod islands;
mod landing;
mod project;

pub use brim::{Brim, brim};
pub use brim_width::{
    AreaMoments, GroupShape, IslandShape, Material, adhesion_of, auto_width, island_width,
    max_print_speed, quantised, thermal_length_of,
};
pub use grow::{
    grow_round, intersect as intersect_loops, ring_between, subtract as subtract_loops,
};
pub use islands::{Island, islands};
pub use landing::{CLASSIC_OVERSHOOT_MM, Landing, first_layer_height, landing};
pub use project::{ProjectSettings, SlicerBuild, Unsupported, bead_spacing};

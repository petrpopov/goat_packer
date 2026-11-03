//! 2D irregular-nesting packer for FDM printing and the tools built around it.
//!
//! The packing core, from the bottom up:
//! - `geometry`: points, rectangles, loop (polygon) helpers.
//! - `footprint`: what one object occupies on the bed (`Footprint`), built from an STL
//!   (`stl`) or from a probe slice's first layer (`gcode_footprint`).
//! - `span_raster` / `pixel_mask` / `raster_mask`: the footprint as pixels, one mask per angle.
//! - `collision`: when two masks (or a mask and the bed) conflict.
//! - `packing` (+ `packing_search`, `frame`): the search itself and the grid baseline.
//! - `validation`: an independent re-check of a finished layout.
//!
//! The slicer-free redesign (docs/superpowers/specs/2026-11-02-slicer-free-packer-design.md)
//! adds `object_print` (an object split into typed kinds), `keepout` (plate zones and the
//! permission matrix over kinds) and the modules named in its section 12.
//!
//! Everything the binaries and tests use is re-exported here.

mod collision;
mod footprint;
mod frame;
mod gcode_footprint;
mod gcode_text;
mod geometry;
mod grid;
mod layout;
mod options;
mod packing;
mod packing_search;
mod pixel_mask;
mod raster_mask;
mod span_raster;
mod stl;
mod validation;
mod validation_geometry;

pub mod ams_calibration;
pub mod assemble;
pub mod bed_outline;
pub mod colored_packer;
pub mod embedded_mesh;
pub mod first_layer;
pub mod gcode_collisions;
pub mod gcode_frame;
pub mod keepout;
pub mod kind_masks;
pub mod mesh_section;
pub mod nozzle_clean;
pub mod object_print;
pub mod orca_presets;
pub mod orca_project;
pub mod pack;
pub mod paint;
pub mod plate_centre;
pub mod plate_preview;
pub mod polygon_ops;
pub mod print_time;
pub mod process_output;
pub mod project_settings_edit;
pub mod progress;
pub mod project_application;
pub mod purge_detect;
pub mod scheduler;
pub mod slicer_cli;
pub mod support_model;
pub mod tower_model;

pub use collision::{placement_is_allowed, placements_collide};
pub use footprint::{
    Footprint, LAYER_ZONE_GAP_MM, LayerZone, SUPPORT_BODY_GAP_MM, patch_overshoot_mm,
    patch_within_silhouette,
};
pub use gcode_footprint::{
    SlicedObject, first_layer_footprint, first_layer_footprint_of, realization_from_gcode,
    realizations_from_gcode, support_interface_loops, support_interface_loops_of,
};
pub use geometry::{
    Point, Rect, axis_aligned_line_keepout, grow_beyond, loops_touch_rect, point_in_footprint,
    polygon_area, union_loops,
};
pub use grid::grid_layout;
pub use keepout::{ForeignGaps, Keepout, KeepoutClass, PlateRules};
pub use object_print::{ObjectPrint, PrintKind, Realization};
pub use options::{PackOptions, PackResult, PackedPlacement};
pub use packing::pack_footprint;
pub use pixel_mask::{PixelMask, masks_overlap};
pub use raster_mask::{RasterMask, rasterize_footprint, rasterize_footprint_with_support};
pub use stl::extract_footprint;
pub use validation::{
    LayoutRules, PrintPlacement, RasterValidator, VectorValidator, validate_raster,
    validate_result, validate_vector,
};

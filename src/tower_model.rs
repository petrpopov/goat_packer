//! The prime tower as the slicer will print it (design 2026-11-02 §6, spec 04, work package
//! I3), without slicing:
//!
//! - `sources` + `schedule` + `changes`: the filaments of every layer from part extruders,
//!   layer ranges and colour paint, the tool changes per layer and the tower top H_t
//!   (04 §4.1-§4.2);
//! - `bambu` / `type2`: the column size each build plans from that schedule (04 §4.3-§4.4);
//! - `outline`: the wall centre line (rectangle, or Bambu's filleted rib wall) and the
//!   mitred offsets of the brim and chamfer loops (04 §4.5-§4.6);
//! - `print`: `TowerPrint` and its plate zones: the column and the chamfer rings of layers
//!   ≥ 1 are tower body, the L0 brim ring is tower brim (design §4.2, §14 Q18);
//! - `anchor`: the anchor range each build's GUI or CLI keeps (04 §6, design §6);
//! - `forced`: the plain tower of a chosen width the packer writes into the project.
//!
//! The tower does not depend on object count (04 §8.2) nor, with support filament 0, on the
//! support realization (04 §7 N1); everything here is a deterministic function of the project.

mod anchor;
mod bambu;
mod changes;
mod forced;
mod formulas;
mod outline;
mod print;
mod schedule;
mod settings;
mod sources;
mod type2;

pub use anchor::{
    BAMBU_GUI_MARGIN_MM, ORCA_GUI_MARGIN_MM, ORCA25_SNAP_MM, TowerExtent, anchor_range,
    bambu_gui_estimate, printed_anchor, snapmaker_gui_depth,
};
pub use bambu::{Column, Rib};
pub use changes::{LayerOrdering, changes_per_layer};
pub use forced::{DEFAULT_TOWER_WIDTH_MM, force_tower_shape};
pub use formulas::{auto_brim_width, bead_area, brim_loops, min_depth_for_height};
pub use outline::{BRIM_MITRE_LIMIT, grow_mitred, rectangle, rib_outline};
pub use print::{
    ChamferRing, TOWER_EDGE_TOLERANCE_MM, TowerPlate, TowerPrint, TowerZone, printed_tower,
};
pub use schedule::{LayerGrid, ToolLayer, ToolSchedule, tool_schedule};
pub use settings::{Ramming, TowerSettings};
pub use sources::{FilamentSources, LayerRange, read_filament_sources};

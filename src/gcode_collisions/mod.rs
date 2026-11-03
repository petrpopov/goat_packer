//! Layer-by-layer check of a sliced plate: does any object's support touch another object's
//! printed body? (spec 2026-10-31-support-landing-and-progress-design, rule 4)
//!
//! The packer reserves a support patch measured on a one-object probe, but the slicer
//! re-generates tree supports for every plate and the realization drifts between slices of
//! the same project (565 / 617 / 643 / 657 mm² of first-layer support for one P1S goat). So the
//! final G-code itself is the evidence: every printed line is attributed to its object via
//! the slicer's object labels and classified as support or body, and per layer each support
//! line is tested against foreign body lines. Two lines collide when their centre lines come
//! closer than the sum of their half widths (they touch or overlap).
//!
//! Both dialects: BambuStudio labels objects with `; start printing object, unique label id:
//! N` / `; stop printing object, unique label id: N` (a single object only with `; OBJECT_ID:
//! N`), OrcaSlicer with `; printing object NAME id:N copy M` / `; stop printing object ...`
//! (plus Klipper `EXCLUDE_OBJECT_START NAME=...` / `EXCLUDE_OBJECT_END`); `reader` explains
//! which one a plate is read by. Layers: `; CHANGE_LAYER` / `;LAYER_CHANGE`; features:
//! `; FEATURE:` / `;TYPE:`; widths: `; LINE_WIDTH:` / `;WIDTH:`. Lines outside any object
//! (prime tower, purge, skirt) are ignored.
//!
//! Modules: `reader` (walks the G-code; `labels` names the objects), `parser` (every object's
//! lines), `touching` (the per-layer geometry), `types` (the report), `tower_layers` (the
//! prime tower per layer), `gui_checks` (what the slicer GUI would refuse, with `conflicts`,
//! `part_edges` and `plane`).

mod conflicts;
pub mod gui_checks;
mod labels;
mod parser;
mod part_edges;
mod plane;
mod reader;
#[cfg(test)]
mod tests;
mod touching;
mod tower_layers;
mod types;

use parser::{extents_of, parse};

pub use conflicts::ConflictKind;
pub(crate) use parser::{KeptLine, object_lines};

pub use tower_layers::{TowerLayer, tower_layers};
pub use types::{ObjectFirstLayer, PlateAnalysis, Strip, SupportCollision};

/// Parses a sliced plate once: support-vs-foreign-body collisions on every layer plus every
/// object's first-layer lines (for learning the realized support patch).
pub fn analyze_plate(gcode: &[u8]) -> PlateAnalysis {
    let parser = parse(gcode, false);
    let extents = extents_of(&parser);
    PlateAnalysis {
        extents,
        collisions: parser.collisions.into_values().collect(),
        first_layer: parser.first_layer,
        layers: parser.layer,
        labelled_lines: parser.labelled_lines,
    }
}

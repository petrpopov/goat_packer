//! Plate zones and the rules between printed kinds (design 2026-11-02, §7): what a profile
//! declares (`Keepout`, `PlateRules`), the permission matrix of kinds against zones
//! (`matrix`) and the pairs of kinds that must stay apart (`pair_rules`).
//!
//! A profile declares geometry and a class only. What a class forbids lives in `matrix`, never
//! in profile data.

mod matrix;
mod rules;

use crate::geometry::{Point, Rect};
use crate::object_print::band_z_range;
use serde::{Deserialize, Serialize};

pub use matrix::{Gap, PairRule, Permission, ZoneClass, pair_gap, pair_rules, permission};
pub use rules::{BodyRule, PlateRules};

/// What a zone forbids, in decreasing strictness (the matrix columns for profile zones).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepoutClass {
    /// Nothing may print there; copies' supports are not clipped by the slicer.
    Hard,
    /// The model body may not enter; supports and brims may.
    Restricted,
    /// Only the first-layer body may not cover it (purge and prime lines).
    Soft,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepoutShape {
    Rect(Rect),
    /// One closed ring.
    Polygon(Vec<Point>),
    /// Printed lines as `[x1, y1, x2, y2, width]`, like `PrinterProfile::purge_lines`.
    Strips(Vec<[f64; 5]>),
    /// A band of this width (mm) inside the printable area along its edges.
    EdgeBand(f64),
}

/// The coordinate frame of a keepout's numbers.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepoutFrame {
    #[default]
    Plate,
    /// G-code coordinates: the plate position is these plus the project's `extruder_offset`
    /// (P1S: (0, 2)), because BambuStudio writes G-code minus it.
    Gcode,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Keepout {
    pub id: String,
    pub class: KeepoutClass,
    pub shape: KeepoutShape,
    /// `[from, to]` in mm; `None` = every height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_range_mm: Option<[f64; 2]>,
    #[serde(default)]
    pub frame: KeepoutFrame,
}

impl Keepout {
    /// Whether a kind printed in absolute band `band` can meet this zone. A zone ending at
    /// `to` still holds the layer printed at z = `to`, which is the first one of the next band.
    pub fn meets_band(&self, band: usize) -> bool {
        let (band_from, band_to) = band_z_range(band);
        self.z_range_mm
            .is_none_or(|[from, to]| from < band_to && to >= band_from)
    }
}

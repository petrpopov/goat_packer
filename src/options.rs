//! What a packing run is given (`PackOptions`) and what it returns (`PackResult`).

use crate::footprint::Footprint;
use crate::geometry::{Point, Rect, polygon_area, rotate_loops};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackOptions {
    /// Where the whole printed extent (`Footprint::bed_loops`: patch ∪ silhouette) must stay;
    /// also the origin of the placement pixel grid.
    pub bed: Rect,
    /// Where the model body (the body zone's bbox: silhouette plus support interfaces; the
    /// whole extent for bodyless footprints) must stay when the supports may use more of the
    /// bed than the model: a profile's edge margin keeps the meshes off the edge, while the
    /// supports only need the printable area. A hand-made 48-goat Snapmaker U1 plate printed
    /// its support bases 0.7 mm from the printable edge with the meshes 3.7 mm off it; with
    /// the 4 mm margin binding the 47.6 mm support patch the packer found 45. `None` = `bed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_bed: Option<Rect>,
    pub exclusions: Vec<Rect>,
    /// Exclusions only the model body must clear (body bbox + clearance), while the support
    /// part of the patch may enter: the prime tower keepout. Tree supports reaching into the
    /// tower's area print as purge-side scrap, and a P1S plate printed fine with them; the
    /// bed exclusion corner and purge lines stay in `exclusions` (full-extent bbox).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_exclusions: Vec<Rect>,
    pub resolution: f64,
    pub clearance: f64,
    pub angle_step_deg: f64,
    /// Deterministic search effort. All orientation pairs get a baseline pass; higher values
    /// explore additional sweep directions and starting orientations, up to the finite portfolio.
    pub attempts: usize,
    /// Compatibility field for older clients; free centers are now searched exhaustively.
    pub candidates_per_step: usize,
    /// Compatibility field. The finite deterministic portfolio does not use racy stall stopping.
    pub stall_attempts: usize,
    /// Hard cap on objects per layout (0 = unlimited). The search itself stops when the bed
    /// is full; a cap is only for deliberately sparse plates or bounded slicing time.
    pub max_objects: usize,
    /// Pack every object at 0°. Tree supports are rebuilt by the slicer in table coordinates,
    /// so a footprint measured at one orientation silently stops matching the printed patch
    /// once the model rotates; a locked orientation keeps every clone identical to the probe.
    pub lock_orientation: bool,
    /// Retained in reports for compatibility; geometry, not a random seed, determines layouts.
    pub seed: u64,
    /// Supports (full patch minus body) of different objects may overlap by at most this many
    /// mm of printed material ("supports may overlap each other, never the model"). `0` =
    /// supports may touch but not overlap; `None` = unlimited (the plain two-zone rule).
    /// Bodies are unaffected: they keep clearance from anything foreign. A no-op for
    /// footprints without a separate body zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support_overlap_mm: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackedPlacement {
    pub x_mm: f64,
    pub y_mm: f64,
    pub angle_deg: f64,
}

impl PackedPlacement {
    /// `loops`, given in the footprint frame, in bed coordinates: rotated by this placement's
    /// angle, then moved to its position.
    pub fn place_loops(&self, loops: &[Vec<Point>]) -> Vec<Vec<Point>> {
        rotate_loops(loops, self.angle_deg)
            .into_iter()
            .map(|loop_| {
                loop_
                    .into_iter()
                    .map(|p| Point {
                        x: p.x + self.x_mm,
                        y: p.y + self.y_mm,
                    })
                    .collect()
            })
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackResult {
    pub placements: Vec<PackedPlacement>,
    pub resolution_mm: f64,
    pub requested_clearance_mm: f64,
    pub angle_step_deg: f64,
    pub attempts: usize,
    /// Number of deterministic sweep layouts evaluated (including the mandatory angle pass).
    pub attempts_run: usize,
    pub seed: u64,
}

pub(crate) fn check_pack_input(footprint: &Footprint, options: &PackOptions) -> Result<(), String> {
    let valid_rect = |r: &Rect| {
        [r.min_x, r.min_y, r.max_x, r.max_y]
            .iter()
            .all(|v| v.is_finite())
            && r.min_x < r.max_x
            && r.min_y < r.max_y
    };
    if !options.resolution.is_finite()
        || options.resolution <= 0.0
        || !options.clearance.is_finite()
        || options.clearance < 0.0
        || !options.angle_step_deg.is_finite()
        || options.angle_step_deg <= 0.0
        || options.angle_step_deg > 360.0
        || !valid_rect(&options.bed)
        || options.body_bed.is_some_and(|r| !valid_rect(&r))
        || options.exclusions.iter().any(|r| !valid_rect(r))
        || options.body_exclusions.iter().any(|r| !valid_rect(r))
        || options
            .support_overlap_mm
            .is_some_and(|x| !x.is_finite() || x < 0.0)
    {
        return Err(
            "invalid packing resolution, clearance, angle step, support overlap or bed/exclusion \
             bounds"
                .to_owned(),
        );
    }
    if footprint.loops.is_empty()
        || footprint
            .loops
            .iter()
            .chain(&footprint.body_loops)
            .chain(&footprint.support_landing_loops)
            .chain(footprint.bed_loops.iter().flatten())
            .chain(
                footprint
                    .layer_zones
                    .iter()
                    .flat_map(|zone| zone.support_loops.iter().chain(&zone.body_loops)),
            )
            .any(|l| {
                l.len() < 4
                    || l.first() != l.last()
                    || l.iter().any(|p| !p.x.is_finite() || !p.y.is_finite())
            })
    {
        return Err("footprint must contain finite, closed polygon rings".to_owned());
    }
    if footprint
        .loops
        .iter()
        .all(|l| polygon_area(l).abs() < 1e-12)
    {
        return Err("footprint has zero area".to_owned());
    }
    Ok(())
}

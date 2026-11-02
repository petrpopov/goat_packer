//! Independent re-checks of a finished layout (design 2026-11-02, §8.5 and §9.6). Neither
//! uses the search's no-fit maps, so a no-fit bug cannot vouch for itself:
//! - `validate_raster` re-rasterizes every placement at its own sub-pixel phase and tests
//!   every zone and every pair through the permission matrix; cheap enough for every
//!   shortlisted candidate.
//! - `validate_vector` (`validation_geometry`) measures the same rules exactly in vector
//!   space; run it on the published winner.
//!
//! `validate_result` keeps the legacy `Footprint` entry point and runs both.
//!
//! Frame convention: a layout's `ObjectPrint`s are one per rotation class, each already in
//! the plate-aligned frame of its rotation (the mesh turned about the frame centre; supports
//! are computed in that frame because they are not rotation-equivariant). A placement is a
//! pure translation of its class's frame origin.

mod raster;

pub use crate::validation_geometry::{VectorValidator, validate_vector};
pub use raster::{RasterValidator, validate_raster};

use crate::footprint::Footprint;
use crate::frame::Frame;
use crate::geometry::{Point, Rect};
use crate::keepout::{BodyRule, PlateRules};
use crate::kind_masks::{PlateZone, plate_zones};
use crate::object_print::ObjectPrint;
use crate::options::{PackOptions, PackResult, PackedPlacement, check_pack_input};
use crate::raster_mask::LegacyPrint;
use crate::validation_geometry::{angle_classes, validate_geometry};
use serde::{Deserialize, Serialize};

/// A placement that turns the object by more than this (degrees) is not at 0°.
const LOCKED_ANGLE_TOLERANCE_DEG: f64 = 1e-7;

/// One copy on the plate: the print of its rotation class and where that print's frame
/// origin goes, plate mm. A pure translation (see the module's frame convention).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct PrintPlacement {
    /// Index into the layout's prints (one per rotation class).
    pub print: usize,
    pub x_mm: f64,
    pub y_mm: f64,
}

impl PrintPlacement {
    pub fn origin(&self) -> Point {
        Point {
            x: self.x_mm,
            y: self.y_mm,
        }
    }
}

/// What a layout of `ObjectPrint`s is checked against.
#[derive(Clone, Debug)]
pub struct LayoutRules {
    /// Every plate zone in the plate frame: `plate_zones` of the job's rules plus the zones
    /// the search placed (the prime tower's body and brim, classes `TowerBody`/`TowerBrim`).
    /// It must hold the printable boundary, as `plate_zones` always does.
    pub zones: Vec<PlateZone>,
    pub body_rule: BodyRule,
    /// What `Gap::Clearance` means, mm.
    pub clearance_mm: f64,
    /// Support bases of two objects may overlap this deep (raster check only).
    pub support_overlap_mm: Option<f64>,
}

impl LayoutRules {
    /// The rules of `plate` with nothing placed yet; push the tower zones onto `zones`.
    /// `gcode_offset` is the project's `extruder_offset`, needed by G-code-frame keepouts.
    pub fn new(
        plate: &PlateRules,
        gcode_offset: Option<Point>,
        clearance_mm: f64,
        support_overlap_mm: Option<f64>,
    ) -> Result<LayoutRules, String> {
        let rules = LayoutRules {
            zones: plate_zones(plate, gcode_offset)?,
            body_rule: plate.body_rule,
            clearance_mm,
            support_overlap_mm,
        };
        rules.check_values()?;
        Ok(rules)
    }

    pub(crate) fn check_values(&self) -> Result<(), String> {
        let bad = |v: f64| !v.is_finite() || v < 0.0;
        if bad(self.clearance_mm) || self.support_overlap_mm.is_some_and(bad) {
            return Err("invalid clearance or support overlap".to_owned());
        }
        Ok(())
    }
}

/// Every point and part box of every print is finite: a NaN would silently pass every
/// distance test.
pub(crate) fn check_prints(prints: &[ObjectPrint]) -> Result<(), String> {
    let finite = |p: &Point| p.x.is_finite() && p.y.is_finite();
    let finite_box = |b: &Rect| {
        [b.min_x, b.min_y, b.max_x, b.max_y]
            .iter()
            .all(|v| v.is_finite())
    };
    for (index, print) in prints.iter().enumerate() {
        let flat = [
            &print.silhouette,
            &print.part_hulls,
            &print.landing,
            &print.brim,
            &print.support_base,
        ];
        let banded = print
            .body
            .iter()
            .chain(&print.support)
            .chain(&print.interface);
        let points_ok = flat
            .into_iter()
            .chain(banded)
            .flatten()
            .flatten()
            .all(finite);
        if !points_ok || !print.part_boxes.iter().all(finite_box) {
            return Err(format!("print {index} has non-finite coordinates"));
        }
    }
    Ok(())
}

/// Finite positions and known classes.
pub(crate) fn check_print_placements(
    placements: &[PrintPlacement],
    classes: usize,
) -> Result<(), String> {
    if let Some(index) = placements
        .iter()
        .position(|p| !p.x_mm.is_finite() || !p.y_mm.is_finite())
    {
        return Err(format!("placement {index} has non-finite coordinates"));
    }
    if let Some(index) = placements.iter().position(|p| p.print >= classes) {
        return Err(format!("placement {index} refers to an unknown print"));
    }
    Ok(())
}

/// Index pairs `(earlier, later)` whose boxes meet (touching counts), ordered by the later
/// index, then the earlier: the order of a sequential pair loop, so the first conflict found
/// does not depend on the thread count. A sweep over x keeps it near-linear.
pub(crate) fn meeting_pairs(boxes: &[Rect]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| boxes[a].min_x.total_cmp(&boxes[b].min_x).then(a.cmp(&b)));
    let mut pairs = Vec::new();
    for (position, &a) in order.iter().enumerate() {
        for &b in &order[position + 1..] {
            if boxes[b].min_x > boxes[a].max_x {
                break;
            }
            if boxes[b].min_y <= boxes[a].max_y && boxes[a].min_y <= boxes[b].max_y {
                pairs.push((a.min(b), a.max(b)));
            }
        }
    }
    pairs.sort_by_key(|&(earlier, later)| (later, earlier));
    pairs
}

/// The error a sequential loop would have met first: placement by placement, its zones
/// before its pairs with earlier placements. Each side is the first failure of its own
/// (ordered) parallel search, as `(index of the later placement, message)`.
pub(crate) fn first_failure(
    zone: Option<(usize, String)>,
    pair: Option<(usize, String)>,
) -> Result<(), String> {
    match (zone, pair) {
        (Some((zone_index, message)), Some((pair_index, _))) if zone_index <= pair_index => {
            Err(message)
        }
        (_, Some((_, message))) | (Some((_, message)), None) => Err(message),
        (None, None) => Ok(()),
    }
}

/// The legacy entry point: `result` (placements of `footprint`, any angle) checked by the
/// raster and then the vector check.
pub fn validate_result(
    footprint: &Footprint,
    options: &PackOptions,
    result: &PackResult,
) -> Result<(), String> {
    check_pack_input(footprint, options)?;
    check_placement_values(options, result)?;
    let frame = Frame::new(footprint, options.lock_orientation);
    let print = LegacyPrint::new(&frame.footprint, options)?;
    let canonical: Vec<PackedPlacement> = result
        .placements
        .iter()
        .map(|p| frame.to_canonical(p))
        .collect();
    let (angles, placements) = angle_classes(&canonical);
    RasterValidator::legacy(&print, angles, options.support_overlap_mm).check(&placements)?;
    validate_geometry(footprint, options, result)
}

/// Checks that need no geometry: object cap, finite numbers, locked orientation.
fn check_placement_values(options: &PackOptions, result: &PackResult) -> Result<(), String> {
    if options.max_objects > 0 && result.placements.len() > options.max_objects {
        return Err("layout exceeds max_objects".to_owned());
    }
    if result
        .placements
        .iter()
        .any(|p| !p.x_mm.is_finite() || !p.y_mm.is_finite() || !p.angle_deg.is_finite())
    {
        return Err("layout contains non-finite placement coordinates".to_owned());
    }
    if options.lock_orientation
        && result
            .placements
            .iter()
            .any(|p| p.angle_deg.rem_euclid(360.0).abs() > LOCKED_ANGLE_TOLERANCE_DEG)
    {
        return Err("layout violates locked orientation".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_pairs_are_every_overlapping_box_pair_in_loop_order() {
        let boxes = [
            Rect::new(0.0, 0.0, 2.0, 2.0),
            Rect::new(10.0, 0.0, 12.0, 2.0),
            Rect::new(2.0, 2.0, 4.0, 4.0),
            Rect::new(1.0, 5.0, 3.0, 6.0),
            Rect::new(11.0, 1.0, 13.0, 3.0),
            Rect::of_points(std::iter::empty()),
        ];
        let brute: Vec<(usize, usize)> = (0..boxes.len())
            .flat_map(|later| (0..later).map(move |earlier| (earlier, later)))
            .filter(|&(a, b)| {
                let (a, b) = (boxes[a], boxes[b]);
                a.min_x <= b.max_x && b.min_x <= a.max_x && a.min_y <= b.max_y && b.min_y <= a.max_y
            })
            .collect();
        assert_eq!(meeting_pairs(&boxes), brute);
        assert_eq!(brute, vec![(0, 2), (1, 4)]);
    }
}

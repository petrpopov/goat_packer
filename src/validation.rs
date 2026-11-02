//! An independent re-check of a finished layout: it re-rasterizes every placement instead of
//! reusing the search's no-fit maps, then checks original-coordinate vector geometry via
//! `validation_geometry`. Neither a no-fit bug nor a raster omission can vouch for itself.

use crate::collision::{support_cores_overlap, zones_collide};
use crate::footprint::Footprint;
use crate::frame::Frame;
use crate::geometry::Point;
use crate::kind_masks::{KindMasks, ZoneChecker};
use crate::options::{PackOptions, PackResult, PackedPlacement, check_pack_input};
use crate::raster_mask::LegacyPrint;

/// A placement this close to a pixel center (mm) is rasterized as is, without shifting.
const ON_GRID_TOLERANCE_MM: f64 = 1e-7;
/// A placement that turns the object by more than this (degrees) is not at 0°.
const LOCKED_ANGLE_TOLERANCE_DEG: f64 = 1e-7;

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
    // Keep the fractional phase of imported/manual placements. Rounding every centre
    // independently can turn a real overlap into two merely touching raster masks.
    let centers: Vec<(i32, i32)> = canonical
        .iter()
        .map(|p| {
            (
                ((p.x_mm - options.bed.min_x) / options.resolution).round() as i32,
                ((p.y_mm - options.bed.min_y) / options.resolution).round() as i32,
            )
        })
        .collect();
    let mut cache = MaskCache::default();
    let masks: Vec<usize> = canonical
        .iter()
        .zip(&centers)
        .map(|(p, &center)| cache.index_of(&print, options, p, center))
        .collect();
    let checkers: Vec<ZoneChecker> = cache
        .masks
        .iter()
        .map(|(_, mask)| ZoneChecker::new(mask, &print.zones, &print.grid))
        .collect();
    for (index, &mask) in masks.iter().enumerate() {
        let (x, y) = centers[index];
        if let Some(conflict) = checkers[mask].conflict(x, y) {
            return Err(format!("placement {index}: {conflict}"));
        }
        for (previous, &previous_mask) in masks.iter().enumerate().take(index) {
            check_pair(
                options,
                (previous, &cache.masks[previous_mask].1, centers[previous]),
                (index, &cache.masks[mask].1, centers[index]),
            )?;
        }
    }
    crate::validation_geometry::validate_geometry(footprint, options, result)
}

/// Masks per distinct (angle, sub-pixel offset): copies on the pixel grid share one.
#[derive(Default)]
struct MaskCache {
    masks: Vec<((u64, u64, u64), KindMasks)>,
}

impl MaskCache {
    /// The mask of one placement, whose rounded pixel center is `(x, y)`. A placement off the
    /// pixel grid is rasterized moved by the leftover offset, so imported or manual layouts
    /// keep their sub-pixel phase.
    fn index_of(
        &mut self,
        print: &LegacyPrint,
        options: &PackOptions,
        placement: &PackedPlacement,
        (x, y): (i32, i32),
    ) -> usize {
        let mut dx = placement.x_mm - options.bed.min_x - f64::from(x) * options.resolution;
        let mut dy = placement.y_mm - options.bed.min_y - f64::from(y) * options.resolution;
        if dx.abs() < ON_GRID_TOLERANCE_MM && dy.abs() < ON_GRID_TOLERANCE_MM {
            (dx, dy) = (0.0, 0.0);
        }
        let key = (placement.angle_deg.to_bits(), dx.to_bits(), dy.to_bits());
        if let Some(index) = self.masks.iter().position(|(known, _)| *known == key) {
            return index;
        }
        let mask = print.masks(placement.angle_deg, Point { x: dx, y: dy });
        self.masks.push((key, mask));
        self.masks.len() - 1
    }
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

/// Two placements (index, mask, pixel center) do not collide.
fn check_pair(
    options: &PackOptions,
    (previous, previous_mask, (previous_px, previous_py)): (usize, &KindMasks, (i32, i32)),
    (index, mask, (px, py)): (usize, &KindMasks, (i32, i32)),
) -> Result<(), String> {
    let (dx, dy) = (px - previous_px, py - previous_py);
    // Pair rules (`keepout::pair_rules`): silhouettes keep clearance, a support base keeps
    // its gap from a foreign landing, supports keep theirs from foreign bodies band by band,
    // supports may touch other supports: the same predicate the packing strategies enforce.
    if zones_collide(previous_mask, mask, dx, dy) {
        return Err(format!(
            "placements {previous} and {index} overlap: a body touches a foreign structure"
        ));
    }
    if support_cores_overlap(previous_mask, mask, dx, dy) {
        return Err(format!(
            "placements {previous} and {index}: supports overlap by more than {} mm",
            options.support_overlap_mm.unwrap_or_default()
        ));
    }
    Ok(())
}

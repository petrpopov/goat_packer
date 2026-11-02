//! An independent re-check of a finished layout: it re-rasterizes every placement instead of
//! reusing the search's no-fit maps, then checks original-coordinate vector geometry via
//! `validation_geometry`. Neither a no-fit bug nor a raster omission can vouch for itself.

use crate::collision::{bounds_hit_exclusion, support_cores_overlap, zones_collide};
use crate::footprint::Footprint;
use crate::frame::Frame;
use crate::geometry::Point;
use crate::options::{PackOptions, PackResult, PackedPlacement, check_pack_input};
use crate::raster_mask::{RasterMask, rasterize_footprint_with_support};

/// Slack when comparing placed bounds with the bed, mm.
const BOUNDS_SLACK_MM: f64 = 1e-6;
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
    let masks: Vec<RasterMask> = canonical
        .iter()
        .zip(&centers)
        .map(|(p, &center)| placement_mask(&frame.footprint, options, p, center))
        .collect();
    for (index, mask) in masks.iter().enumerate() {
        check_bed_and_exclusions(options, mask, centers[index], index)?;
        for (previous, previous_mask) in masks.iter().enumerate().take(index) {
            check_pair(
                options,
                (previous, previous_mask, centers[previous]),
                (index, mask, centers[index]),
            )?;
        }
    }
    crate::validation_geometry::validate_geometry(footprint, options, result)
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

/// The mask of one placement, whose rounded pixel center is `center`. A placement off the
/// pixel grid is rasterized from the footprint moved by the leftover offset, so imported or
/// manual layouts keep their sub-pixel phase.
fn placement_mask(
    footprint: &Footprint,
    options: &PackOptions,
    placement: &PackedPlacement,
    (x, y): (i32, i32),
) -> RasterMask {
    let dx = placement.x_mm - options.bed.min_x - f64::from(x) * options.resolution;
    let dy = placement.y_mm - options.bed.min_y - f64::from(y) * options.resolution;
    let rasterize = |footprint: &Footprint| {
        rasterize_footprint_with_support(
            footprint,
            placement.angle_deg,
            options.resolution,
            options.clearance,
            options.support_overlap_mm,
        )
    };
    if dx.abs() < ON_GRID_TOLERANCE_MM && dy.abs() < ON_GRID_TOLERANCE_MM {
        return rasterize(footprint);
    }
    let (s, c) = placement.angle_deg.to_radians().sin_cos();
    let offset = Point {
        x: dx * c + dy * s,
        y: -dx * s + dy * c,
    };
    let shift = |loops: &[Vec<Point>]| -> Vec<Vec<Point>> {
        loops
            .iter()
            .map(|l| {
                l.iter()
                    .map(|p| Point {
                        x: p.x + offset.x,
                        y: p.y + offset.y,
                    })
                    .collect()
            })
            .collect()
    };
    rasterize(&footprint.map_loops(shift))
}

/// One placement stays on the bed (and the body on the body bed) and clear of exclusions.
fn check_bed_and_exclusions(
    options: &PackOptions,
    mask: &RasterMask,
    (px, py): (i32, i32),
    index: usize,
) -> Result<(), String> {
    let x_mm = options.bed.min_x + f64::from(px) * options.resolution;
    let y_mm = options.bed.min_y + f64::from(py) * options.resolution;
    let translated_bounds = mask.bed_physical_bounds.translated(x_mm, y_mm);
    if translated_bounds.sticks_out_of(options.bed, BOUNDS_SLACK_MM) {
        return Err(format!(
            "placement {index} leaves bed: {translated_bounds:?}"
        ));
    }
    if let Some(body_bed) = options.body_bed {
        let body = mask.body_bounds.translated(x_mm, y_mm);
        if body.sticks_out_of(body_bed, BOUNDS_SLACK_MM) {
            return Err(format!(
                "placement {index} body leaves the body bed: {body:?}"
            ));
        }
    }
    // Exclusion feasibility follows the placement's bbox — the semantics BambuStudio's
    // own validation uses (see aabb_hits_exclusion).
    if bounds_hit_exclusion(
        mask.bed_physical_bounds,
        x_mm,
        y_mm,
        options.clearance,
        &options.exclusions,
    ) {
        return Err(format!("placement {index} intersects exclusion (bbox)"));
    }
    if bounds_hit_exclusion(
        mask.body_bounds,
        x_mm,
        y_mm,
        options.clearance,
        &options.body_exclusions,
    ) {
        return Err(format!(
            "placement {index} body intersects a body exclusion (bbox)"
        ));
    }
    Ok(())
}

/// Two placements (index, mask, pixel center) do not collide.
fn check_pair(
    options: &PackOptions,
    (previous, previous_mask, (previous_px, previous_py)): (usize, &RasterMask, (i32, i32)),
    (index, mask, (px, py)): (usize, &RasterMask, (i32, i32)),
) -> Result<(), String> {
    let (dx, dy) = (px - previous_px, py - previous_py);
    // Zone rule: a patch must not land on a foreign landing zone (body when the footprint has
    // none), silhouettes keep clearance, supports may touch other supports — the same
    // predicate the packing strategies enforce.
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

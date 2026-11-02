//! When placements conflict: two rasters overlapping under the zone rules, or one running into
//! the bed edge or an exclusion.

use crate::geometry::Rect;
use crate::options::PackOptions;
use crate::pixel_mask::{PixelMask, masks_overlap};
use crate::raster_mask::RasterMask;

/// The mask pairs whose overlap makes two placements collide (`b` shifted relative to `a`):
/// a full patch against the other's landing zone both ways, and silhouette against
/// silhouette (the patch need not contain the body's overhangs, so that pair is mandatory). Layer zones (`Footprint::layer_zones`) add support-vs-body pairs per
/// height band. Support cores (`support_overlap_mm`) are a separate pair, see
/// `support_cores_overlap`.
pub(crate) fn zone_pairs<'a>(
    a: &'a RasterMask,
    b: &'a RasterMask,
) -> Vec<(&'a PixelMask, &'a PixelMask)> {
    let mut pairs = vec![
        (&a.clearance, &b.landing_clearance),
        (&a.landing_clearance, &b.clearance),
    ];
    // Bodies may extend past the first-layer patch; neither patch/body pair implies this.
    pairs.push((&a.body_clearance, &b.body_clearance));
    // Same height band: supports of one against the body of the other, both ways.
    for ((support_a, body_a), (support_b, body_b)) in a.layer_zones.iter().zip(&b.layer_zones) {
        if support_a.pixel_count > 0 && body_b.pixel_count > 0 {
            pairs.push((support_a, body_b));
        }
        if body_a.pixel_count > 0 && support_b.pixel_count > 0 {
            pairs.push((body_a, support_b));
        }
    }
    pairs
}

/// `b` at pixel offset `(dx, dy)` from `a` breaks the zone rule (`zone_pairs`).
pub(crate) fn zones_collide(a: &RasterMask, b: &RasterMask, dx: i32, dy: i32) -> bool {
    zone_pairs(a, b)
        .into_iter()
        .any(|(za, zb)| masks_overlap(za, 0, 0, zb, dx, dy))
}

/// `b` at pixel offset `(dx, dy)` from `a` collides: the zone rule or the support cores.
pub fn placements_collide(a: &RasterMask, b: &RasterMask, dx: i32, dy: i32) -> bool {
    zones_collide(a, b, dx, dy) || support_cores_overlap(a, b, dx, dy)
}

/// Support-overlap pair: the support cores of two placements (shifted by `(dx, dy)` pixels
/// of `b` relative to `a`) overlap. Never true when either side carries no core.
pub(crate) fn support_cores_overlap(a: &RasterMask, b: &RasterMask, dx: i32, dy: i32) -> bool {
    match (&a.support_core, &b.support_core) {
        (Some(core_a), Some(core_b)) => masks_overlap(core_a, 0, 0, core_b, dx, dy),
        _ => false,
    }
}

pub fn placement_is_allowed(
    mask: &RasterMask,
    center_x_pixels: i32,
    center_y_pixels: i32,
    bed: Rect,
    exclusions: &[Rect],
) -> bool {
    let center_x = f64::from(center_x_pixels) * mask.resolution;
    let center_y = f64::from(center_y_pixels) * mask.resolution;
    let bounds = mask.bed_physical_bounds;
    if bounds
        .translated(center_x, center_y)
        .sticks_out_of(bed, 0.0)
    {
        return false;
    }
    !bounds_hit_exclusion(bounds, center_x, center_y, 0.0, exclusions)
        && !mask.bed_clearance.pixels().any(|(x, y)| {
            let wx = center_x + (f64::from(x) + 0.5) * mask.resolution;
            let wy = center_y + (f64::from(y) + 0.5) * mask.resolution;
            exclusions
                .iter()
                .any(|r| wx >= r.min_x && wx <= r.max_x && wy >= r.min_y && wy <= r.max_y)
        })
}

/// BambuStudio's plate validation rejects an object whose AXIS-ALIGNED bounding box comes
/// near a bed exclusion area — the silhouette is irrelevant there. A rotated goat can hold
/// its silhouette 6 mm off the exclusion corner while its bbox still covers the zone, and
/// the final slice then dies with "too close to exclusion area" after minutes of packing
/// (a real 17-object P1S magnet plate, twice). Exclusion feasibility therefore measures the
/// placement's rotated bbox — the patch ∪ silhouette extent, `bed_physical_bounds` — padded
/// by the same clearance the pixel-based check used. Bed bounds stay silhouette-accurate;
/// only exclusion checks take the bbox semantics. `x_px`/`y_px` are bed pixel coordinates
/// (origin at `bed.min`, the coordinate space of the packing loops).
pub(crate) fn aabb_hits_exclusion(
    options: &PackOptions,
    mask: &RasterMask,
    x_px: i32,
    y_px: i32,
) -> bool {
    let origin_x = options.bed.min_x + f64::from(x_px) * options.resolution;
    let origin_y = options.bed.min_y + f64::from(y_px) * options.resolution;
    bounds_hit_exclusion(
        mask.bed_physical_bounds,
        origin_x,
        origin_y,
        options.clearance,
        &options.exclusions,
    ) || bounds_hit_exclusion(
        mask.body_bounds,
        origin_x,
        origin_y,
        options.clearance,
        &options.body_exclusions,
    )
}

/// Float noise absorbed when turning bed distances into whole pixels: a mask that touches the
/// edge exactly (up to rounding of the raster bounds) still fits.
const ORIGIN_SLACK_PX: f64 = 1e-9;

/// Placement origins (pixels from `options.bed.min`) that keep a mask on the bed: the whole
/// printed extent inside `bed`, the body inside `body_bed` (see `PackOptions::body_bed`).
/// Inclusive `(min_x, min_y, max_x, max_y)`; empty when a min exceeds its max.
pub(crate) fn origin_range(options: &PackOptions, mask: &RasterMask) -> (i32, i32, i32, i32) {
    let r = options.resolution;
    let within = |bounds: Rect, area: Rect| {
        (
            ((area.min_x - options.bed.min_x - bounds.min_x) / r - ORIGIN_SLACK_PX).ceil() as i32,
            ((area.min_y - options.bed.min_y - bounds.min_y) / r - ORIGIN_SLACK_PX).ceil() as i32,
            ((area.max_x - options.bed.min_x - bounds.max_x) / r + ORIGIN_SLACK_PX).floor() as i32,
            ((area.max_y - options.bed.min_y - bounds.max_y) / r + ORIGIN_SLACK_PX).floor() as i32,
        )
    };
    let full = within(mask.bed_physical_bounds, options.bed);
    match options.body_bed {
        None => full,
        Some(area) => {
            let body = within(mask.body_bounds, area);
            (
                full.0.max(body.0),
                full.1.max(body.1),
                full.2.min(body.2),
                full.3.min(body.3),
            )
        }
    }
}

pub(crate) fn bounds_hit_exclusion(
    bounds: Rect,
    x: f64,
    y: f64,
    pad: f64,
    exclusions: &[Rect],
) -> bool {
    exclusions.iter().any(|r| {
        x + bounds.min_x - pad < r.max_x
            && x + bounds.max_x + pad > r.min_x
            && y + bounds.min_y - pad < r.max_y
            && y + bounds.max_y + pad > r.min_y
    })
}

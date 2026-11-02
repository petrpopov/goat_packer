//! The lattice baseline: copies on a regular grid, the pitch as tight as the zone rule allows.

use crate::collision::{aabb_hits_exclusion, origin_range, placements_collide};
use crate::footprint::Footprint;
use crate::frame::Frame;
use crate::layout::{PixelPlacement, cap_reached};
use crate::options::{PackOptions, PackResult, check_pack_input};
use crate::raster_mask::{RasterMask, rasterize_footprint_with_support};

/// Slack when testing a patch against the bed edge, mm: float noise of the raster bounds.
const BED_EDGE_SLACK_MM: f64 = 1e-7;

/// Deterministic rectangular-grid baseline in the same canonical frame as the search.
/// Uses two-zone pitches and checks every placed neighbour (including distant islands).
/// The physical extent must stay on the bed; clearance is only between foreign structures.
pub fn grid_layout(footprint: &Footprint, options: &PackOptions) -> PackResult {
    check_pack_input(footprint, options).expect("invalid packing input");
    let frame = Frame::new(footprint, options.lock_orientation);
    let mut result = grid_layout_inner(&frame.footprint, options);
    result.placements = result
        .placements
        .iter()
        .map(|p| frame.to_input(p))
        .collect();
    result
}

/// `grid_layout` in the canonical frame: the better of the 0° and 90° lattices.
pub(crate) fn grid_layout_inner(footprint: &Footprint, options: &PackOptions) -> PackResult {
    assert!(options.resolution > 0.0);
    let mut best: Vec<PixelPlacement> = Vec::new();
    let mut best_angle = 0.0f64;
    let orientations: &[f64] =
        if options.lock_orientation || (footprint.width - footprint.height).abs() < 1e-9 {
            &[0.0]
        } else {
            &[0.0, 90.0]
        };
    for &angle in orientations {
        let mask = rasterize_footprint_with_support(
            footprint,
            angle,
            options.resolution,
            options.clearance,
            options.support_overlap_mm,
        );
        let placed = fill_lattice(&mask, options);
        if placed.len() > best.len() {
            best = placed;
            best_angle = angle;
        }
    }
    PackResult {
        placements: best
            .iter()
            .map(|p| p.to_packed(best_angle, options))
            .collect(),
        resolution_mm: options.resolution,
        requested_clearance_mm: options.clearance,
        angle_step_deg: options.angle_step_deg,
        attempts: 1,
        attempts_run: 1,
        seed: options.seed,
    }
}

/// Copies of `mask` on the densest sheared lattice: rows `pitch_x` apart in x, each row shifted
/// by `shift` against the one below (shift 0 = the plain rectangular grid, tried first so a
/// tie keeps it). Shaped models nest: a U1 goat patch (47.6 x 43 mm) tiles a 270 mm bed with
/// 48 in brick rows shifted by half a pitch — a hand-made plate printed so — while the
/// rectangular grid fits 37 and the search 47.
fn fill_lattice(mask: &RasterMask, options: &PackOptions) -> Vec<PixelPlacement> {
    let pitch_x = row_pitch(mask);
    let mut best: Vec<PixelPlacement> = Vec::new();
    for shift in (0..pitch_x).step_by(SHIFT_STEP_PX) {
        let pitch_y = lattice_row_spacing(mask, pitch_x, shift);
        let placed = fill_rows(mask, options, pitch_x, pitch_y, shift);
        if placed.len() > best.len() {
            best = placed;
        }
    }
    best
}

/// Row shifts tried, in pixels: every other pixel (0.5 mm at the usual 0.25 mm resolution)
/// keeps the scan well under a second for a goat-sized patch.
const SHIFT_STEP_PX: usize = 2;

/// Rows checked above a row when spacing a sheared lattice: copies further up are far enough
/// for any patch that fits on a bed; `cell_is_free` still checks every placed copy.
const SPACING_ROWS_CHECKED: i32 = 3;

/// One lattice: rows `pitch_y` apart, row `r` starting `r * shift mod pitch_x` right of the
/// bed's first origin, cells that do not fit skipped.
fn fill_rows(
    mask: &RasterMask,
    options: &PackOptions,
    pitch_x: i32,
    pitch_y: i32,
    shift: i32,
) -> Vec<PixelPlacement> {
    let mut placed: Vec<PixelPlacement> = Vec::new();
    let (min_x, min_y, max_x, max_y) = origin_range(options, mask);
    let mut y = min_y;
    let mut row = 0;
    while y <= max_y {
        let mut x = min_x + (row * shift).rem_euclid(pitch_x);
        while x <= max_x {
            if cell_is_free(mask, options, &placed, x, y) {
                placed.push(PixelPlacement { mask: 0, x, y });
            }
            x += pitch_x;
            if cap_reached(placed.len(), options) {
                return placed;
            }
        }
        y += pitch_y;
        row += 1;
    }
    placed
}

/// The SMALLEST whole-pixel x shift where two copies of the mask satisfy the zone rules — not
/// the full bounding box.
fn row_pitch(mask: &RasterMask) -> i32 {
    (1..)
        .find(|&s| !placements_collide(mask, mask, s, 0))
        .unwrap_or(mask.bed_clearance.width as i32 + 1)
}

/// The smallest row spacing of a lattice with rows `pitch_x` apart and shifted by `shift`:
/// every copy of the next few rows clears the copy at the origin. Distant lattice neighbours
/// can still conflict for disconnected shapes; every cell is checked in `cell_is_free`.
fn lattice_row_spacing(mask: &RasterMask, pitch_x: i32, shift: i32) -> i32 {
    let reach = mask.bed_clearance.width as i32 / pitch_x + 1;
    let clears = |dy: i32| {
        (1..=SPACING_ROWS_CHECKED).all(|row| {
            let offset = (row * shift).rem_euclid(pitch_x);
            (-reach - 1..=reach).all(|k| !placements_collide(mask, mask, offset + k * pitch_x, row * dy))
        })
    };
    (1..).find(|&dy| clears(dy)).expect("rows clear once they no longer overlap")
}

/// A copy of `mask` at pixel `(x, y)` fits: on the bed, clear of exclusions and of every copy
/// already `placed`.
fn cell_is_free(
    mask: &RasterMask,
    options: &PackOptions,
    placed: &[PixelPlacement],
    x: i32,
    y: i32,
) -> bool {
    // Bed membership follows the patch+silhouette extent; the dilated mask may poke past the
    // bed edge, the physical model may not.
    let bounds = mask.bed_physical_bounds;
    let r = options.resolution;
    let on_bed = bounds.min_x + f64::from(x) * r >= -BED_EDGE_SLACK_MM
        && bounds.max_x + f64::from(x) * r
            <= options.bed.max_x - options.bed.min_x + BED_EDGE_SLACK_MM
        && bounds.min_y + f64::from(y) * r >= -BED_EDGE_SLACK_MM
        && bounds.max_y + f64::from(y) * r
            <= options.bed.max_y - options.bed.min_y + BED_EDGE_SLACK_MM;
    // Exclusions follow the placement's bbox (see aabb_hits_exclusion); bed bounds stay on the
    // patch+silhouette extent above.
    on_bed
        && !aabb_hits_exclusion(options, mask, x, y)
        && !placed
            .iter()
            .any(|p| placements_collide(mask, mask, x - p.x, y - p.y))
}

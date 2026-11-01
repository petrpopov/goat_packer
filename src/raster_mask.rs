//! A footprint rasterized at one rotation: the pixel masks the collision rules compare.

use crate::footprint::{Footprint, LAYER_ZONE_GAP_MM};
use crate::geometry::{Point, Rect, rotate_loops};
use crate::pixel_mask::PixelMask;
use crate::span_raster::SpanRaster;

#[derive(Clone, Debug)]
pub struct RasterMask {
    pub angle_deg: f64,
    pub resolution: f64,
    pub physical: PixelMask,
    pub clearance: PixelMask,
    /// The body subset of the patch, dilated by the same half-clearance as `clearance`.
    /// Two-zone packing rule: a body keeps clearance from anything foreign, while support
    /// structures may touch other supports. Equals `clearance` when the footprint carries no
    /// separate body loops (mesh projection, or a support-free first layer).
    pub body_clearance: PixelMask,
    /// `Footprint::support_landing_loops`, dilated like the other zones: a foreign full
    /// patch must not touch it. Equals `body_clearance` (and `landing_is_body` is set) when
    /// the footprint carries no landing loops.
    pub landing_clearance: PixelMask,
    pub landing_is_body: bool,
    /// Bed/exclusion variant of the patch (patch + model silhouette when overhangs extend
    /// past the first layer). Collisions between objects still use the patch above.
    pub bed_physical: PixelMask,
    pub bed_clearance: PixelMask,
    pub bed_physical_bounds: Rect,
    pub physical_bounds: Rect,
    /// Rotated bbox of the body loops (`bed_physical_bounds` when the footprint has no
    /// separate body): what `PackOptions::body_exclusions` are checked against.
    pub body_bounds: Rect,
    /// Support-overlap rule (`PackOptions::support_overlap_mm = Some(X)`): the printed patch
    /// eroded by X/2 per side. Two cores must not overlap, so printed supports of two objects
    /// overlap by at most X (X = 0: they may touch). `None` without the option, and when the
    /// footprint carries no support zone (body == full): the two-zone rule then already
    /// forbids any overlap of the clearance zones, which contain the cores.
    pub support_core: Option<PixelMask>,
    /// `Footprint::layer_zones` as (support, body) masks, each dilated by half of
    /// `LAYER_ZONE_GAP_MM`.
    pub layer_zones: Vec<(PixelMask, PixelMask)>,
}

pub fn rasterize_footprint(
    footprint: &Footprint,
    angle_deg: f64,
    resolution: f64,
    clearance: f64,
) -> RasterMask {
    rasterize_footprint_with_support(footprint, angle_deg, resolution, clearance, None)
}

/// Pixel radius of the support core: `ceil(-X / 2 / resolution)`, an erosion of the printed
/// patch by X/2 per side (negative = erosion). Rounding toward zero erodes sparingly, so the
/// raster never allows more than X of physical overlap; at X = 0 the core is the patch itself
/// (neighbouring supports may touch, not overlap). Clearance deliberately plays no part: the
/// user's tolerance is "supports may overlap each other, never the model", and the body keeps
/// its clearance through the two-zone pairs.
fn support_core_radius(support_overlap_mm: f64, resolution: f64) -> i32 {
    (-support_overlap_mm * 0.5 / resolution).ceil() as i32
}

/// The rasterization settings of one footprint at one angle.
struct Rasterizer {
    angle_deg: f64,
    resolution: f64,
    /// Dilation radius (pixels) for half of the requested clearance.
    clearance_radius: i32,
}

impl Rasterizer {
    /// Inclusive pixel ranges `(min_ix, max_ix, min_iy, max_iy)` covering `bounds`.
    fn pixel_range(&self, bounds: &Rect) -> (i32, i32, i32, i32) {
        let r = self.resolution;
        (
            (bounds.min_x / r - 0.5).floor() as i32,
            (bounds.max_x / r - 0.5).ceil() as i32,
            (bounds.min_y / r - 0.5).floor() as i32,
            (bounds.max_y / r - 0.5).ceil() as i32,
        )
    }

    /// The patch pixels. A patch thinner than a pixel rasterizes to nothing: it then falls
    /// back to its whole bounding box so it still collides with something.
    fn patch_pixels(&self, rotated: &[Vec<Point>], bounds: &Rect) -> SpanRaster {
        let pixels = SpanRaster::scanline(rotated, self.resolution, self.pixel_range(bounds));
        if !pixels.is_empty() {
            return pixels;
        }
        let r = self.resolution;
        let min_x = (bounds.min_x / r).floor() as i32;
        let max_x = (bounds.max_x / r).ceil() as i32;
        let min_y = (bounds.min_y / r).floor() as i32;
        let max_y = (bounds.max_y / r).ceil() as i32;
        SpanRaster {
            min_y,
            rows: vec![vec![(min_x, max_x.max(min_x + 1))]; (max_y - min_y).max(1) as usize],
        }
    }

    /// A sub-zone of the footprint (body, landing) dilated like the full patch, in the range
    /// of the patch; `None` when the footprint has no such loops or they rasterize to nothing.
    fn dilated_zone(
        &self,
        loops: &[Vec<Point>],
        range: (i32, i32, i32, i32),
    ) -> Option<SpanRaster> {
        if loops.is_empty() {
            return None;
        }
        let zone =
            SpanRaster::scanline(&rotate_loops(loops, self.angle_deg), self.resolution, range);
        (!zone.is_empty()).then(|| zone.dilate(self.clearance_radius))
    }

    /// Patch ∪ whole-model silhouette (`Footprint::bed_loops`) with its bounds; the patch
    /// itself when the footprint carries no bed loops.
    fn bed_variant(
        &self,
        footprint: &Footprint,
        patch: &SpanRaster,
        patch_bounds: Rect,
    ) -> (SpanRaster, Rect) {
        let Some(bed_loops) = &footprint.bed_loops else {
            return (patch.clone(), patch_bounds);
        };
        let rotated = rotate_loops(bed_loops, self.angle_deg);
        let bed_bounds = Rect::of_points(rotated.iter().flatten());
        let bed_only =
            SpanRaster::scanline(&rotated, self.resolution, self.pixel_range(&bed_bounds));
        (patch.union(&bed_only), bed_bounds.union(patch_bounds))
    }

    /// `Footprint::layer_zones` as (support, body) masks. Layer zones keep their own pixel
    /// range: trunks and interfaces may reach past the first-layer patch the other zones are
    /// sized by.
    fn layer_zone_masks(&self, footprint: &Footprint) -> Vec<(PixelMask, PixelMask)> {
        let layer_radius = (LAYER_ZONE_GAP_MM * 0.5 / self.resolution).ceil() as i32;
        let mask = |loops: &[Vec<Point>]| -> Option<PixelMask> {
            let rotated = rotate_loops(loops, self.angle_deg);
            let zone_bounds = Rect::of_points(rotated.iter().flatten());
            if !zone_bounds.min_x.is_finite() {
                return None;
            }
            let zone =
                SpanRaster::scanline(&rotated, self.resolution, self.pixel_range(&zone_bounds));
            (!zone.is_empty()).then(|| PixelMask::from_spans(&zone.dilate(layer_radius)))
        };
        footprint
            .layer_zones
            .iter()
            .filter_map(|zone| Some((mask(&zone.support_loops)?, mask(&zone.body_loops)?)))
            .collect()
    }

    /// The support core of `PackOptions::support_overlap_mm`: the patch eroded (or, for a
    /// negative erosion radius, dilated) by half the allowed overlap.
    fn support_core(&self, patch: &SpanRaster, overlap_mm: f64) -> PixelMask {
        assert!(overlap_mm.is_finite() && overlap_mm >= 0.0);
        let core_radius = support_core_radius(overlap_mm, self.resolution);
        let core = if core_radius >= 0 {
            patch.dilate(core_radius)
        } else {
            patch.erode(-core_radius)
        };
        PixelMask::from_spans(&core)
    }
}

/// `rasterize_footprint` plus the support core for `PackOptions::support_overlap_mm`.
pub fn rasterize_footprint_with_support(
    footprint: &Footprint,
    angle_deg: f64,
    resolution: f64,
    clearance: f64,
    support_overlap_mm: Option<f64>,
) -> RasterMask {
    assert!(resolution.is_finite() && resolution > 0.0);
    assert!(angle_deg.is_finite());
    assert!(clearance.is_finite() && clearance >= 0.0);
    let rasterizer = Rasterizer {
        angle_deg,
        resolution,
        clearance_radius: (clearance * 0.5 / resolution).ceil() as i32,
    };
    let rotated = rotate_loops(&footprint.loops, angle_deg);
    let bounds = Rect::of_points(rotated.iter().flatten());
    let range = rasterizer.pixel_range(&bounds);

    let physical_pixels = rasterizer.patch_pixels(&rotated, &bounds);
    let clearance_pixels = physical_pixels.dilate(rasterizer.clearance_radius);
    let body_clearance_pixels = rasterizer
        .dilated_zone(&footprint.body_loops, range)
        .unwrap_or_else(|| clearance_pixels.clone());
    let landing_clearance_pixels = rasterizer.dilated_zone(&footprint.support_landing_loops, range);
    let landing_is_body = landing_clearance_pixels.is_none();
    let landing_clearance_pixels =
        landing_clearance_pixels.unwrap_or_else(|| body_clearance_pixels.clone());

    let (bed_physical_pixels, bed_physical_bounds) =
        rasterizer.bed_variant(footprint, &physical_pixels, bounds);
    let bed_clearance_pixels = bed_physical_pixels.dilate(rasterizer.clearance_radius);
    let body_bounds = if footprint.body_loops.is_empty() {
        bed_physical_bounds
    } else {
        Rect::of_points(
            rotate_loops(&footprint.body_loops, angle_deg)
                .iter()
                .flatten(),
        )
    };

    let support_core = support_overlap_mm
        .filter(|_| body_clearance_pixels != clearance_pixels)
        .map(|overlap| rasterizer.support_core(&physical_pixels, overlap));

    RasterMask {
        angle_deg,
        resolution,
        physical: PixelMask::from_spans(&physical_pixels),
        clearance: PixelMask::from_spans(&clearance_pixels),
        body_clearance: PixelMask::from_spans(&body_clearance_pixels),
        landing_clearance: PixelMask::from_spans(&landing_clearance_pixels),
        landing_is_body,
        bed_physical: PixelMask::from_spans(&bed_physical_pixels),
        bed_clearance: PixelMask::from_spans(&bed_clearance_pixels),
        bed_physical_bounds,
        physical_bounds: bounds,
        body_bounds,
        support_core,
        layer_zones: rasterizer.layer_zone_masks(footprint),
    }
}

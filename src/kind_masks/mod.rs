//! Raster masks per kind and rotation class (design 2026-11-02, §8.1-§8.4): what the packing
//! core compares instead of whole footprints.
//!
//! Frame convention: an `ObjectPrint` is built for one rotation class and its loops are
//! already in the plate-aligned frame of that rotation (the mesh turned by `rotation_deg`
//! about the frame centre; supports are not rotation-equivariant, so they are computed in
//! that frame). Placing a print is therefore a pure translation: a mask origin is the frame
//! origin's position on the plate. The only rotation left is the legacy one
//! (`raster_mask`), for `Footprint`s whose kinds are rigid with the model.
//!
//! Pipeline: `KindShapes::new` grows every kind of a pair rule by half its gap as a vector
//! (`buffer`), `KindShapes::rasterize` covers the grown shapes conservatively, and
//! `origins` turns the plate zones of `PlateRules` into the origins a print may take.

mod buffer;
mod origins;
mod shapes;
mod zone_tests;
mod zones;

pub(crate) use buffer::join_sag_mm;
pub use origins::{
    FeasibleOrigins, OriginGrid, OriginSet, ZoneChecker, feasible_origins, forbidden_origins,
    zone_conflict,
};
pub use shapes::{KindShapes, MaskKey, MaskRules};
pub(crate) use zone_tests::zone_material;
pub use zones::{PlateZone, ZoneArea, plate_zones};

use crate::geometry::{Point, Rect};
use crate::object_print::{Loops, ObjectPrint, PrintKind};
use crate::pixel_mask::PixelMask;
use crate::span_raster::SpanRaster;

/// One print rasterized at one placement phase: the pair masks, the support core and what
/// the plate zones need. Built once per rotation class and shared by every copy.
#[derive(Clone, Debug)]
pub struct KindMasks {
    pub rotation_deg: f64,
    pub resolution: f64,
    pub rules: MaskRules,
    pub keys: Vec<MaskKey>,
    /// `masks[i]` is `keys[i]`'s kind grown by half its gap and covered.
    pub masks: Vec<PixelMask>,
    /// Index pairs into `masks`: this object's `.0` must not overlap another object's `.1`.
    pub slots: Vec<(usize, usize)>,
    /// `PackOptions::support_overlap_mm`: two cores must not overlap.
    pub support_core: Option<PixelMask>,
    /// Box around everything printed (`KindShapes::extent_bounds`), print frame, mm.
    pub extent_bounds: Rect,
    /// The vector print, for the plate zones (`origins`).
    pub print: ObjectPrint,
    pub bed_extent: Loops,
}

impl KindShapes {
    pub fn rasterize(&self, resolution: f64) -> KindMasks {
        assert!(resolution.is_finite() && resolution > 0.0);
        KindMasks {
            rotation_deg: self.print.rotation_deg,
            resolution,
            rules: self.rules,
            keys: self.keys.clone(),
            masks: self
                .grown
                .iter()
                .map(|loops| PixelMask::from_spans(&cover(loops, resolution)))
                .collect(),
            slots: self.slots.clone(),
            support_core: self
                .core
                .as_ref()
                .map(|core| PixelMask::from_spans(&cover(core, resolution))),
            extent_bounds: self.extent_bounds(),
            print: self.print.clone(),
            bed_extent: self.bed_extent.clone(),
        }
    }
}

impl KindMasks {
    /// The first mask of `kind` in `band` (0 for kinds without bands), whatever its gap.
    pub fn mask(&self, kind: PrintKind, band: usize) -> Option<&PixelMask> {
        self.keys
            .iter()
            .position(|key| key.kind == kind && key.band == band)
            .map(|index| &self.masks[index])
    }

    /// Size in pixels of the box around every mask: two copies shifted by at least this
    /// much along x or y never collide, whatever the pair.
    pub fn reach_px(&self) -> i32 {
        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        for mask in self.masks.iter().chain(&self.support_core) {
            if mask.pixel_count == 0 {
                continue;
            }
            let (x0, y0) = (mask.min_x, mask.min_y);
            let (x1, y1) = (x0 + mask.width as i32, y0 + mask.height as i32);
            bounds = Some(bounds.map_or((x0, y0, x1, y1), |(a, b, c, d)| {
                (a.min(x0), b.min(y0), c.max(x1), d.max(y1))
            }));
        }
        bounds.map_or(1, |(x0, y0, x1, y1)| (x1 - x0).max(y1 - y0))
    }
}

/// Conservative coverage of even-odd `loops` at `resolution`: every pixel a ring's area or
/// boundary reaches (`SpanRaster::cover`). Loops thinner than float noise still occupy their
/// bounding box, so no material can vanish from a mask.
pub(crate) fn cover(loops: &[Vec<Point>], resolution: f64) -> SpanRaster {
    if loops.iter().all(|ring| ring.len() < 2) {
        return SpanRaster::default();
    }
    let bounds = Rect::of_points(loops.iter().flatten());
    let r = resolution;
    let range = (
        (bounds.min_x / r - 0.5).floor() as i32,
        (bounds.max_x / r - 0.5).ceil() as i32,
        (bounds.min_y / r - 0.5).floor() as i32,
        (bounds.max_y / r - 0.5).ceil() as i32,
    );
    let pixels = SpanRaster::cover(loops, r, range);
    if !pixels.is_empty() {
        return pixels;
    }
    let min_x = (bounds.min_x / r).floor() as i32;
    let min_y = (bounds.min_y / r).floor() as i32;
    let max_x = ((bounds.max_x / r).ceil() as i32 - 1).max(min_x);
    let max_y = ((bounds.max_y / r).ceil() as i32 - 1).max(min_y);
    SpanRaster::rect(min_x, min_y, max_x, max_y)
}

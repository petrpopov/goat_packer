//! A layout in pixel units while it is being searched, and how two layouts are compared.

use crate::geometry::Rect;
use crate::options::{PackOptions, PackedPlacement};
use crate::raster_mask::RasterMask;

/// One placed mask: which of the search's masks, and its origin in bed pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PixelPlacement {
    pub(crate) mask: usize,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

impl PixelPlacement {
    /// The placement in millimetres on the bed, at `angle_deg`.
    pub(crate) fn to_packed(self, angle_deg: f64, options: &PackOptions) -> PackedPlacement {
        PackedPlacement {
            x_mm: options.bed.min_x + f64::from(self.x) * options.resolution,
            y_mm: options.bed.min_y + f64::from(self.y) * options.resolution,
            angle_deg,
        }
    }
}

/// More objects win; equal counts are decided by the smaller bounding box, and a tie keeps `a`.
pub(crate) fn better_layout(
    a: Vec<PixelPlacement>,
    b: Vec<PixelPlacement>,
    masks: &[RasterMask],
) -> Vec<PixelPlacement> {
    if b.len() > a.len() {
        return b;
    }
    if a.len() > b.len() {
        return a;
    }
    if layout_bbox_area(&b, masks) < layout_bbox_area(&a, masks) {
        b
    } else {
        a
    }
}

/// Area of the box around every placed patch, in pixel units (infinite for an empty layout).
pub(crate) fn layout_bbox_area(layout: &[PixelPlacement], masks: &[RasterMask]) -> f64 {
    if layout.is_empty() {
        return f64::INFINITY;
    }
    let mut bounds = Rect::new(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in layout {
        let mask = &masks[p.mask];
        bounds.min_x = bounds
            .min_x
            .min(f64::from(p.x) + mask.extent_bounds.min_x / mask.resolution);
        bounds.min_y = bounds
            .min_y
            .min(f64::from(p.y) + mask.extent_bounds.min_y / mask.resolution);
        bounds.max_x = bounds
            .max_x
            .max(f64::from(p.x) + mask.extent_bounds.max_x / mask.resolution);
        bounds.max_y = bounds
            .max_y
            .max(f64::from(p.y) + mask.extent_bounds.max_y / mask.resolution);
    }
    (bounds.max_x - bounds.min_x) * (bounds.max_y - bounds.min_y)
}

/// True when the optional per-plate object cap is set and reached (0 = unlimited).
pub(crate) fn cap_reached(placed: usize, options: &PackOptions) -> bool {
    options.max_objects > 0 && placed >= options.max_objects
}

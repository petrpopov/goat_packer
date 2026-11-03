//! A layout in pixel units while it is being searched: which rotation class each copy is and
//! where its frame origin sits on the run's origin grid.

use crate::geometry::Rect;
use crate::kind_masks::OriginGrid;

/// One placed copy: its rotation class (an index the run defines) and the pixel origin of
/// the class's frame. Placing a class is a pure translation (see `object_print::ObjectPrint`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PixelPlacement {
    pub(crate) class: usize,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

impl PixelPlacement {
    /// The origin's position on the plate, mm.
    pub(crate) fn mm(self, grid: &OriginGrid) -> (f64, f64) {
        (
            grid.origin.x + f64::from(self.x) * grid.resolution,
            grid.origin.y + f64::from(self.y) * grid.resolution,
        )
    }
}

/// A class's printed extent in whole pixels of its frame, `(min_x, min_y, max_x, max_y)`:
/// the box rounded outwards, so the compactness key compares integers only.
pub(crate) fn extent_px(extent: Rect, resolution: f64) -> (i64, i64, i64, i64) {
    let finite = [extent.min_x, extent.min_y, extent.max_x, extent.max_y]
        .iter()
        .all(|v| v.is_finite());
    if !finite {
        return (0, 0, 0, 0);
    }
    (
        (extent.min_x / resolution).floor() as i64,
        (extent.min_y / resolution).floor() as i64,
        (extent.max_x / resolution).ceil() as i64,
        (extent.max_y / resolution).ceil() as i64,
    )
}

/// Area in pixels of the box around every placed copy's extent (`extents[class]`); the
/// largest value for an empty layout, so it never wins on compactness.
pub(crate) fn bbox_area_px(layout: &[PixelPlacement], extents: &[(i64, i64, i64, i64)]) -> u64 {
    let boxes = layout.iter().map(|p| {
        let (x0, y0, x1, y1) = extents[p.class];
        let (x, y) = (i64::from(p.x), i64::from(p.y));
        (x + x0, y + y0, x + x1, y + y1)
    });
    let bounds = boxes.reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)));
    bounds.map_or(u64::MAX, |(x0, y0, x1, y1)| ((x1 - x0) * (y1 - y0)) as u64)
}

/// True when the optional per-plate object cap is set and reached (0 = unlimited).
pub(crate) fn cap_reached(placed: usize, max_objects: usize) -> bool {
    max_objects > 0 && placed >= max_objects
}

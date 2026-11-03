//! Where the improvement looks for room: a fixed list of rectangles of origins, in the order
//! the search visits them on a tie. A lattice clipped to the plate leaves its slack in the
//! corners, along the edges and around the tower, so the windows are the four corners, the
//! tower, then every edge in overlapping segments, each at two depths measured in copies.

use crate::layout::extent_px;
use crate::pack::context::Ctx;
use crate::span_raster::SpanRaster;

/// Window sizes in copies (the largest extent of a class): a shallow and a deep strip. The
/// deep one also lets the first interior row or column of the layout move.
const DEPTHS: [f64; 3] = [1.5, 2.5, 4.0];
/// Length of an edge segment in copies and the step between segments.
const SEGMENT: f64 = 3.0;
const SEGMENT_STEP: f64 = 1.0;
/// The tower window reaches this many copies beyond the tower's origin set.
const TOWER_MARGIN: f64 = 1.5;

/// An inclusive rectangle of origins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Window {
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) x1: i32,
    pub(crate) y1: i32,
}

impl Window {
    pub(crate) fn contains(&self, x: i32, y: i32) -> bool {
        (self.x0..=self.x1).contains(&x) && (self.y0..=self.y1).contains(&y)
    }
}

/// The windows of a layout at tower `anchor`, in visiting order.
pub(super) fn window_list(ctx: &Ctx, anchor: Option<usize>) -> Vec<Window> {
    let Some(bounds) = origin_bounds(ctx) else {
        return Vec::new();
    };
    let (ex, ey) = copy_size(ctx);
    let px = |copies: f64, size: i32| (copies * f64::from(size)).ceil() as i32;
    let mut windows = Vec::new();
    for depth in DEPTHS {
        let (w, h) = (px(depth, ex), px(depth, ey));
        for corner in 0..4 {
            windows.push(corner_window(bounds, corner, w, h));
        }
    }
    if let Some(a) = anchor {
        windows.extend(tower_windows(ctx, a, ex, ey));
    }
    for depth in DEPTHS {
        windows.extend(edge_windows(bounds, ex, ey, depth));
    }
    windows.push(Window {
        x0: bounds.0,
        y0: bounds.1,
        x1: bounds.2,
        y1: bounds.3,
    });
    windows
}

/// Bounding box of the origins any class may take, `(min_x, min_y, max_x, max_y)`.
fn origin_bounds(ctx: &Ctx) -> Option<(i32, i32, i32, i32)> {
    (0..ctx.classes())
        .filter_map(|c| ctx.origins(c).bounds())
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
}

/// The largest width and height of a copy's printed extent over the classes, pixels.
fn copy_size(ctx: &Ctx) -> (i32, i32) {
    let sizes = (0..ctx.classes()).map(|c| {
        let b = extent_px(ctx.masks(c).extent_bounds, ctx.grid().resolution);
        ((b.2 - b.0) as i32, (b.3 - b.1) as i32)
    });
    sizes.fold((1, 1), |a, b| (a.0.max(b.0), a.1.max(b.1)))
}

/// Corner `c` (bit 0 = right, bit 1 = top) of the bounds, `w` x `h` large.
fn corner_window(b: (i32, i32, i32, i32), c: usize, w: i32, h: i32) -> Window {
    let (x0, x1) = if c & 1 == 0 {
        (b.0, b.0 + w)
    } else {
        (b.2 - w, b.2)
    };
    let (y0, y1) = if c & 2 == 0 {
        (b.1, b.1 + h)
    } else {
        (b.3 - h, b.3)
    };
    Window { x0, y0, x1, y1 }
}

/// Segments along the four edges, each `depth` copies deep.
fn edge_windows(b: (i32, i32, i32, i32), ex: i32, ey: i32, depth: f64) -> Vec<Window> {
    let mut windows = Vec::new();
    let (len_x, step_x) = (
        (SEGMENT * f64::from(ex)) as i32,
        ((SEGMENT_STEP * f64::from(ex)) as i32).max(1),
    );
    let (len_y, step_y) = (
        (SEGMENT * f64::from(ey)) as i32,
        ((SEGMENT_STEP * f64::from(ey)) as i32).max(1),
    );
    let (deep_x, deep_y) = (
        (depth * f64::from(ex)).ceil() as i32,
        (depth * f64::from(ey)).ceil() as i32,
    );
    for x in (b.0..=b.2).step_by(step_x as usize) {
        let x1 = (x + len_x).min(b.2);
        windows.push(Window {
            x0: x,
            y0: b.1,
            x1,
            y1: b.1 + deep_y,
        });
        windows.push(Window {
            x0: x,
            y0: b.3 - deep_y,
            x1,
            y1: b.3,
        });
    }
    for y in (b.1..=b.3).step_by(step_y as usize) {
        let y1 = (y + len_y).min(b.3);
        windows.push(Window {
            x0: b.0,
            y0: y,
            x1: b.0 + deep_x,
            y1,
        });
        windows.push(Window {
            x0: b.2 - deep_x,
            y0: y,
            x1: b.2,
            y1,
        });
    }
    windows
}

/// The tower's surroundings: the origins it forbids, grown by `TOWER_MARGIN` copies; and the
/// same grown twice as far.
fn tower_windows(ctx: &Ctx, anchor: usize, ex: i32, ey: i32) -> Vec<Window> {
    let layer = &ctx.towers()[anchor];
    let boxes = (0..ctx.classes()).filter_map(|c| spans_bounds(&layer.forbidden(c).spans));
    let Some(b) = boxes.reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
    else {
        return Vec::new();
    };
    DEPTHS
        .iter()
        .map(|&depth| {
            let scale = TOWER_MARGIN * depth / DEPTHS[0];
            let (mx, my) = (
                (scale * f64::from(ex)) as i32,
                (scale * f64::from(ey)) as i32,
            );
            Window {
                x0: b.0 - mx,
                y0: b.1 - my,
                x1: b.2 + mx,
                y1: b.3 + my,
            }
        })
        .collect()
}

/// Inclusive bounding box of the set pixels.
fn spans_bounds(spans: &SpanRaster) -> Option<(i32, i32, i32, i32)> {
    let mut found: Option<(i32, i32, i32, i32)> = None;
    for (index, row) in spans.rows.iter().enumerate() {
        let (Some(first), Some(last)) = (row.first(), row.last()) else {
            continue;
        };
        let y = spans.min_y + index as i32;
        found = Some(match found {
            None => (first.0, y, last.1 - 1, y),
            Some(f) => (f.0.min(first.0), f.1, f.2.max(last.1 - 1), y),
        });
    }
    found
}

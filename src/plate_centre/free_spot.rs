//! The nearest free place for a prime tower between the packed objects.

use super::ensure_relocated_tower_clear;
use crate::geometry::{Point, Rect};

/// Grid step of the free-spot search and of its occupancy raster.
const FREE_SPOT_STEP_MM: f64 = 0.5;

/// How many of the nearest raster-free spots get the exact check before giving up: the raster
/// is conservative, so the first one almost always passes.
const FREE_SPOT_EXACT_CHECKS: usize = 64;

/// Float slack when testing a spot against the search area, mm.
const AREA_SLACK_MM: f64 = 1e-9;

/// The free place nearest to `current` (by min corner, which `wipe_tower_x/y` anchors) for a
/// tower keepout of `size` inside `area`: it must keep `clearance` from every object's loops
/// (the full patch, supports included) and must not overlap a fixed keepout. `None` when no
/// such place exists.
///
/// The objects are rasterized at `FREE_SPOT_STEP_MM` and dilated by the clearance plus one
/// pixel (pixel-centre sampling misses up to half a diagonal), a summed-area table finds every
/// empty window, and the nearest windows are confirmed against the exact loops.
pub fn free_tower_spot(
    current: Rect,
    size: (f64, f64),
    area: Rect,
    objects: &[Vec<Vec<Point>>],
    fixed_keepouts: &[Rect],
    clearance: f64,
) -> Option<Rect> {
    let grid = Grid::new(area, size)?;
    let mut blocked = grid.rasterize(objects);
    grid.dilate(
        &mut blocked,
        (clearance / FREE_SPOT_STEP_MM).ceil() as usize + 1,
    );
    grid.block_keepouts(&mut blocked, fixed_keepouts);
    let mut spots = grid.empty_windows(&blocked, size, current);
    spots.sort_by(|a, b| a.0.total_cmp(&b.0));
    spots
        .into_iter()
        .take(FREE_SPOT_EXACT_CHECKS)
        .map(|(_, spot)| spot)
        .find(|spot| {
            ensure_relocated_tower_clear(*spot, fixed_keepouts).is_ok()
                && !objects
                    .iter()
                    .any(|loops| crate::loops_touch_rect(loops, spot.grown(clearance)))
        })
}

/// The search area as a raster of `FREE_SPOT_STEP_MM` pixels, row-major.
struct Grid {
    area: Rect,
    columns: usize,
    rows: usize,
    /// The tower's size in pixels.
    span_x: usize,
    span_y: usize,
}

impl Grid {
    /// `None` when the tower does not fit the area at all.
    fn new(area: Rect, size: (f64, f64)) -> Option<Self> {
        let step = FREE_SPOT_STEP_MM;
        let columns = ((area.max_x - area.min_x) / step).floor() as usize;
        let rows = ((area.max_y - area.min_y) / step).floor() as usize;
        let (span_x, span_y) = (
            (size.0 / step).ceil() as usize,
            (size.1 / step).ceil() as usize,
        );
        (columns >= span_x && rows >= span_y).then_some(Self {
            area,
            columns,
            rows,
            span_x,
            span_y,
        })
    }

    /// Pixels whose centre lies inside any object (even-odd within one object's loops).
    fn rasterize(&self, objects: &[Vec<Vec<Point>>]) -> Vec<bool> {
        let step = FREE_SPOT_STEP_MM;
        let (area, columns, rows) = (self.area, self.columns, self.rows);
        let mut occupied = vec![false; columns * rows];
        for loops in objects {
            for row in 0..rows {
                let y = area.min_y + (row as f64 + 0.5) * step;
                let mut crossings: Vec<f64> = loops
                    .iter()
                    .flat_map(|loop_| loop_.windows(2))
                    .filter(|edge| (edge[0].y > y) != (edge[1].y > y))
                    .map(|edge| {
                        let (a, b) = (edge[0], edge[1]);
                        a.x + (b.x - a.x) * (y - a.y) / (b.y - a.y)
                    })
                    .collect();
                crossings.sort_by(f64::total_cmp);
                for pair in crossings.chunks_exact(2) {
                    let first = ((pair[0] - area.min_x) / step - 0.5).ceil().max(0.0) as usize;
                    let last = ((pair[1] - area.min_x) / step - 0.5).floor();
                    if last < 0.0 {
                        continue;
                    }
                    for column in first..=(last as usize).min(columns - 1) {
                        occupied[row * columns + column] = true;
                    }
                }
            }
        }
        occupied
    }

    /// Grows every occupied pixel into a square of `radius` pixels around it.
    fn dilate(&self, occupied: &mut Vec<bool>, radius: usize) {
        let (columns, rows) = (self.columns, self.rows);
        let original = occupied.clone();
        for row in 0..rows {
            for column in 0..columns {
                if !original[row * columns + column] {
                    continue;
                }
                for r in row.saturating_sub(radius)..(row + radius + 1).min(rows) {
                    for c in column.saturating_sub(radius)..(column + radius + 1).min(columns) {
                        occupied[r * columns + c] = true;
                    }
                }
            }
        }
    }

    /// Fixed keepouts block every pixel they touch (edge contact is not an overlap).
    fn block_keepouts(&self, blocked: &mut [bool], fixed_keepouts: &[Rect]) {
        let step = FREE_SPOT_STEP_MM;
        let (area, columns, rows) = (self.area, self.columns, self.rows);
        for rect in fixed_keepouts {
            let c0 = ((rect.min_x - area.min_x) / step).floor().max(0.0) as usize;
            let c1 = (((rect.max_x - area.min_x) / step).ceil().max(0.0) as usize).min(columns);
            let r0 = ((rect.min_y - area.min_y) / step).floor().max(0.0) as usize;
            let r1 = (((rect.max_y - area.min_y) / step).ceil().max(0.0) as usize).min(rows);
            for r in r0..r1 {
                for c in c0..c1 {
                    blocked[r * columns + c] = true;
                }
            }
        }
    }

    /// Every window of the tower's size without a blocked pixel (found through a summed-area
    /// table), with its distance from `current` by min corner.
    fn empty_windows(&self, blocked: &[bool], size: (f64, f64), current: Rect) -> Vec<(f64, Rect)> {
        let step = FREE_SPOT_STEP_MM;
        let (area, columns, rows) = (self.area, self.columns, self.rows);
        let (span_x, span_y) = (self.span_x, self.span_y);
        let width = columns + 1;
        let mut sums = vec![0u32; width * (rows + 1)];
        for row in 0..rows {
            for column in 0..columns {
                sums[(row + 1) * width + column + 1] = u32::from(blocked[row * columns + column])
                    + sums[row * width + column + 1]
                    + sums[(row + 1) * width + column]
                    - sums[row * width + column];
            }
        }
        let empty = |row: usize, column: usize| {
            let (r1, c1) = (row + span_y, column + span_x);
            sums[r1 * width + c1] + sums[row * width + column]
                == sums[row * width + c1] + sums[r1 * width + column]
        };
        let mut spots = Vec::new();
        for row in 0..=rows - span_y {
            for column in 0..=columns - span_x {
                if !empty(row, column) {
                    continue;
                }
                let (x, y) = (
                    area.min_x + column as f64 * step,
                    area.min_y + row as f64 * step,
                );
                let spot = Rect::new(x, y, x + size.0, y + size.1);
                if spot.max_x > area.max_x + AREA_SLACK_MM
                    || spot.max_y > area.max_y + AREA_SLACK_MM
                {
                    continue;
                }
                spots.push(((x - current.min_x).hypot(y - current.min_y), spot));
            }
        }
        spots
    }
}

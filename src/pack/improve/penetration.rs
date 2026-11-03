//! Penetration depth of one class pair on the raster (after Umetani and Murakami 2022): at
//! every shift inside the no-fit set, the shortest axis-parallel move that leaves it. Zero
//! outside the set, so "depth > 0" is exactly "the pair rule breaks" and the sum of depths
//! is a measure of how overlapped a layout is, which the guided search drives to zero.

use crate::pack::context::Ctx;
use crate::span_raster::SpanRaster;

/// Depth map of one ordered class pair over the bounding box of its no-fit set.
pub(super) struct Depth {
    x0: i32,
    y0: i32,
    w: usize,
    h: usize,
    cells: Vec<u16>,
}

impl Depth {
    fn new(no_fit: &SpanRaster) -> Depth {
        let spans = || no_fit.rows.iter().flatten();
        let (Some(x0), Some(x1)) = (spans().map(|s| s.0).min(), spans().map(|s| s.1).max()) else {
            return Depth {
                x0: 0,
                y0: 0,
                w: 0,
                h: 0,
                cells: Vec::new(),
            };
        };
        let (w, h) = ((x1 - x0) as usize, no_fit.rows.len());
        let mut inside = vec![false; w * h];
        for (y, row) in no_fit.rows.iter().enumerate() {
            for &(a, b) in row {
                inside[y * w + (a - x0) as usize..y * w + (b - x0) as usize].fill(true);
            }
        }
        let mut cells = vec![u16::MAX; w * h];
        // Four directional run lengths; the depth is the shortest.
        for y in 0..h {
            run_min(&inside, &mut cells, (0..w).map(|x| y * w + x));
            run_min(&inside, &mut cells, (0..w).rev().map(|x| y * w + x));
        }
        for x in 0..w {
            run_min(&inside, &mut cells, (0..h).map(|y| y * w + x));
            run_min(&inside, &mut cells, (0..h).rev().map(|y| y * w + x));
        }
        Depth {
            x0,
            y0: no_fit.min_y,
            w,
            h,
            cells,
        }
    }

    /// Depth at shift `(dx, dy)` of the second copy relative to the first.
    #[inline]
    pub(super) fn at(&self, dx: i32, dy: i32) -> u32 {
        let (x, y) = (dx - self.x0, dy - self.y0);
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return 0;
        }
        u32::from(self.cells[y as usize * self.w + x as usize])
    }

    /// Largest `|dx|` or `|dy|` at which the depth can be nonzero.
    fn reach(&self) -> i32 {
        if self.w == 0 {
            return 0;
        }
        [
            self.x0,
            self.x0 + self.w as i32,
            self.y0,
            self.y0 + self.h as i32,
        ]
        .iter()
        .map(|v| v.abs())
        .max()
        .unwrap_or(0)
    }
}

/// Walks `order` and lowers each cell to the length of the run of set cells that ends there.
fn run_min(inside: &[bool], cells: &mut [u16], order: impl Iterator<Item = usize>) {
    let mut run = 0u16;
    for index in order {
        run = if inside[index] {
            run.saturating_add(1)
        } else {
            0
        };
        cells[index] = cells[index].min(run);
    }
}

/// The depth map of every ordered class pair of a run.
pub(super) struct DepthTable {
    classes: usize,
    maps: Vec<Depth>,
    reach: i32,
}

impl DepthTable {
    pub(super) fn new(ctx: &Ctx) -> DepthTable {
        ctx.prepare_no_fit();
        DepthTable::of_sets(ctx.classes(), |a, b| ctx.no_fit(a, b).clone())
    }

    /// The table of `set(a, b)`, what a copy of class `b` may not take relative to one of
    /// class `a`: the no-fit sets of the run, or those grown by a margin.
    pub(super) fn of_sets(classes: usize, set: impl Fn(usize, usize) -> SpanRaster) -> DepthTable {
        let maps: Vec<Depth> = (0..classes * classes)
            .map(|i| Depth::new(&set(i / classes, i % classes)))
            .collect();
        let reach = maps.iter().map(Depth::reach).max().unwrap_or(0);
        DepthTable {
            classes,
            maps,
            reach,
        }
    }

    pub(super) fn classes(&self) -> usize {
        self.classes
    }

    /// Depth of a copy of class `b` at shift `(dx, dy)` from one of class `a`.
    #[inline]
    pub(super) fn at(&self, a: usize, b: usize, dx: i32, dy: i32) -> u32 {
        self.maps[a * self.classes + b].at(dx, dy)
    }

    /// Two copies farther apart than this on an axis never overlap.
    pub(super) fn reach(&self) -> i32 {
        self.reach
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_is_the_shortest_way_out_along_an_axis() {
        // A 5 wide, 3 tall block: the centre column leaves in 2 steps up or down.
        let block = SpanRaster::rect(-2, -1, 2, 1);
        let depth = Depth::new(&block);
        assert_eq!(depth.at(0, 0), 2);
        assert_eq!(depth.at(-2, 0), 1);
        assert_eq!(depth.at(0, 1), 1);
        assert_eq!(depth.at(3, 0), 0);
        assert_eq!(depth.at(0, -2), 0);
    }
}

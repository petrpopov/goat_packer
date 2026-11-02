//! A uniform grid over the edges of one area, so a distance query only visits the edges near
//! it. Built once per area (per rotation class), queried at any translation.

use crate::geometry::{Point, Rect};

/// Grid cells are never finer than this (mm): about the gaps the rules ask for, so a query
/// box covers a few cells.
const MIN_CELL_MM: f64 = 0.5;
/// Cells per side at most, so a plate-sized zone does not get a huge grid.
const MAX_CELLS_PER_SIDE: f64 = 128.0;

/// Edges bucketed by the cells their bounding boxes cover (CSR layout).
pub(super) struct EdgeGrid {
    origin: Point,
    cell: f64,
    columns: usize,
    rows: usize,
    /// Edge indices of cell `c` (row-major): `items[starts[c]..starts[c + 1]]`.
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl EdgeGrid {
    pub(super) fn new(edges: &[[Point; 2]], bounds: Rect) -> EdgeGrid {
        if edges.is_empty() {
            return EdgeGrid {
                origin: Point { x: 0.0, y: 0.0 },
                cell: 1.0,
                columns: 0,
                rows: 0,
                starts: vec![0],
                items: vec![],
            };
        }
        let (width, height) = (bounds.max_x - bounds.min_x, bounds.max_y - bounds.min_y);
        let cell = (width.max(height) / MAX_CELLS_PER_SIDE).max(MIN_CELL_MM);
        let mut grid = EdgeGrid {
            origin: Point {
                x: bounds.min_x,
                y: bounds.min_y,
            },
            cell,
            columns: (width / cell) as usize + 1,
            rows: (height / cell) as usize + 1,
            starts: vec![],
            items: vec![],
        };
        let cells = |edge: &[Point; 2]| grid.cells(Rect::of_points(edge.iter()));
        let mut counts = vec![0u32; grid.columns * grid.rows + 1];
        for range in edges.iter().filter_map(cells) {
            for_each_cell(range, grid.columns, |c| counts[c + 1] += 1);
        }
        for c in 1..counts.len() {
            counts[c] += counts[c - 1];
        }
        let mut fill = counts.clone();
        let mut items = vec![0u32; counts[counts.len() - 1] as usize];
        for (index, range) in edges
            .iter()
            .enumerate()
            .filter_map(|(i, e)| Some((i, cells(e)?)))
        {
            for_each_cell(range, grid.columns, |c| {
                items[fill[c] as usize] = index as u32;
                fill[c] += 1;
            });
        }
        grid.starts = counts;
        grid.items = items;
        grid
    }

    /// Inclusive cell range `(x0, y0, x1, y1)` that box `b` covers, clamped to the grid;
    /// `None` when the box misses it.
    fn cells(&self, b: Rect) -> Option<(usize, usize, usize, usize)> {
        let index = |v: f64, origin: f64| ((v - origin) / self.cell).floor();
        let (x0, x1) = (index(b.min_x, self.origin.x), index(b.max_x, self.origin.x));
        let (y0, y1) = (index(b.min_y, self.origin.y), index(b.max_y, self.origin.y));
        let (columns, rows) = (self.columns as f64, self.rows as f64);
        if x1 < 0.0 || y1 < 0.0 || x0 >= columns || y0 >= rows || x0 > x1 || y0 > y1 {
            return None;
        }
        Some((
            x0.max(0.0) as usize,
            y0.max(0.0) as usize,
            x1.min(columns - 1.0) as usize,
            y1.min(rows - 1.0) as usize,
        ))
    }

    /// Whether `test` holds for an edge listed in a cell that `query` covers. Edges spanning
    /// several cells may be tested more than once.
    pub(super) fn any_near(
        &self,
        edges: &[[Point; 2]],
        query: Rect,
        mut test: impl FnMut(&[Point; 2]) -> bool,
    ) -> bool {
        let Some((x0, y0, x1, y1)) = self.cells(query) else {
            return false;
        };
        for y in y0..=y1 {
            let row = y * self.columns;
            let (from, to) = (self.starts[row + x0], self.starts[row + x1 + 1]);
            if self.items[from as usize..to as usize]
                .iter()
                .any(|&edge| test(&edges[edge as usize]))
            {
                return true;
            }
        }
        false
    }
}

fn for_each_cell(
    (x0, y0, x1, y1): (usize, usize, usize, usize),
    columns: usize,
    mut f: impl FnMut(usize),
) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            f(y * columns + x);
        }
    }
}

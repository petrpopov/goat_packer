//! The union of the ensemble's drawings, drawn once (`ensemble::run_union`). Every consumer of
//! an ensemble reads only the union of its realizations (`envelope`, holes filled), and
//! drawing each distinct graph on its own was most of the model's cost: the 180 graphs of a
//! P1S goat under Bambu 02.09 spent 200 of 330 CPU s per rotation uniting, trimming and
//! closing their own layers. Here the raw slices of every graph are filled into one bitmap per
//! layer (an OR, whatever the number of slices: a polygon union of all of them took 17 GB),
//! and each layer is turned back into rings and finished (`draw::finish_layer`) once.
//!
//! The bitmap samples cell centres, with one cell kept in every row a slice crosses (plain
//! centre sampling lost thin slivers of tilted pieces, and the closing grew whole notches
//! around them: misses of 1 mm), so the union matches the members' up to a cell (`CELL_MM`
//! 0.05 mm, against model margins of 0.5-2.3 mm).
//! The closing over notches narrower than a line now also fills notches between slices of
//! different graphs; both only grow the union.

use std::sync::Mutex;

use super::bands::member_base;
use super::draw::{NOTCH_CLOSING_MM, trim_layer};
use super::stage_a::StageA;
use crate::polygon_ops::{FillRule, IntPoint, Ring, Rings, bbox, resolve, simplify, union};

/// Cell size of the layer bitmaps, mm.
const CELL_MM: f64 = 0.05;

/// How far beyond the object's outline (over all layers) the bitmaps reach, mm. A slice
/// outside makes `UnionSlices::finish` return `None` (the caller then draws member by member).
const REACH_MM: f64 = 40.0;

/// One bitmap per layer over a fixed window of the object frame.
pub struct UnionSlices {
    origin: IntPoint,
    cell: i64,
    width: usize,
    height: usize,
    words: usize,
    layers: Vec<Mutex<Vec<u64>>>,
    /// The union of the members' own bases (`bands::member_base`).
    bases: Mutex<Rings>,
    /// The cells inside the object per layer (`collision`).
    collisions: Vec<Mutex<Option<std::sync::Arc<Rows>>>>,
    /// The closing radius in cells.
    closing: usize,
    outside: Mutex<bool>,
}

impl UnionSlices {
    pub fn new(stage: &StageA) -> UnionSlices {
        let grid = stage.derived.grid;
        let cell = grid.length(CELL_MM).max(1);
        let reach = grid.length(REACH_MM);
        let outline: Rings = stage.sections.outline.iter().flatten().cloned().collect();
        let (min, max) = bbox(&outline).map_or(((0, 0), (0, 0)), |found| {
            (
                (i64::from(found.min.x), i64::from(found.min.y)),
                (i64::from(found.max.x), i64::from(found.max.y)),
            )
        });
        let origin = IntPoint::new((min.0 - reach) as i32, (min.1 - reach) as i32);
        let width = ((max.0 - min.0 + 2 * reach) / cell + 1) as usize;
        let height = ((max.1 - min.1 + 2 * reach) / cell + 1) as usize;
        let words = width.div_ceil(64);
        UnionSlices {
            origin,
            cell,
            width,
            height,
            words,
            layers: (0..stage.grid.len())
                .map(|_| Mutex::new(Vec::new()))
                .collect(),
            bases: Mutex::new(Rings::new()),
            collisions: (0..stage.grid.len()).map(|_| Mutex::new(None)).collect(),
            closing: (0.5 * NOTCH_CLOSING_MM / CELL_MM).round() as usize,
            outside: Mutex::new(false),
        }
    }

    /// Fills the slices of one graph (`draw::slices`) and adds its own base.
    pub fn add(&self, stage: &StageA, slices: &[Vec<Ring>]) {
        if let Some(layer0) = slices.first()
            && !layer0.is_empty()
        {
            let base = member_base(stage, layer0.clone());
            let mut bases = lock(&self.bases);
            *bases = union(&bases, &base);
        }
        for (layer, rings) in slices.iter().enumerate() {
            let Some(bitmap) = self.layers.get(layer) else {
                continue;
            };
            if rings.is_empty() {
                continue;
            }
            let mut spans: Vec<(usize, usize, usize)> = Vec::new();
            for ring in rings {
                if !self.fill(ring, &mut spans) {
                    *lock(&self.outside) = true;
                    return;
                }
            }
            // The member's own layer as `draw::finish_layer` treats it: trimmed by the
            // object, then closed over notches narrower than a line, on the cells.
            let trimmed = Rows::from_spans(spans).difference(&self.collision(stage, layer));
            let closed = trimmed.dilate(self.closing).erode(self.closing).union(&trimmed);
            let mut bits = lock(bitmap);
            if bits.is_empty() {
                bits.resize(self.words * self.height, 0);
            }
            for (row, from, to) in closed.spans() {
                set_span(&mut bits[row * self.words..(row + 1) * self.words], from, to);
            }
        }
    }

    /// The cells lying wholly inside the object's radius-0 collision of `layer` (cells whose
    /// centre is inside, less one cell all round), rasterized once per layer.
    fn collision(&self, stage: &StageA, layer: usize) -> std::sync::Arc<Rows> {
        let mut cache = lock(&self.collisions[layer]);
        if let Some(rows) = cache.as_ref() {
            return rows.clone();
        }
        let rings = stage.clearance.collision(0, layer, true);
        let rows = std::sync::Arc::new(self.centre_rows(&rings).erode(1));
        *cache = Some(rows.clone());
        rows
    }

    /// The cells whose centres lie inside `rings` (non-zero winding over all rings), clipped
    /// to the window.
    fn centre_rows(&self, rings: &[Ring]) -> Rows {
        let mut spans = Vec::new();
        let Some(found) = bbox(rings) else {
            return Rows::default();
        };
        let cell = self.cell as f64;
        let (ox, oy) = (f64::from(self.origin.x), f64::from(self.origin.y));
        let first_row = ((f64::from(found.min.y) - oy) / cell - 0.5).ceil().max(0.0) as i64;
        let last_row = (((f64::from(found.max.y) - oy) / cell - 0.5).floor() as i64)
            .min(self.height as i64 - 1);
        let mut crossings: Vec<(f64, i32)> = Vec::new();
        for row in first_row..=last_row {
            let y = oy + (row as f64 + 0.5) * cell;
            crossings.clear();
            for ring in rings {
                for (index, a) in ring.iter().enumerate() {
                    let b = ring[(index + 1) % ring.len()];
                    let (ay, by) = (f64::from(a.y), f64::from(b.y));
                    if (ay <= y) != (by <= y) {
                        let t = (y - ay) / (by - ay);
                        let x = f64::from(a.x) + t * (f64::from(b.x) - f64::from(a.x));
                        crossings.push((x, if by > ay { 1 } else { -1 }));
                    }
                }
            }
            crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
            let mut winding = 0;
            for pair in crossings.windows(2) {
                winding += pair[0].1;
                if winding == 0 {
                    continue;
                }
                let from = ((pair[0].0 - ox) / cell - 0.5).ceil().max(0.0) as usize;
                let to = (((pair[1].0 - ox) / cell - 0.5).ceil().max(0.0) as usize).min(self.width);
                if from < to {
                    spans.push((row as usize, from, to));
                }
            }
        }
        Rows::from_spans(spans)
    }

    /// The cells (row, first, past-last column) of the ring: per row, the cells whose centres
    /// lie between the ring's least and greatest x on the row's centre line (a slice is convex
    /// or nearly so); a row the ring crosses without covering a centre gets the one cell in the
    /// middle of the ring's part of the row, so a thin sliver is never lost (covering every
    /// touched cell instead grew the union by 0.06 mm all round and cost P1S 2 goats).
    /// `false` when the ring leaves the window.
    fn fill(&self, ring: &Ring, spans: &mut Vec<(usize, usize, usize)>) -> bool {
        let Some(found) = bbox(std::slice::from_ref(ring)) else {
            return true;
        };
        let cell = self.cell as f64;
        let (ox, oy) = (f64::from(self.origin.x), f64::from(self.origin.y));
        let row_of = |y: f64| ((y - oy) / cell).floor() as i64;
        let column_of = |x: f64| ((x - ox) / cell).floor() as i64;
        let (first_row, last_row) = (row_of(f64::from(found.min.y)), row_of(f64::from(found.max.y)));
        if column_of(f64::from(found.min.x)) < 0
            || first_row < 0
            || column_of(f64::from(found.max.x)) + 1 >= self.width as i64
            || last_row + 1 >= self.height as i64
        {
            return false;
        }
        let rows = (last_row - first_row + 1) as usize;
        let mut extent = vec![(f64::INFINITY, f64::NEG_INFINITY); rows];
        let mut centre = vec![(f64::INFINITY, f64::NEG_INFINITY); rows];
        let widen = |list: &mut Vec<(f64, f64)>, row: i64, x: f64| {
            let slot = &mut list[(row - first_row) as usize];
            slot.0 = slot.0.min(x);
            slot.1 = slot.1.max(x);
        };
        for (index, a) in ring.iter().enumerate() {
            let b = ring[(index + 1) % ring.len()];
            let (ax, ay, bx, by) = (
                f64::from(a.x),
                f64::from(a.y),
                f64::from(b.x),
                f64::from(b.y),
            );
            let (low, high) = (row_of(ay.min(by)), row_of(ay.max(by)));
            for row in low..=high {
                // The part of the edge inside the strip of `row`.
                let (y0, y1) = (oy + row as f64 * cell, oy + (row + 1) as f64 * cell);
                let at = |y: f64| {
                    if by == ay {
                        (ax, bx)
                    } else {
                        let t = ((y - ay) / (by - ay)).clamp(0.0, 1.0);
                        let x = ax + t * (bx - ax);
                        (x, x)
                    }
                };
                let (p, q) = (at(y0.max(ay.min(by))), at(y1.min(ay.max(by))));
                for x in [p.0, p.1, q.0, q.1] {
                    widen(&mut extent, row, x);
                }
                let middle = oy + (row as f64 + 0.5) * cell;
                if (ay <= middle) != (by <= middle) {
                    widen(&mut centre, row, at(middle).0);
                }
            }
        }
        for (offset, ((low, high), (left, right))) in extent.into_iter().zip(centre).enumerate() {
            if low > high {
                continue;
            }
            let row = first_row as usize + offset;
            let from = ((left - ox) / cell - 0.5).ceil().max(0.0) as usize;
            let to = ((((right - ox) / cell - 0.5).floor() + 1.0).max(0.0) as usize).min(self.width);
            if left <= right && from < to {
                spans.push((row, from, to));
            } else {
                let middle = column_of(0.5 * (low + high)).max(0) as usize;
                spans.push((row, middle, middle + 1));
            }
        }
        true
    }

    #[doc(hidden)]
    pub fn raw(&self, layer: usize) -> Rings {
        let shape = (self.origin, self.cell, self.width, self.height, self.words);
        let bits = lock(&self.layers[layer]).clone();
        if bits.is_empty() { return Rings::new(); }
        rings_of(&bits, shape)
    }

    /// The finished tube layers of the union and the union of the members' bases (`None`
    /// when a slice left the window).
    pub fn finish(self, stage: &StageA) -> Option<(Vec<Rings>, Rings)> {
        use rayon::prelude::*;
        if into_inner(self.outside) {
            return None;
        }
        let shape = (self.origin, self.cell, self.width, self.height, self.words);
        let bases = into_inner(self.bases);
        let tubes = self.layers
                .into_par_iter()
                .enumerate()
                .map(|(layer, bitmap)| {
                    let bits = into_inner(bitmap);
                    if bits.is_empty() {
                        return Rings::new();
                    }
                    {
                        // The cells' staircase (steps of a cell) would reach the band unions
                        // with a vertex per cell edge: 8 GB in one union_all.
                        let stairs = rings_of(&bits, shape);
                        let smooth = simplify(&stairs, self.cell as f64, false);
                        trim_layer(stage, layer, &smooth)
                    }
                })
                .collect();
        Some((tubes, bases))
    }
}

type Shape = (IntPoint, i64, usize, usize, usize);

/// The set cells as rings in the object frame: one rectangle per run of a row, united.
fn rings_of(bits: &[u64], (origin, cell, width, _, words): Shape) -> Vec<Ring> {
    let mut rectangles: Vec<Ring> = Vec::new();
    for (row, cells) in bits.chunks_exact(words).enumerate() {
        let mut column = 0;
        while column < width {
            if !get(cells, column) {
                column += 1;
                continue;
            }
            let start = column;
            while column < width && get(cells, column) {
                column += 1;
            }
            let x0 = i64::from(origin.x) + start as i64 * cell;
            let x1 = i64::from(origin.x) + column as i64 * cell;
            let y0 = i64::from(origin.y) + row as i64 * cell;
            let y1 = y0 + cell;
            let corner = |x: i64, y: i64| IntPoint::new(x as i32, y as i32);
            rectangles.push(vec![
                corner(x0, y0),
                corner(x1, y0),
                corner(x1, y1),
                corner(x0, y1),
            ]);
        }
    }
    resolve(&rectangles, FillRule::NonZero)
}

fn get(cells: &[u64], column: usize) -> bool {
    cells[column / 64] >> (column % 64) & 1 == 1
}

fn set_span(cells: &mut [u64], from: usize, to: usize) {
    for column in from..to {
        cells[column / 64] |= 1 << (column % 64);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn into_inner<T>(mutex: Mutex<T>) -> T {
    mutex
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A set of cells as sorted, disjoint, half-open column spans per row; `rows[i]` is row
/// `first + i`.
#[derive(Clone, Debug, Default)]
struct Rows {
    first: usize,
    rows: Vec<Vec<(usize, usize)>>,
}

impl Rows {
    fn from_spans(mut spans: Vec<(usize, usize, usize)>) -> Rows {
        if spans.is_empty() {
            return Rows::default();
        }
        spans.sort_unstable();
        let first = spans[0].0;
        let last = spans[spans.len() - 1].0;
        let mut rows: Vec<Vec<(usize, usize)>> = vec![Vec::new(); last - first + 1];
        for (row, from, to) in spans {
            let list = &mut rows[row - first];
            match list.last_mut() {
                Some(previous) if from <= previous.1 => previous.1 = previous.1.max(to),
                _ => list.push((from, to)),
            }
        }
        Rows { first, rows }
    }

    fn row(&self, row: i64) -> &[(usize, usize)] {
        if row < self.first as i64 {
            return &[];
        }
        self.rows
            .get((row - self.first as i64) as usize)
            .map_or(&[], Vec::as_slice)
    }

    fn spans(&self) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
        self.rows.iter().enumerate().flat_map(move |(index, list)| {
            list.iter().map(move |&(from, to)| (self.first + index, from, to))
        })
    }

    /// Half widths of the digital disc of `radius` cells per row offset.
    fn disc(radius: usize) -> Vec<usize> {
        let r = radius as f64;
        (0..=radius)
            .map(|dy| (r * r - (dy * dy) as f64).max(0.0).sqrt().floor() as usize)
            .collect()
    }

    fn dilate(&self, radius: usize) -> Rows {
        if self.rows.is_empty() || radius == 0 {
            return self.clone();
        }
        let widths = Rows::disc(radius);
        let first = self.first.saturating_sub(radius);
        let last = self.first + self.rows.len() - 1 + radius;
        let mut spans = Vec::new();
        for row in first..=last {
            for dy in -(radius as i64)..=radius as i64 {
                let width = widths[dy.unsigned_abs() as usize];
                for &(from, to) in self.row(row as i64 + dy) {
                    spans.push((row, from.saturating_sub(width), to + width));
                }
            }
        }
        Rows::from_spans(spans)
    }

    fn erode(&self, radius: usize) -> Rows {
        if self.rows.is_empty() || radius == 0 {
            return self.clone();
        }
        let widths = Rows::disc(radius);
        let mut spans = Vec::new();
        for index in 0..self.rows.len() {
            let row = (self.first + index) as i64;
            let mut kept: Vec<(usize, usize)> = self.rows[index]
                .iter()
                .filter_map(|&(from, to)| shrink(from, to, widths[0]))
                .collect();
            for dy in 1..=radius as i64 {
                if kept.is_empty() {
                    break;
                }
                let width = widths[dy as usize];
                for other in [row - dy, row + dy] {
                    let shrunk: Vec<(usize, usize)> = self
                        .row(other)
                        .iter()
                        .filter_map(|&(from, to)| shrink(from, to, width))
                        .collect();
                    kept = intersect(&kept, &shrunk);
                }
            }
            spans.extend(kept.into_iter().map(|(from, to)| (row as usize, from, to)));
        }
        Rows::from_spans(spans)
    }

    fn union(&self, other: &Rows) -> Rows {
        Rows::from_spans(self.spans().chain(other.spans()).collect())
    }

    fn difference(&self, other: &Rows) -> Rows {
        let mut spans = Vec::new();
        for (index, list) in self.rows.iter().enumerate() {
            let row = self.first + index;
            let cut = other.row(row as i64);
            for &(from, to) in list {
                let mut start = from;
                for &(a, b) in cut {
                    if b <= start || a >= to {
                        continue;
                    }
                    if a > start {
                        spans.push((row, start, a));
                    }
                    start = start.max(b);
                }
                if start < to {
                    spans.push((row, start, to));
                }
            }
        }
        Rows::from_spans(spans)
    }
}

fn shrink(from: usize, to: usize, width: usize) -> Option<(usize, usize)> {
    let (from, to) = (from + width, to.checked_sub(width)?);
    (from < to).then_some((from, to))
}

fn intersect(a: &[(usize, usize)], b: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        let (from, to) = (a[i].0.max(b[j].0), a[i].1.min(b[j].1));
        if from < to {
            out.push((from, to));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

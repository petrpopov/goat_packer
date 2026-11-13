//! The union of a layer's pieces and its notch closing on a fine raster instead of as vector
//! booleans. Thousands of nearly coincident tube slices per layer (every distinct graph of an
//! ensemble) made the vector union and the two closing offsets about a third of a support
//! model run; on row spans both are a few sorted merges per row.
//!
//! The result is a superset of the vector one: a piece covers every pixel its outline meets
//! (pieces are convex, so a row's run is the piece's x extent over the row's band), the
//! closing works on that cover, and the outline is the exact boundary of the pixel set. Only
//! the cells the outline crosses are added, less than a pixel diagonal around the vector
//! result, so margins calibrated against the vector union stay safe.

use std::collections::BTreeMap;

use crate::polygon_ops::{IntPoint, Ring, Rings};

/// Sorted, disjoint, half-open column runs per row; `rows[0]` is pixel row `first`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Runs {
    first: i64,
    rows: Vec<Vec<(i64, i64)>>,
}

/// Every pixel of side `pixel` (grid units) that a piece of `rings` meets. The pixel `(i, j)`
/// covers `[i·pixel, (i + 1)·pixel) × [j·pixel, (j + 1)·pixel)`.
pub(super) fn cover(rings: &[Ring], pixel: i64) -> Runs {
    let p = pixel as f64;
    let mut runs: Vec<(i64, i64, i64)> = Vec::new();
    // Per ring, the x extent over every row band its edges cross: the convex piece's own
    // extent there (and a superset for any other outline).
    let mut extents: Vec<(f64, f64)> = Vec::new();
    for ring in rings.iter().filter(|ring| ring.len() >= 3) {
        let low = ring.iter().map(|point| point.y).min().expect("ring") as f64;
        let high = ring.iter().map(|point| point.y).max().expect("ring") as f64;
        let row0 = (low / p).floor() as i64;
        let row1 = (high / p).floor() as i64;
        extents.clear();
        extents.resize((row1 - row0 + 1) as usize, (f64::INFINITY, f64::NEG_INFINITY));
        for (index, &a) in ring.iter().enumerate() {
            let b = ring[(index + 1) % ring.len()];
            let (a, b) = (
                (f64::from(a.x), f64::from(a.y)),
                (f64::from(b.x), f64::from(b.y)),
            );
            let (bottom, top) = if a.1 <= b.1 { (a, b) } else { (b, a) };
            let first = (bottom.1 / p).floor() as i64;
            let last = (top.1 / p).floor() as i64;
            for row in first..=last {
                // The edge clipped to the row's band, by its y range.
                let (y0, y1) = (row as f64 * p, (row + 1) as f64 * p);
                let at = |y: f64| {
                    if top.1 == bottom.1 {
                        (bottom.0, top.0)
                    } else {
                        let t = ((y - bottom.1) / (top.1 - bottom.1)).clamp(0.0, 1.0);
                        let x = bottom.0 + t * (top.0 - bottom.0);
                        (x, x)
                    }
                };
                let (s0, s1) = at(y0.max(bottom.1));
                let (e0, e1) = at(y1.min(top.1));
                let extent = &mut extents[(row - row0) as usize];
                extent.0 = extent.0.min(s0.min(s1).min(e0).min(e1));
                extent.1 = extent.1.max(s0.max(s1).max(e0).max(e1));
            }
        }
        for (offset, &(x0, x1)) in extents.iter().enumerate() {
            if x0 <= x1 {
                let start = (x0 / p).floor() as i64;
                let end = (x1 / p).floor() as i64 + 1;
                runs.push((row0 + offset as i64, start, end));
            }
        }
    }
    Runs::from_runs(runs)
}

impl Runs {
    fn from_runs(runs: Vec<(i64, i64, i64)>) -> Runs {
        let (Some(first), Some(last)) = (
            runs.iter().map(|run| run.0).min(),
            runs.iter().map(|run| run.0).max(),
        ) else {
            return Runs::default();
        };
        let mut rows: Vec<Vec<(i64, i64)>> = vec![Vec::new(); (last - first + 1) as usize];
        for (row, start, end) in runs {
            rows[(row - first) as usize].push((start, end));
        }
        Runs::from_rows(first, rows)
    }

    /// Rows of unsorted, possibly overlapping runs, sorted and merged row by row.
    fn from_rows(first: i64, mut rows: Vec<Vec<(i64, i64)>>) -> Runs {
        for spans in &mut rows {
            spans.sort_unstable();
            let mut merged: Vec<(i64, i64)> = Vec::with_capacity(spans.len());
            for &(start, end) in spans.iter() {
                match merged.last_mut() {
                    Some(open) if start <= open.1 => open.1 = open.1.max(end),
                    _ => merged.push((start, end)),
                }
            }
            *spans = merged;
        }
        Runs { first, rows }
    }

    fn is_empty(&self) -> bool {
        self.rows.iter().all(Vec::is_empty)
    }

    /// Every pixel within `radius` pixels (centre to centre) of a pixel of `self`.
    pub(super) fn dilated(&self, radius: f64) -> Runs {
        let reach = radius.floor() as i64;
        let widths: Vec<i64> = (-reach..=reach)
            .map(|dy| ((radius * radius - (dy * dy) as f64).max(0.0)).sqrt().floor() as i64)
            .collect();
        let mut rows: Vec<Vec<(i64, i64)>> = vec![Vec::new(); self.rows.len() + 2 * reach as usize];
        for (index, spans) in self.rows.iter().enumerate() {
            for &(start, end) in spans {
                for (k, &width) in widths.iter().enumerate() {
                    rows[index + k].push((start - width, end + width));
                }
            }
        }
        Runs::from_rows(self.first - reach, rows)
    }

    /// The pixels of the box `[x0, x1) × [y0, y1)` outside `self`.
    fn complement_in(&self, (x0, y0, x1, y1): (i64, i64, i64, i64)) -> Runs {
        let rows = (y0..y1)
            .map(|row| {
                let mut out = Vec::new();
                let mut at = x0;
                for &(start, end) in self.row(row) {
                    if start > at {
                        out.push((at, start.min(x1)));
                    }
                    at = at.max(end);
                }
                if at < x1 {
                    out.push((at, x1));
                }
                out.retain(|&(start, end)| start < end);
                out
            })
            .collect();
        Runs { first: y0, rows }
    }

    fn row(&self, row: i64) -> &[(i64, i64)] {
        usize::try_from(row - self.first)
            .ok()
            .and_then(|index| self.rows.get(index))
            .map_or(&[], Vec::as_slice)
    }

    /// The box around every pixel, `[x0, x1) × [y0, y1)`.
    fn bounds(&self) -> Option<(i64, i64, i64, i64)> {
        let mut found: Option<(i64, i64, i64, i64)> = None;
        for (index, spans) in self.rows.iter().enumerate() {
            let (Some(first), Some(last)) = (spans.first(), spans.last()) else {
                continue;
            };
            let row = self.first + index as i64;
            found = Some(found.map_or((first.0, row, last.1, row + 1), |(x0, y0, x1, _)| {
                (x0.min(first.0), y0, x1.max(last.1), row + 1)
            }));
        }
        found
    }

    /// The closing by a disc of `radius` pixels: dilated, then eroded (the complement of the
    /// dilated complement), so every notch narrower than the disc fills.
    pub(super) fn closed(&self, radius: f64) -> Runs {
        if self.is_empty() {
            return Runs::default();
        }
        let grown = self.dilated(radius);
        let Some((x0, y0, x1, y1)) = grown.bounds() else {
            return Runs::default();
        };
        let margin = radius.ceil() as i64 + 1;
        let frame = (x0 - margin, y0 - margin, x1 + margin, y1 + margin);
        let outside = grown.complement_in(frame).dilated(radius);
        let mut closed = outside.complement_in(frame);
        closed.rows.iter_mut().for_each(|spans| spans.retain(|&(a, b)| a < b));
        closed
    }

    /// The exact outline of the pixel set in grid units: outer rings counter-clockwise, holes
    /// clockwise (the material on the left of every edge).
    pub(super) fn outline(&self, pixel: i64) -> Rings {
        // Directed boundary edges between pixel corners, keyed by their start corner.
        // Ordered, so the rings and their start corners do not depend on a hash seed.
        let mut edges: BTreeMap<(i64, i64), Vec<(i64, i64)>> = BTreeMap::new();
        let mut add = |from: (i64, i64), to: (i64, i64)| edges.entry(from).or_default().push(to);
        let count = self.rows.len() as i64;
        for index in 0..=count {
            let row = self.first + index;
            let (below, above) = (self.row(row - 1), self.row(row));
            for &(start, end) in above {
                // Left side down, right side up.
                add((start, row + 1), (start, row));
                add((end, row), (end, row + 1));
            }
            // Horizontal boundary at y = row: material above only runs +x, below only -x.
            for (start, end) in difference(above, below) {
                add((start, row), (end, row));
            }
            for (start, end) in difference(below, above) {
                add((end, row), (start, row));
            }
        }
        let mut rings = Rings::new();
        while let Some((&start, _)) = edges.first_key_value() {
            let mut ring: Vec<(i64, i64)> = vec![start];
            let mut at = start;
            let mut heading = (0i64, 0i64);
            loop {
                let next = take_edge(&mut edges, at, heading);
                heading = ((next.0 - at.0).signum(), (next.1 - at.1).signum());
                at = next;
                if at == start {
                    break;
                }
                ring.push(at);
            }
            let ring = without_collinear(ring);
            if ring.len() >= 3 {
                rings.push(
                    ring.into_iter()
                        .map(|(x, y)| IntPoint::new((x * pixel) as i32, (y * pixel) as i32))
                        .collect(),
                );
            }
        }
        rings
    }
}

/// The parts of `a` outside `b` (both sorted, disjoint).
fn difference(a: &[(i64, i64)], b: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut k = 0;
    for &(start, end) in a {
        let mut at = start;
        while k < b.len() && b[k].1 <= at {
            k += 1;
        }
        let mut j = k;
        while j < b.len() && b[j].0 < end {
            if b[j].0 > at {
                out.push((at, b[j].0));
            }
            at = at.max(b[j].1);
            j += 1;
        }
        if at < end {
            out.push((at, end));
        }
    }
    out
}

/// Takes the edge leaving `at`; where two leave (pixels touching at a corner), the one turning
/// left of `heading`, so diagonal neighbours stay separate rings.
fn take_edge(
    edges: &mut BTreeMap<(i64, i64), Vec<(i64, i64)>>,
    at: (i64, i64),
    heading: (i64, i64),
) -> (i64, i64) {
    let outgoing = edges.get_mut(&at).expect("a closed boundary");
    let pick = if outgoing.len() == 1 {
        0
    } else {
        let left = (-heading.1, heading.0);
        outgoing
            .iter()
            .position(|&to| ((to.0 - at.0).signum(), (to.1 - at.1).signum()) == left)
            .unwrap_or(0)
    };
    let next = outgoing.swap_remove(pick);
    if outgoing.is_empty() {
        edges.remove(&at);
    }
    next
}

/// Drops the corners where the outline goes straight on.
fn without_collinear(ring: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    let n = ring.len();
    (0..n)
        .filter(|&i| {
            let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
            (b.0 - a.0) * (c.1 - b.1) != (b.1 - a.1) * (c.0 - b.0)
        })
        .map(|i| ring[i])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::polygon_ops::{area, difference as vector_difference, offset, Join, union_many};

    fn square(x: i32, y: i32, size: i32) -> Ring {
        vec![
            IntPoint::new(x, y),
            IntPoint::new(x + size, y),
            IntPoint::new(x + size, y + size),
            IntPoint::new(x, y + size),
        ]
    }

    fn disc(cx: i32, cy: i32, r: f64, n: usize) -> Ring {
        (0..n)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                IntPoint::new(
                    cx + (r * a.cos()).round() as i32,
                    cy + (r * a.sin()).round() as i32,
                )
            })
            .collect()
    }

    #[test]
    fn the_cover_of_squares_on_the_grid_is_exact_and_outlined_with_holes() {
        // A 3x3 block of 10-unit pixels with the centre missing: one outer ring, one hole.
        let rings: Rings = (0..3)
            .flat_map(|i| (0..3).map(move |j| (i, j)))
            .filter(|&(i, j)| (i, j) != (1, 1))
            .map(|(i, j)| square(i * 10 + 2, j * 10 + 2, 6))
            .collect();
        let runs = cover(&rings, 10);
        let outline = runs.outline(10);
        assert_eq!(outline.len(), 2);
        assert_eq!(area(&outline), 800.0);
    }

    #[test]
    fn the_raster_union_covers_the_vector_union_within_a_pixel() {
        let rings: Rings = (0..40)
            .map(|k| disc(1000 * (k % 7), 900 * (k / 7), 1300.0 + 37.0 * k as f64, 48))
            .collect();
        let pixel = 50;
        let vector = union_many(&rings);
        let raster = cover(&rings, pixel).outline(pixel);
        assert!(area(&vector_difference(&vector, &raster)) == 0.0);
        let grown = offset(&vector, 1.5 * pixel as f64, Join::Round(5.0));
        assert!(area(&vector_difference(&raster, &grown)) == 0.0);
    }

    #[test]
    fn closing_fills_a_notch_narrower_than_the_disc_and_keeps_a_wider_one() {
        // Two blocks of pixels 0..10 and 13..23, 3 pixels apart, joined by rows 0..3: a
        // U-shaped notch (the squares stay off the pixel edges, which a cover would take).
        let rings = vec![square(1, 1, 98), square(131, 1, 98)];
        let base = cover(&rings, 10);
        let joined = Runs::from_runs(
            base.rows
                .iter()
                .enumerate()
                .flat_map(|(i, spans)| spans.iter().map(move |&(a, b)| (base.first + i as i64, a, b)))
                .chain((0..3).map(|row| (row, 0, 23)))
                .collect(),
        );
        let inside = |runs: &Runs, x: i64, y: i64| runs.row(y).iter().any(|&(a, b)| a <= x && x < b);
        // A disc wider than half the gap fills it below the top edge (a closing leaves a dent
        // where the notch opens, as the vector closing does); a smaller one keeps it.
        let filled = joined.closed(2.5);
        assert!((3..8).all(|y| (10..13).all(|x| inside(&filled, x, y))));
        let kept = joined.closed(1.0);
        // (it only rounds the notch's inner corners on row 3)
        assert!((4..8).all(|y| (10..13).all(|x| !inside(&kept, x, y))));
        assert!(area(&filled.outline(10)) > area(&kept.outline(10)));
        // The closing never removes a pixel.
        assert!(area(&vector_difference(&joined.outline(10), &filled.outline(10))) == 0.0);
    }
}

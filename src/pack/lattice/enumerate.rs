//! The admissible lattices of one rotation class (design 2026-11-02, §9.2 member A).
//!
//! A lattice is admissible when no nonzero lattice vector lies in the no-fit set N of the
//! class with itself: then any two points of it are legal neighbours, and every phase of it
//! is a valid layout before the plate is looked at. The densest lattices touch their
//! neighbours, so their basis vectors lie on the boundary of N: the free pixels next to N.
//! Pairs of such vectors give every candidate; the minimum determinant over the admissible
//! ones sets the cut-off, and the lattices within `DET_SLACK` of it are kept (a slightly
//! looser lattice often has a phase that fills the plate better).

use super::hnf::{Hnf, PixelBox};
use crate::span_raster::SpanRaster;
use rayon::prelude::*;
use std::sync::atomic::{AtomicI64, Ordering};

/// Lattices with a determinant up to this multiple of the smallest admissible one.
pub const DET_SLACK: f64 = 1.25;

/// Most lattices kept (smallest determinant first, then the HNF tuple): a rectangle has one
/// lattice per shear, which would be thousands of identical-density candidates.
pub const MAX_LATTICES: usize = 4096;

type Vector = (i64, i64);

/// N as a bitmap with O(1) lookups.
pub struct NoFitBits {
    min_x: i32,
    min_y: i32,
    width: i32,
    height: i32,
    bits: Vec<bool>,
}

impl NoFitBits {
    pub(crate) fn new(no_fit: &SpanRaster) -> NoFitBits {
        let spans = no_fit.rows.iter().flatten();
        let min_x = spans.clone().map(|s| s.0).min().unwrap_or(0);
        let max_x = spans.map(|s| s.1).max().unwrap_or(0);
        let height = no_fit.rows.len() as i32;
        let width = max_x - min_x;
        let mut bits = vec![false; (width.max(0) * height) as usize];
        for (row, spans) in no_fit.rows.iter().enumerate() {
            for &(start, end) in spans {
                let base = row * width as usize;
                bits[base + (start - min_x) as usize..base + (end - min_x) as usize].fill(true);
            }
        }
        NoFitBits {
            min_x,
            min_y: no_fit.min_y,
            width,
            height,
            bits,
        }
    }

    /// The shift `(x, y)` collides.
    pub fn hit(&self, x: i64, y: i64) -> bool {
        let (x, y) = (x - i64::from(self.min_x), y - i64::from(self.min_y));
        x >= 0
            && y >= 0
            && x < i64::from(self.width)
            && y < i64::from(self.height)
            && self.bits[(y * i64::from(self.width) + x) as usize]
    }

    /// The box around N (inclusive); `None` when N is empty.
    pub fn bounds(&self) -> Option<PixelBox> {
        (self.width > 0 && self.height > 0).then(|| {
            (
                self.min_x,
                self.min_y,
                self.min_x + self.width - 1,
                self.min_y + self.height - 1,
            )
        })
    }

    /// No nonzero vector of `lattice` is in N. Every vector that could be in N is inside its
    /// bounding box, so the lattice points of that box are all there is to test.
    pub fn admits(&self, lattice: &Hnf) -> bool {
        let Some(bounds) = self.bounds() else {
            return true;
        };
        let origin = lattice.coset_of(0, 0);
        lattice
            .points_in(origin, bounds)
            .into_iter()
            .all(|(x, y)| (x, y) == (0, 0) || !self.hit(i64::from(x), i64::from(y)))
    }

    /// Free pixels with a collision on one of their four sides, in the upper half plane (a
    /// vector and its negative give the same lattice), in scan order.
    fn boundary(&self) -> Vec<Vector> {
        let mut boundary = Vec::new();
        let (x0, y0) = (i64::from(self.min_x) - 1, i64::from(self.min_y) - 1);
        let (x1, y1) = (
            i64::from(self.min_x + self.width),
            i64::from(self.min_y + self.height),
        );
        for y in y0.max(0)..=y1 {
            for x in x0..=x1 {
                let upper = y > 0 || x > 0;
                let touches = self.hit(x - 1, y)
                    || self.hit(x + 1, y)
                    || self.hit(x, y - 1)
                    || self.hit(x, y + 1);
                if upper && touches && !self.hit(x, y) {
                    boundary.push((x, y));
                }
            }
        }
        boundary
    }
}

/// Every admissible lattice within `DET_SLACK` of the densest one, at most `MAX_LATTICES`,
/// ordered by determinant then HNF tuple. Empty when N is empty (nothing to keep apart: the
/// caller falls back to the other members) or no lattice is admissible.
pub(crate) fn enumerate(no_fit: &SpanRaster) -> Vec<Hnf> {
    let bits = NoFitBits::new(no_fit);
    let boundary = bits.boundary();
    let Some(min_det) = min_admissible_det(&bits, &boundary) else {
        return Vec::new();
    };
    let cap = (min_det as f64 * DET_SLACK) as i64;
    let mut found: Vec<Hnf> = boundary
        .par_iter()
        .flat_map_iter(|&v1| contacts(&bits, &boundary, v1, cap))
        .collect();
    found.sort_by_key(|l| (l.det(), *l));
    found.dedup();
    found.retain(|l| bits.admits(l));
    found.truncate(MAX_LATTICES);
    found
}

/// The pairs `(v1, v2)` with `0 < det <= cap` that could generate an admissible lattice:
/// neither the sum nor the difference is in N (a cheap test that drops most pairs).
fn contacts<'a>(
    bits: &'a NoFitBits,
    boundary: &'a [Vector],
    v1: Vector,
    cap: i64,
) -> impl Iterator<Item = Hnf> + 'a {
    boundary.iter().filter_map(move |&v2| {
        let det = v1.0 * v2.1 - v1.1 * v2.0;
        let plausible = det > 0
            && det <= cap
            && !bits.hit(v1.0 + v2.0, v1.1 + v2.1)
            && !bits.hit(v1.0 - v2.0, v1.1 - v2.1);
        plausible.then(|| Hnf::from_basis(v1, v2)).flatten()
    })
}

/// The smallest determinant of an admissible lattice generated by two boundary vectors.
/// Parallel over the first vector; the shared bound only skips pairs that cannot lower the
/// minimum, so the result does not depend on the order the pairs are met in.
fn min_admissible_det(bits: &NoFitBits, boundary: &[Vector]) -> Option<i64> {
    let best = AtomicI64::new(i64::MAX);
    boundary.par_iter().for_each(|&v1| {
        let cap = best.load(Ordering::Relaxed).saturating_sub(1);
        let lowest = contacts(bits, boundary, v1, cap)
            .filter(|lattice| lattice.det() < best.load(Ordering::Relaxed))
            .filter(|lattice| bits.admits(lattice))
            .map(|lattice| lattice.det())
            .min();
        if let Some(det) = lowest {
            best.fetch_min(det, Ordering::Relaxed);
        }
    });
    let best = best.into_inner();
    (best != i64::MAX).then_some(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N of two `w x h` boxes: collision when both shifts are under the size.
    fn box_no_fit(w: i32, h: i32) -> SpanRaster {
        SpanRaster::rect(-w + 1, -h + 1, w - 1, h - 1)
    }

    #[test]
    fn a_box_packs_in_its_own_size_cells_with_every_shear() {
        let lattices = enumerate(&box_no_fit(6, 4));
        assert_eq!(lattices[0].det(), 24);
        assert!(lattices.iter().all(|l| l.det() <= 30));
        // The plain grid is among them, and so is the half-brick shift.
        assert!(lattices.contains(&Hnf::from_basis((6, 0), (0, 4)).unwrap()));
        assert!(lattices.contains(&Hnf::from_basis((6, 0), (3, 4)).unwrap()));
        let mut sorted = lattices.clone();
        sorted.sort_by_key(|l| (l.det(), *l));
        assert_eq!(sorted, lattices, "deterministic order, no duplicates");
        sorted.dedup();
        assert_eq!(sorted.len(), lattices.len());
    }

    #[test]
    fn every_enumerated_lattice_is_admissible_and_none_denser_exists() {
        // A plus-shaped N: brute force over all small bases.
        let mut n = box_no_fit(7, 3);
        n = n.union(&SpanRaster::rect(-2, -6, 2, 6));
        let bits = NoFitBits::new(&n);
        let lattices = enumerate(&n);
        assert!(lattices.iter().all(|l| bits.admits(l)));
        let mut best = i64::MAX;
        for a in -14..=14i64 {
            for b in -14..=14i64 {
                for c in -14..=14i64 {
                    for d in -14..=14i64 {
                        let lattice = Hnf::from_basis((a, b), (c, d));
                        if let Some(l) = lattice.filter(|l| l.det() < best && bits.admits(l)) {
                            best = l.det();
                        }
                    }
                }
            }
        }
        assert_eq!(lattices[0].det(), best);
    }

    #[test]
    fn nothing_to_keep_apart_gives_no_lattice() {
        assert!(enumerate(&SpanRaster::default()).is_empty());
    }
}

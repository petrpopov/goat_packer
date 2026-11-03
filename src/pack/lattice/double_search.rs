//! Which lattices a double lattice may use (design 2026-11-02, §9.2 member B): class A on a
//! lattice L, class B on L + w. L must keep every A-A, B-B and A-B pair legal, so no nonzero
//! vector of L lies in N_AA or N_BB, and no vector of L + w lies in N_AB.
//!
//! The densest pairs touch, so L's basis vectors lie on the boundary of the forbidden set
//! F_w = N_AA u N_BB u (N_AB - w) u -(N_AB - w) and w on the boundary of N_AB. Scanning every
//! such w at full resolution is too slow, so the scan runs on the 2x subsampled sets (exact
//! for even lattices, see `span_ops`), keeps the densest few (L, w), and a local search of
//! +-`REFINE` pixels around each, at full resolution, recovers the odd vectors.

use super::enumerate::{NoFitBits, enumerate};
use super::hnf::Hnf;
use super::span_ops::{negate, subsample2, translate};
use crate::span_raster::SpanRaster;
use rayon::prelude::*;

/// Coarse pairs within this multiple of the densest one are kept (design: 1.10).
pub const DET_SLACK: f64 = 1.30;
/// Coarse (lattice, w) pairs refined, densest first.
pub const COARSE_POOL: usize = 256;
/// The full-resolution search window around each coarse vector, pixels.
pub const REFINE: i64 = 2;
/// Lattices per w kept by the coarse scan: more only repeat shears of the same cell.
const PER_W: usize = 16;

type Vector = (i64, i64);

/// The no-fit sets of one class pair at full resolution.
pub(crate) struct PairSets<'a> {
    pub aa: &'a SpanRaster,
    pub bb: &'a SpanRaster,
    pub ab: &'a SpanRaster,
}

/// The fine lattices worth scoring, densest first (ties: HNF order), at most `pool`. Empty
/// when no admissible pair exists.
pub(crate) fn seed_lattices(sets: &PairSets, pool: usize) -> Vec<Hnf> {
    let coarse = coarse_pairs(sets);
    let bits = FineBits {
        aa: NoFitBits::new(sets.aa),
        bb: NoFitBits::new(sets.bb),
        ab: NoFitBits::new(sets.ab),
    };
    let mut lattices: Vec<Hnf> = coarse
        .par_iter()
        .flat_map_iter(|&(lattice, w)| refine(&bits, &lattice, w))
        .collect();
    lattices.sort_by_key(|l| (l.det(), *l));
    lattices.dedup();
    lattices.truncate(pool);
    lattices
}

/// Densest admissible coarse pairs (lattice, w): per w the admissible lattices of the union
/// F_w, then globally those within `DET_SLACK` of the smallest determinant, one entry per
/// lattice (the first w), `COARSE_POOL` at most.
fn coarse_pairs(sets: &PairSets) -> Vec<(Hnf, Vector)> {
    let (aa, bb, ab) = (
        subsample2(sets.aa),
        subsample2(sets.bb),
        subsample2(sets.ab),
    );
    let fixed = aa.union(&bb);
    let ws = boundary_pixels(&NoFitBits::new(&ab));
    let per_w: Vec<Vec<(Hnf, Vector)>> = ws
        .par_iter()
        .map(|&w| {
            let shifted = translate(&ab, -(w.0 as i32), -(w.1 as i32));
            let forbidden = fixed.union(&shifted).union(&negate(&shifted));
            let mut found = enumerate(&forbidden);
            found.truncate(PER_W);
            found.into_iter().map(|l| (l, w)).collect()
        })
        .collect();
    let mut all: Vec<(Hnf, Vector)> = per_w.into_iter().flatten().collect();
    all.sort_by_key(|&(l, w)| (l.det(), l, w));
    let Some(min_det) = all.first().map(|(l, _)| l.det()) else {
        return Vec::new();
    };
    let cap = (min_det as f64 * DET_SLACK) as i64;
    all.retain(|(l, _)| l.det() <= cap);
    all.dedup_by_key(|(l, _)| *l);
    all.truncate(COARSE_POOL);
    all
}

/// Free pixels with a collision on one of their four sides, in scan order.
fn boundary_pixels(bits: &NoFitBits) -> Vec<Vector> {
    let Some((x0, y0, x1, y1)) = bits.bounds() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for y in i64::from(y0) - 1..=i64::from(y1) + 1 {
        for x in i64::from(x0) - 1..=i64::from(x1) + 1 {
            let touches = bits.hit(x - 1, y)
                || bits.hit(x + 1, y)
                || bits.hit(x, y - 1)
                || bits.hit(x, y + 1);
            if touches && !bits.hit(x, y) {
                found.push((x, y));
            }
        }
    }
    found
}

struct FineBits {
    aa: NoFitBits,
    bb: NoFitBits,
    ab: NoFitBits,
}

/// The full-resolution lattices near the doubled coarse `lattice` and `w` that are
/// admissible and no sparser than it: every basis vector and w moved by up to `REFINE`.
fn refine(bits: &FineBits, lattice: &Hnf, w: Vector) -> Vec<Hnf> {
    let [u, v] = lattice.reduced();
    let (u, v) = (
        (2 * i64::from(u.0), 2 * i64::from(u.1)),
        (2 * i64::from(v.0), 2 * i64::from(v.1)),
    );
    let w = (2 * w.0, 2 * w.1);
    let limit = 4 * lattice.det();
    let mut found = Vec::new();
    for du in window() {
        for dv in window() {
            let basis = ((u.0 + du.0, u.1 + du.1), (v.0 + dv.0, v.1 + dv.1));
            let Some(fine) = Hnf::from_basis(basis.0, basis.1) else {
                continue;
            };
            if fine.det() > limit || !bits.aa.admits(&fine) || !bits.bb.admits(&fine) {
                continue;
            }
            if window().any(|dw| coset_clear(&bits.ab, &fine, (w.0 + dw.0, w.1 + dw.1))) {
                found.push(fine);
            }
        }
    }
    found
}

fn window() -> impl Iterator<Item = Vector> + Clone {
    (-REFINE..=REFINE).flat_map(|x| (-REFINE..=REFINE).map(move |y| (x, y)))
}

/// No vector of the coset of `w` (every `w + l`) is in N.
pub(crate) fn coset_clear(n: &NoFitBits, lattice: &Hnf, w: Vector) -> bool {
    let Some(bounds) = n.bounds() else {
        return true;
    };
    let coset = lattice.coset_of(w.0 as i32, w.1 as i32);
    lattice
        .points_in(coset, bounds)
        .into_iter()
        .all(|(x, y)| !n.hit(i64::from(x), i64::from(y)))
}

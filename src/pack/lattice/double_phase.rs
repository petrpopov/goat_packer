//! The best joint phases of a double lattice, exactly (design 2026-11-02, §9.2, §9.3).
//!
//! A layout is class A on coset `a` of the lattice L and class B on coset `b`; every A-A and
//! B-B pair is legal by admissibility, and an A-B pair is legal iff the coset `b - a` has no
//! pixel of N_AB (and `a - b` none of N_BA). Folding both no-fit sets modulo L marks the
//! blocked cosets, and the histograms of the two base maps give each coset's free origins,
//! so the plain count of a pair is `count_a[a] + count_b[b]` and the best pairs are found
//! without placing anything.

use super::hnf::Hnf;
use super::residue::{TowerOrigins, histogram};
use crate::span_raster::SpanRaster;

/// A pair of cosets and the free origins they hold before the tower.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Pair {
    pub a: usize,
    pub b: usize,
    pub plain: u32,
}

/// The base maps and no-fit sets of the two classes.
pub(crate) struct Maps<'a> {
    pub base_a: &'a SpanRaster,
    pub base_b: &'a SpanRaster,
    pub ab: &'a SpanRaster,
    pub ba: &'a SpanRaster,
}

/// Work above which the pairs are found by scanning cosets in count order instead of per
/// relative shift (checks, a constant: never the thread count).
const SHIFT_SCAN_BUDGET: usize = 8_000_000;

/// The `limit` best legal coset pairs of `lattice`, by plain count, then the order the
/// search met them in (deterministic, independent of anything but the inputs).
pub(crate) fn best_pairs(maps: &Maps, lattice: &Hnf, limit: usize) -> Vec<Pair> {
    let count_a = histogram(maps.base_a, lattice);
    let count_b = histogram(maps.base_b, lattice);
    let blocked = Blocked {
        lattice: *lattice,
        ab: histogram(maps.ab, lattice),
        ba: histogram(maps.ba, lattice),
    };
    let shifts: Vec<usize> = (0..lattice.cosets())
        .filter(|&w| blocked.ab[w] == 0)
        .collect();
    if shifts.is_empty() {
        return Vec::new();
    }
    if shifts.len() * lattice.cosets() <= SHIFT_SCAN_BUDGET {
        by_shift(&count_a, &count_b, &blocked, &shifts, limit)
    } else {
        by_count(&count_a, &count_b, &blocked, limit)
    }
}

struct Blocked {
    lattice: Hnf,
    ab: Vec<u32>,
    ba: Vec<u32>,
}

impl Blocked {
    /// The coset `to - from`.
    fn difference(&self, to: usize, from: usize) -> usize {
        let s = self.lattice.s as usize;
        let point = |c: usize| ((c % s) as i32, (c / s) as i32);
        let (to, from) = (point(to), point(from));
        self.lattice.coset_of(to.0 - from.0, to.1 - from.1)
    }

    /// A on coset `a` and B on coset `b` never collide.
    fn legal(&self, a: usize, b: usize) -> bool {
        self.ab[self.difference(b, a)] == 0 && self.ba[self.difference(a, b)] == 0
    }
}

/// For each unblocked shift `w`, the best `a` (B then on `a + w`); the best `limit` of them.
fn by_shift(
    count_a: &[u32],
    count_b: &[u32],
    blocked: &Blocked,
    shifts: &[usize],
    limit: usize,
) -> Vec<Pair> {
    let s = blocked.lattice.s as usize;
    let mut top = Top::new(limit);
    for &w in shifts {
        let w = ((w % s) as i32, (w / s) as i32);
        let mut best: Option<Pair> = None;
        for (a, &free_a) in count_a.iter().enumerate().filter(|(_, c)| **c > 0) {
            let (x, y) = ((a % s) as i32, (a / s) as i32);
            let b = blocked.lattice.coset_of(x + w.0, y + w.1);
            let plain = free_a + count_b[b];
            if count_b[b] > 0 && best.is_none_or(|p| plain > p.plain) && blocked.legal(a, b) {
                best = Some(Pair { a, b, plain });
            }
        }
        if let Some(pair) = best {
            top.offer(pair);
        }
    }
    top.into_pairs()
}

/// Cosets of A by falling count, each against the cosets of B by falling count, until no
/// pair can beat the `limit`-th found.
fn by_count(count_a: &[u32], count_b: &[u32], blocked: &Blocked, limit: usize) -> Vec<Pair> {
    let order = |counts: &[u32]| {
        let mut order: Vec<usize> = (0..counts.len()).filter(|&c| counts[c] > 0).collect();
        order.sort_by_key(|&c| (std::cmp::Reverse(counts[c]), c));
        order
    };
    let (order_a, order_b) = (order(count_a), order(count_b));
    let Some(&best_b) = order_b.first() else {
        return Vec::new();
    };
    let mut top = Top::new(limit);
    for &a in &order_a {
        if top.cannot_beat(count_a[a] + count_b[best_b]) {
            break;
        }
        for &b in &order_b {
            let plain = count_a[a] + count_b[b];
            if top.cannot_beat(plain) {
                break;
            }
            if blocked.legal(a, b) {
                top.offer(Pair { a, b, plain });
            }
        }
    }
    top.into_pairs()
}

/// The best `limit` pairs so far; a later pair of equal count never replaces an earlier.
struct Top {
    limit: usize,
    pairs: Vec<Pair>,
}

impl Top {
    fn new(limit: usize) -> Top {
        Top {
            limit,
            pairs: Vec::new(),
        }
    }

    fn cannot_beat(&self, plain: u32) -> bool {
        self.pairs.len() == self.limit && self.pairs.last().is_some_and(|p| plain <= p.plain)
    }

    fn offer(&mut self, pair: Pair) {
        if self.cannot_beat(pair.plain) {
            return;
        }
        let at = self.pairs.partition_point(|p| p.plain >= pair.plain);
        self.pairs.insert(at, pair);
        self.pairs.truncate(self.limit);
    }

    fn into_pairs(self) -> Vec<Pair> {
        self.pairs
    }
}

/// A pair's score under its best tower anchor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Scored {
    pub pair: Pair,
    pub score: u32,
    pub anchor: Option<usize>,
}

/// The towers' forbidden origins of both classes, per anchor.
pub(crate) struct AnchorLosses<'a> {
    pub a: Vec<TowerOrigins<'a>>,
    pub b: Vec<TowerOrigins<'a>>,
}

/// `pairs` scored under the anchor that loses the fewest origins (the first on ties), best
/// score first (ties keep the plain order). Without anchors the plain count is the score.
pub(crate) fn rescore(lattice: &Hnf, pairs: &[Pair], towers: &AnchorLosses) -> Vec<Scored> {
    let walker = lattice.walker();
    let mut scored: Vec<Scored> = pairs
        .iter()
        .map(|&pair| {
            let loss = |anchor: usize| {
                towers.a[anchor].loss(&walker, pair.a, u32::MAX)
                    + towers.b[anchor].loss(&walker, pair.b, u32::MAX)
            };
            let best = (0..towers.a.len()).map(|i| (loss(i), i)).min();
            let (loss, anchor) = best.map_or((0, None), |(l, i)| (l, Some(i)));
            Scored {
                pair,
                score: pair.plain.saturating_sub(loss),
                anchor,
            }
        })
        .collect();
    scored.sort_by_key(|s| std::cmp::Reverse(s.score));
    scored
}

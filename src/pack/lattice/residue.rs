//! The best phase of a lattice, exactly (design 2026-11-02, §9.2, §9.3).
//!
//! Every translation of a lattice is a coset of it. For an admissible lattice all points of
//! one coset are legal neighbours, so the layout a coset gives has as many copies as the
//! coset has free origins. Folding the feasible-origin bitmap modulo the lattice (one pass
//! over its free spans) counts every coset at once. The tower is a removable set of origins:
//! its anchor changes the count of a coset only by the coset's points inside it, which a few
//! lookups give, and only cosets whose plain count can still beat the current top need them.

use super::hnf::{Hnf, PixelBox, Walker};
use crate::kind_masks::OriginSet;
use crate::span_raster::SpanRaster;

/// Free origins per coset of `lattice`, indexed by `Hnf::coset_of`. `free` is the base map of
/// the class (plate zones only).
pub(crate) fn histogram(free: &SpanRaster, lattice: &Hnf) -> Vec<u32> {
    let (s, t, h) = (lattice.s as usize, i64::from(lattice.t), lattice.h);
    let mut counts = vec![0u32; lattice.cosets()];
    for (index, spans) in free.rows.iter().enumerate() {
        let y = free.min_y + index as i32;
        let (row, r) = (y.div_euclid(h), y.rem_euclid(h) as usize);
        let line = &mut counts[r * s..(r + 1) * s];
        let shear = i64::from(row) * t;
        for &(start, end) in spans {
            let first = (i64::from(start) - shear).rem_euclid(s as i64) as usize;
            add_cyclic(line, first, (end - start) as usize);
        }
    }
    counts
}

/// Adds one to `len` consecutive cells of `line` from `first`, wrapping around.
fn add_cyclic(line: &mut [u32], first: usize, len: usize) {
    let s = line.len();
    let (turns, rest) = ((len / s) as u32, len % s);
    if turns > 0 {
        line.iter_mut().for_each(|c| *c += turns);
    }
    let head = rest.min(s - first);
    line[first..first + head].iter_mut().for_each(|c| *c += 1);
    line[..rest - head].iter_mut().for_each(|c| *c += 1);
}

/// The origins an anchor's tower takes out of a class, with the box around them.
pub struct TowerOrigins<'a> {
    set: &'a OriginSet,
    bounds: Option<PixelBox>,
}

impl<'a> TowerOrigins<'a> {
    pub fn new(set: &'a OriginSet) -> TowerOrigins<'a> {
        let rows = &set.spans.rows;
        let mut bounds: Option<PixelBox> = None;
        for (index, row) in rows.iter().enumerate().filter(|(_, r)| !r.is_empty()) {
            let y = set.spans.min_y + index as i32;
            let (x0, x1) = (row[0].0, row[row.len() - 1].1 - 1);
            bounds =
                Some(bounds.map_or((x0, y, x1, y), |(a, b, c, _)| (a.min(x0), b, c.max(x1), y)));
        }
        TowerOrigins { set, bounds }
    }

    /// Points of `coset` the tower removes, counted up to `cutoff` (a caller that only
    /// wants to know whether the loss is below it stops there).
    pub fn loss(&self, lattice: &Walker, coset: usize, cutoff: u32) -> u32 {
        let Some(bounds) = self.bounds else {
            return 0;
        };
        lattice
            .points(coset, bounds)
            .filter(|&(x, y)| self.set.contains(x, y))
            .take(cutoff as usize)
            .count() as u32
    }
}

/// One phase worth trying: its coset, its exact count with the tower at `anchor`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Phase {
    pub coset: usize,
    pub score: u32,
    /// Index into the caller's tower list; `None` without a tower.
    pub anchor: Option<usize>,
}

/// The `limit` best phases of a lattice under the best anchor each, by score then by the
/// order of the plain counts (count descending, coset ascending). `towers` is one entry per
/// anchor. A phase takes the first anchor that loses the fewest points.
///
/// Exact and incremental: cosets are met by falling plain count, a coset's score is its
/// count minus its smallest loss, and the scan stops at the first coset whose plain count
/// cannot beat the `limit`-th score so far (a tie keeps the earlier coset).
pub fn best_phases(
    lattice: &Hnf,
    counts: &[u32],
    towers: &[TowerOrigins<'_>],
    limit: usize,
) -> Vec<Phase> {
    let walker = lattice.walker();
    let mut order: Vec<u32> = (0..counts.len() as u32)
        .filter(|&c| counts[c as usize] > 0)
        .collect();
    order.sort_unstable_by_key(|&c| (std::cmp::Reverse(counts[c as usize]), c));
    let mut top: Vec<Phase> = Vec::new();
    for coset in order.into_iter().map(|c| c as usize) {
        let plain = counts[coset];
        if top.len() == limit && top.last().is_some_and(|last| plain <= last.score) {
            break;
        }
        // Only a loss below this keeps the coset in the top `limit`.
        let bound = match top.last() {
            Some(last) if top.len() == limit => plain - last.score - 1,
            _ => plain - 1,
        };
        let Some((loss, anchor)) = smallest_loss(&walker, coset, towers, bound) else {
            continue;
        };
        let score = plain - loss;
        let at = top.partition_point(|p| p.score >= score);
        top.insert(
            at,
            Phase {
                coset,
                score,
                anchor,
            },
        );
        top.truncate(limit);
    }
    top
}

/// The fewest points any anchor removes from `coset` and the first anchor doing so, if that is
/// at most `bound`; without towers nothing is removed.
fn smallest_loss(
    lattice: &Walker,
    coset: usize,
    towers: &[TowerOrigins<'_>],
    bound: u32,
) -> Option<(u32, Option<usize>)> {
    if towers.is_empty() {
        return Some((0, None));
    }
    let mut best: Option<(u32, usize)> = None;
    for (anchor, tower) in towers.iter().enumerate() {
        // Counting stops one past what could still improve on the best so far.
        let cutoff = best.map_or(bound, |(loss, _)| loss.saturating_sub(1).min(bound)) + 1;
        let loss = tower.loss(lattice, coset, cutoff);
        if loss < cutoff {
            best = Some((loss, anchor));
            if loss == 0 {
                break;
            }
        }
    }
    best.map(|(loss, anchor)| (loss, Some(anchor)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A notched box of free origins crossing several spans per row.
    fn free() -> SpanRaster {
        let rect = SpanRaster::rect(-9, -7, 60, 41);
        rect.difference(&SpanRaster::rect(10, 5, 24, 20))
            .difference(&SpanRaster::rect(-3, 30, 70, 31))
    }

    /// A tower's origin set: the part of the box that is in the base map, as the context
    /// builds it.
    fn tower_set(rect: SpanRaster) -> OriginSet {
        let free = free();
        OriginSet {
            spans: free.difference(&free.difference(&rect)),
        }
    }

    fn brute(free: &SpanRaster, lattice: &Hnf) -> Vec<u32> {
        let mut counts = vec![0u32; lattice.cosets()];
        for y in -20..60 {
            for x in -30..90 {
                if free.contains(x, y) {
                    counts[lattice.coset_of(x, y)] += 1;
                }
            }
        }
        counts
    }

    #[test]
    fn the_folded_histogram_equals_a_brute_force_count() {
        for basis in [
            ((7, 2), (-3, 6)),
            ((5, 0), (2, 9)),
            ((90, 0), (0, 1)),
            ((13, 4), (4, 13)),
        ] {
            let lattice = Hnf::from_basis(basis.0, basis.1).unwrap();
            assert_eq!(
                histogram(&free(), &lattice),
                brute(&free(), &lattice),
                "{basis:?}"
            );
        }
    }

    #[test]
    fn rescoring_with_a_tower_equals_a_full_recount() {
        let free = free();
        let lattice = Hnf::from_basis((7, 2), (-3, 6)).unwrap();
        let counts = histogram(&free, &lattice);
        let sets = [
            tower_set(SpanRaster::rect(0, 0, 20, 14)),
            tower_set(SpanRaster::rect(30, 10, 55, 40)),
            OriginSet::default(),
        ];
        let towers: Vec<TowerOrigins> = sets.iter().map(TowerOrigins::new).collect();
        for (index, set) in sets.iter().enumerate() {
            let recount = histogram(&free.difference(&set.spans), &lattice);
            let expected_best = *recount.iter().max().unwrap();
            let phases = best_phases(&lattice, &counts, &towers[index..=index], 3);
            assert_eq!(phases[0].score, expected_best, "anchor {index}");
            for p in &phases {
                assert_eq!(p.score, recount[p.coset]);
            }
            assert!(phases.windows(2).all(|w| w[0].score >= w[1].score));
        }
    }

    #[test]
    fn the_best_anchor_per_phase_is_the_first_with_the_smallest_loss() {
        let free = free();
        let lattice = Hnf::from_basis((7, 2), (-3, 6)).unwrap();
        let counts = histogram(&free, &lattice);
        let sets = [
            tower_set(SpanRaster::rect(0, 0, 20, 14)),
            tower_set(SpanRaster::rect(30, 10, 55, 40)),
        ];
        let towers: Vec<TowerOrigins> = sets.iter().map(TowerOrigins::new).collect();
        let phases = best_phases(&lattice, &counts, &towers, 5);
        for p in phases {
            let losses: Vec<u32> = towers
                .iter()
                .map(|t| t.loss(&lattice.walker(), p.coset, u32::MAX))
                .collect();
            let min = *losses.iter().min().unwrap();
            assert_eq!(p.score, counts[p.coset] - min);
            assert_eq!(p.anchor, losses.iter().position(|&l| l == min));
        }
    }

    #[test]
    fn a_larger_limit_extends_the_prefix() {
        let free = free();
        let lattice = Hnf::from_basis((11, 3), (-2, 8)).unwrap();
        let counts = histogram(&free, &lattice);
        let set = tower_set(SpanRaster::rect(0, 0, 25, 25));
        let towers = [TowerOrigins::new(&set)];
        let few = best_phases(&lattice, &counts, &towers, 3);
        let many = best_phases(&lattice, &counts, &towers, 9);
        assert_eq!(few, many[..3]);
    }
}

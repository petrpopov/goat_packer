//! Greedy bottom-left sweeps (design 2026-11-02, §9.2 member D), moved from
//! `packing_search`: a sweep repeatedly takes, over every class of the run, the free origin
//! whose copy reaches least far in the sweep direction, and clears that copy's no-fit sets
//! from every class's free map. Variants differ by start corner, corner cycling, the class of
//! the first copy and where along the first row it starts. No lookahead, no backtracking.
//!
//! Also here: the angle-family ranking and the mixed palette of the legacy search, where a
//! footprint may turn freely.

use super::context::Ctx;
use super::free_map::FreeMap;
use crate::layout::{PixelPlacement, cap_reached};
use rayon::prelude::*;
use std::cmp::Reverse;
use std::ops::Range;

/// A sweep starts from one of the bed's four corners.
const CORNERS: usize = 4;
/// Variants per first class between two start phases: all corners, twice (fixed and cycling).
const START_PHASE_STRIDE: usize = 2 * CORNERS;
/// Candidate heights are compared as integers at this many units per pixel, so float noise
/// cannot reorder two candidates that are equally high.
const HEIGHT_QUANTUM: f64 = 1e6;
/// Most sweep variants any single run makes, however high the requested effort.
pub(crate) const MAX_SWEEP_VARIANTS: usize = 64;
/// Mixed palette: the strongest single-orientation families are intermixed rather than
/// restricting the whole plate to a half-turn pair, and orthogonal orientations are added as
/// well — rectangles can need a windmill layout even when all single-orientation runs tie.
const MIXED_FAMILIES: usize = 4;
/// Quarter turns tried around each shortlisted family's angle (degrees).
pub(crate) const MIXED_OFFSETS_DEG: [f64; 4] = [0.0, 90.0, 180.0, 270.0];
/// Sweep variants per palette angle in the mixed pass.
pub(crate) const MIXED_VARIANTS_PER_ANGLE: usize = 8;

/// What one variant index varies. Corners run fastest, then the class of the first copy,
/// then fixed vs cycling corners, then the start phase. The phase is not taken modulo
/// anything: with a single class the old schedule ran out of phases after 32 variants and
/// variants 32..63 repeated 0..31 exactly (half of the locked budget).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Variant {
    corner: usize,
    first_class: usize,
    cycling: bool,
    phase: usize,
}

impl Variant {
    fn of(variant: usize, classes: usize) -> Variant {
        Variant {
            corner: variant % CORNERS,
            first_class: (variant / CORNERS) % classes,
            cycling: !(variant / (CORNERS * classes)).is_multiple_of(2),
            phase: variant / (START_PHASE_STRIDE * classes),
        }
    }

    /// Which corner the sweep starts from for the next copy (bit 0 = from the right, bit 1 =
    /// from the top): a fixed corner, or one cycling around the four corners. The latter
    /// avoids greedy height traps such as four 6x4 rectangles around a 2x2 central hole.
    fn corner(self, placed: usize) -> usize {
        if self.cycling {
            self.corner ^ [0, 1, 3, 2][placed % CORNERS]
        } else {
            self.corner
        }
    }
}

/// Columns the first copy of a plate may start in: everything after a skip of the van der
/// Corput fraction of `phase` (0, 1/2, 1/4, 3/4, 1/8, 5/8, 3/8, 7/8, ...: the phase's binary
/// digits mirrored). Its first four terms are the four quarter positions the sweeps always
/// used, so runs with two or more classes keep their old schedule.
fn start_columns(phase: usize, width: usize, reverse_x: bool) -> Range<usize> {
    let bits = (usize::BITS - phase.leading_zeros()).max(2);
    let mirrored = phase.reverse_bits() >> (usize::BITS - bits);
    let skip = (mirrored * width) >> bits;
    if reverse_x {
        0..width - skip
    } else {
        skip..width
    }
}

/// A candidate origin for the next copy, ordered by the sweep's preference.
struct Next {
    /// Height of the copy's far edge in sweep direction, quantized so ties compare exactly.
    top: i64,
    side: i32,
    placement: PixelPlacement,
}

/// The best next placement over every class's free origins, or `None` when nothing fits.
fn next_placement(
    ctx: &Ctx,
    free: &[FreeMap],
    variant: Variant,
    placed: usize,
) -> Option<PixelPlacement> {
    let corner = variant.corner(placed);
    let (reverse_x, reverse_y) = (corner & 1 != 0, corner & 2 != 0);
    let phase = if placed == 0 { variant.phase } else { 0 };
    let resolution = ctx.grid().resolution;
    let candidates = free.iter().enumerate().filter_map(|(class, f)| {
        let columns = start_columns(phase, f.width(), reverse_x);
        let (x, y) = f.first_in(columns, reverse_x, reverse_y)?;
        let b = ctx.masks(class).extent_bounds;
        let top = if reverse_y {
            -f64::from(y) - b.min_y / resolution
        } else {
            f64::from(y) + b.max_y / resolution
        };
        Some(Next {
            top: (top * HEIGHT_QUANTUM).round() as i64,
            side: if reverse_x { -x } else { x },
            placement: PixelPlacement { class, x, y },
        })
    });
    let classes = ctx.classes();
    let rank = |class: usize| (class + classes - variant.first_class) % classes;
    let best = candidates.min_by(|a, b| {
        let (ca, cb) = (a.placement.class, b.placement.class);
        if placed == 0 {
            (rank(ca), a.top, a.side).cmp(&(rank(cb), b.top, b.side))
        } else {
            (a.top, a.side, ca).cmp(&(b.top, b.side, cb))
        }
    });
    best.map(|next| next.placement)
}

/// One greedy sweep over every class of `ctx`, with the tower at `anchor`.
fn sweep(ctx: &Ctx, anchor: Option<usize>, variant: Variant, cap: usize) -> Vec<PixelPlacement> {
    let mut free: Vec<FreeMap> = (0..ctx.classes())
        .map(|class| ctx.free_map(class, anchor))
        .collect();
    let mut placed = Vec::new();
    while let Some(p) = next_placement(ctx, &free, variant, placed.len()) {
        for (class, f) in free.iter_mut().enumerate() {
            f.exclude(ctx.no_fit(p.class, class), p.x, p.y);
        }
        placed.push(p);
        if cap_reached(placed.len(), cap) {
            break;
        }
    }
    placed
}

/// The best of a run's sweeps: most copies, then the smallest box, then the earliest variant.
pub(crate) struct SweepRun {
    pub(crate) layout: Vec<PixelPlacement>,
    pub(crate) variant: usize,
    /// (copies, box area in pixels) of the first sweep alone.
    pub(crate) baseline: (usize, u64),
}

/// `variants` sweeps over every class of `ctx` (local class ids), in parallel; the
/// reduction runs in variant order, so the result is the sequential one.
pub(crate) fn sweeps(ctx: &Ctx, anchor: Option<usize>, variants: usize, cap: usize) -> SweepRun {
    ctx.prepare_no_fit();
    let classes = ctx.classes();
    let layouts: Vec<Vec<PixelPlacement>> = (0..variants.max(1))
        .into_par_iter()
        .map(|v| sweep(ctx, anchor, Variant::of(v, classes), cap))
        .collect();
    let scores: Vec<(usize, u64)> = layouts
        .iter()
        .map(|layout| (layout.len(), ctx.bbox_area(layout)))
        .collect();
    let best = (0..scores.len())
        .min_by_key(|&v| (Reverse(scores[v].0), scores[v].1, v))
        .expect("at least one variant");
    SweepRun {
        layout: layouts.into_iter().nth(best).expect("chosen above"),
        variant: best,
        baseline: scores[0],
    }
}

/// Legacy angle families ordered by their first sweep alone (`baselines[i]` = family `i`):
/// increasing effort never changes this order, so a larger effort never drops a previously
/// searched mixed palette. Family indices, best first.
pub(crate) fn family_order(baselines: &[(usize, u64)]) -> Vec<usize> {
    let mut ranked: Vec<usize> = (0..baselines.len()).collect();
    ranked.sort_by(|&a, &b| {
        let (ba, bb) = (baselines[a], baselines[b]);
        bb.0.cmp(&ba.0).then(ba.1.cmp(&bb.1)).then(a.cmp(&b))
    });
    ranked
}

/// Angles (degrees) of the mixed palette: the best families' angles and their quarter turns,
/// without duplicates. Only a bounded local palette needs quadratic no-fit sets; the full
/// angle grid never incurs an O(angles^2) allocation.
pub(crate) fn mixed_palette(order: &[usize], angle_step_deg: f64) -> Vec<f64> {
    let mut angles: Vec<f64> = Vec::new();
    for &index in order.iter().take(MIXED_FAMILIES) {
        let base = index as f64 * angle_step_deg;
        for offset in MIXED_OFFSETS_DEG {
            let angle = (base + offset).rem_euclid(360.0);
            if !angles.iter().any(|&a| (a - angle).abs() < 1e-7) {
                angles.push(angle);
            }
        }
    }
    angles
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The schedule before the fix, for runs with two or more classes.
    fn old_variant(variant: usize, classes: usize) -> (usize, usize, bool, usize) {
        let quarter = [0, 2, 1, 3][(variant / (START_PHASE_STRIDE * classes)) % 4];
        (
            variant % CORNERS,
            (variant / CORNERS) % classes,
            !(variant / (CORNERS * classes)).is_multiple_of(2),
            quarter,
        )
    }

    #[test]
    fn a_single_class_gets_distinct_variants_for_the_whole_budget() {
        let width = 1000;
        let schedule: Vec<_> = (0..MAX_SWEEP_VARIANTS)
            .map(|v| {
                let variant = Variant::of(v, 1);
                let columns = start_columns(variant.phase, width, false);
                (variant.corner, variant.cycling, columns.start)
            })
            .collect();
        for (i, a) in schedule.iter().enumerate() {
            for b in &schedule[..i] {
                assert_ne!(a, b, "variant {i} repeats an earlier one");
            }
        }
    }

    #[test]
    fn two_or_more_classes_keep_the_old_schedule() {
        let width = 1077;
        for classes in [2, 3, 16] {
            for v in 0..MAX_SWEEP_VARIANTS {
                let variant = Variant::of(v, classes);
                let (corner, first, cycling, quarter) = old_variant(v, classes);
                assert_eq!(
                    (variant.corner, variant.first_class, variant.cycling),
                    (corner, first, cycling)
                );
                for reverse_x in [false, true] {
                    let skip = quarter * width / 4;
                    let old = if reverse_x {
                        0..width - skip
                    } else {
                        skip..width
                    };
                    assert_eq!(start_columns(variant.phase, width, reverse_x), old);
                }
            }
        }
    }

    #[test]
    fn start_phases_follow_the_van_der_corput_sequence() {
        let starts: Vec<usize> = (0..8).map(|p| start_columns(p, 800, false).start).collect();
        assert_eq!(starts, [0, 400, 200, 600, 100, 500, 300, 700]);
    }
}

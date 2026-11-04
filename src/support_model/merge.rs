//! Stage B3: merging branches that can meet (spec 02 §4). Per layer step the candidates are
//! reordered by their boxes (`order`), cut into buckets whose size depends on the slicer's
//! thread count (nondeterminism S2: `rounds`), and merged pairwise (`pair`). The bucket rule
//! is the only place a member's thread class enters; the pair test's overlap threshold is
//! the member's `Overlap` reading.

mod order;
pub mod pair;
mod rounds;

use super::descend::Moved;
use super::flavour::MergeBucket;
use super::stage_a::StageA;
use crate::polygon_ops::IntBox;

pub use order::aabb_order;
pub use rounds::{Buckets, merge_rounds};

/// Which way the box-tree order is read when buckets are cut (an ensemble axis of the model,
/// not of the slicer). The behaviour spec's order vectors fix the permutation but not whether
/// the slicer's own list runs along it or against it. `Along` is the default: with the roof
/// lines at the build's own angle, deterministic Orca slices and Bambu's both fit it, and
/// `Against` adds area without moving the worst miss (corpus campaign 2026-11-03). It stays
/// selectable (`--support-orders`) because the order of the tip list itself is a modelled
/// unknown (spec 01 §5.7, 02 §4.2).
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum MergeOrder {
    #[default]
    Along,
    Against,
}

/// How thick a pair's overlap must be to merge (an ensemble axis of the model, not of the
/// slicer). `Slicer` is the slicer's own rule (spec 02 §4.5); the stricter readings refuse
/// the marginal merges it accepts. Two influence areas that only touch at a corner merge or
/// not on rounding, the platform's floating point (S7) and everything that moved them
/// before, and one such decision moves a whole support foot by 2-5 mm: on the P1S goat at
/// one thread, the model merged a branch with its upper neighbour through a 0.8 x 0.25 mm
/// corner overlap where BambuStudio merged it with the lower one. No single threshold fits
/// (that member's first-layer miss: 5.2, 1.9, 3.1, 3.0 mm at 0.025, 0.1, 0.2, 0.4 mm), but
/// their union covers what the slicer prints at any thread count on macOS and Linux: over 52
/// slices of that goat the worst first-layer miss fell 4.26 -> 2.67 mm and the mean area
/// outside 48.7 -> 2.0 mm² (2026-11-04). Readings that accept a gap instead added nothing.
/// `Lone` (0.8 mm) keeps branches apart that the others still merge: a Linux slice at six
/// threads landed one branch alone as a third foot, 2.67 mm outside the four readings, 0.36
/// mm outside with it (the union grows 4 %); covering it by margin instead cost the P1S
/// plate 7 goats (41 -> 34).
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum Overlap {
    #[default]
    Slicer,
    Thicker,
    Thick,
    Thickest,
    Lone,
}

impl Overlap {
    pub const ALL: [Overlap; 5] = [
        Overlap::Slicer,
        Overlap::Thicker,
        Overlap::Thick,
        Overlap::Thickest,
        Overlap::Lone,
    ];

    /// How far the overlap must survive shrinking, mm.
    pub fn erosion_mm(self) -> f64 {
        match self {
            Overlap::Slicer => pair::THICK_OVERLAP_MM,
            Overlap::Thicker => 0.1,
            Overlap::Thick => 0.2,
            Overlap::Thickest => 0.4,
            Overlap::Lone => 0.8,
        }
    }
}

/// The slicer's thread class of a member (design §5.4): `threads` is the arena size T the
/// merge bucket rule compares against; a fixed-four build ignores it. `order` and `overlap`
/// are the model's own readings.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ThreadClass {
    pub threads: usize,
    pub order: MergeOrder,
    pub overlap: Overlap,
}

impl ThreadClass {
    /// Thread count `threads`, box-tree order read along the permutation, the slicer's own
    /// overlap rule.
    pub fn along(threads: usize) -> ThreadClass {
        ThreadClass {
            threads,
            order: MergeOrder::Along,
            overlap: Overlap::Slicer,
        }
    }
}

impl ThreadClass {
    /// m = ⌊(n + 2) / 4⌋ for a layer with n candidates (spec 02 §4.3).
    pub fn decision_value(candidates: usize) -> usize {
        (candidates + 2) / 4
    }

    /// Buckets of 4 when m ≥ T, else of 2; R always uses 4.
    pub fn buckets(self, stage: &StageA, candidates: usize) -> Buckets {
        let four = match stage.flavour.merge_bucket() {
            MergeBucket::FixedFour => true,
            MergeBucket::ThreadCount => ThreadClass::decision_value(candidates) >= self.threads,
        };
        Buckets {
            size: if four { 4 } else { 2 },
        }
    }
}

/// Merges the non-bypass candidates of the step to `layer`; also returns m for the thread
/// class walk (`None` when fewer than two candidates).
pub fn merge_layer(
    stage: &StageA,
    layer: usize,
    candidates: Vec<Moved>,
    class: ThreadClass,
) -> (Vec<Moved>, Option<usize>) {
    if candidates.len() < 2 {
        return (candidates, None);
    }
    let boxes: Vec<IntBox> = candidates
        .iter()
        .map(|candidate| candidate.bounds.expect("a merge candidate has a box"))
        .collect();
    let mut order = aabb_order(&boxes);
    if class.order == MergeOrder::Against {
        order.reverse();
    }
    let ordered: Vec<Moved> = order
        .into_iter()
        .map(|index| candidates[index].clone())
        .collect();
    let count = ordered.len();
    let buckets = class.buckets(stage, count);
    let test = pair::PairTest::new(stage, layer, class.overlap.erosion_mm());
    let merge = |destination: &Moved, source: &Moved| test.try_merge(destination, source);
    let merged = merge_rounds(ordered, buckets, &merge);
    (merged, Some(ThreadClass::decision_value(count)))
}

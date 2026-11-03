//! Stage B3: merging branches that can meet (spec 02 §4). Per layer step the candidates are
//! reordered by their boxes (`order`), cut into buckets whose size depends on the slicer's
//! thread count (nondeterminism S2: `rounds`), and merged pairwise (`pair`). The bucket rule
//! is the only place a member's thread class enters.

mod order;
pub mod pair;
mod rounds;

use super::descend::Moved;
use super::flavour::MergeBucket;
use super::stage_a::StageA;
use crate::polygon_ops::IntBox;

pub use order::aabb_order;
pub use rounds::{Buckets, merge_rounds};

/// The slicer's thread class of a member (design §5.4): `threads` is the arena size T the
/// merge bucket rule compares against; a fixed-four build ignores it.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ThreadClass {
    pub threads: usize,
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
    let ordered: Vec<Moved> = aabb_order(&boxes)
        .into_iter()
        .map(|index| candidates[index].clone())
        .collect();
    let count = ordered.len();
    let buckets = class.buckets(stage, count);
    let test = pair::PairTest::new(stage, layer);
    let merge = |destination: &Moved, source: &Moved| test.try_merge(destination, source);
    let merged = merge_rounds(ordered, buckets, &merge);
    (merged, Some(ThreadClass::decision_value(count)))
}

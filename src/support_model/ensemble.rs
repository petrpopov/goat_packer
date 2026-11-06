//! The ensemble of realizations the slicer can print for one object (design §5.4): every
//! thread class of the merge bucket rule (nondeterminism S2, spec 02 §4.7) for every arc
//! family of the line connector (S9, spec 01 §5.5), under the flavour's `Overlap` readings of the
//! pair test (marginal merges, `merge::Overlap`). The classes are found exactly: a run at
//! thread count T makes the decision "m ≥ T" on every merge, and all T that make the same
//! decisions give the same run, so the class of a run is the interval of T around it.
//!
//! The walk is a tree (`descend::descend_classes`): the classes share their descent down to
//! the first merge they decide differently, so each class is descended once, from there. It
//! replaced speculative batches of 16 thread counts from the top, which ran 704 descents for
//! the 424 members of a P1S goat under Bambu 02.09: 514 s per rotation at two threads (an M
//! Mac), and the server's two cores did not finish one rotation in the 30 minute job limit.
//! With the tree and the stage memo (`memo`) the same rotation takes 155 s. The result does
//! not depend on the order branches finish in.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::bands::{layers_with_base, realization};
use super::draw::slices;
use super::smooth::smooth;
use super::tips::RoofPiece;
use super::union::UnionSlices;
use super::clearance::Avoidance;
use super::flavour::MergeBucket;
use super::descend::{ClassDescent, descend_classes};
use super::graph::BranchGraph;
use super::place::place;
use super::merge::{MergeOrder, Overlap, ThreadClass};
use super::stage_a::StageA;
use super::tips::{ArcFamily, Seeds, seed_tips};
use crate::object_print::Realization;

/// One run of the generator.
#[derive(Clone, Debug, PartialEq)]
pub struct MemberRun {
    pub family: ArcFamily,
    pub order: MergeOrder,
    pub overlap: Overlap,
    /// A representative thread count of the class.
    pub threads: usize,
    /// The thread counts (a, b] that give this very run; `None` = unbounded.
    pub class: (usize, Option<usize>),
    /// The member's realization, or `None` when another member produced the very same graph
    /// first (equal graphs draw equal realizations, and drawing is most of a member's cost).
    pub realization: Option<Realization>,
    /// Hash of the branch graph and roofs: equal hashes are one realization.
    pub hash: u64,
}

/// The arc families of a build.
pub fn families(stage: &StageA) -> Vec<ArcFamily> {
    if stage.flavour.static_arc_thresholds() {
        vec![ArcFamily::Roof, ArcFamily::Base]
    } else {
        vec![ArcFamily::PerCall]
    }
}

/// Every member of the core ensemble for the given merge-order readings, in (family, order,
/// overlap, thread count) order. Members whose graph another member already drew carry no
/// realization.
pub fn run_members(stage: &StageA, orders: &[MergeOrder]) -> Vec<MemberRun> {
    run_members_with(stage, orders, stage.flavour.overlap_readings())
}

/// `run_members` under the given pair-test readings.
pub fn run_members_with(
    stage: &StageA,
    orders: &[MergeOrder],
    overlaps: &[Overlap],
) -> Vec<MemberRun> {
    walk_axes(stage, orders, overlaps, &Sink::Members)
}

/// The members and the union of their realizations (`union`): the slices of every distinct
/// graph are drawn into one bitmap per layer, and each layer is finished once instead of once
/// per graph. The realizations: that one union, or every distinct member's (`distinct`) in
/// the rare case a slice leaves the union's window.
pub fn run_union(
    stage: &StageA,
    orders: &[MergeOrder],
    overlaps: &[Overlap],
) -> (Vec<MemberRun>, Vec<Realization>) {
    let union = Union {
        slices: UnionSlices::new(stage),
        roofs: Mutex::default(),
    };
    let runs = walk_axes(stage, orders, overlaps, &Sink::Union(&union));
    let roofs: Vec<RoofPiece> = into_inner(union.roofs).into_values().flatten().collect();
    match union.slices.finish(stage) {
        Some((tubes, bases)) => {
            let realization = layers_with_base(stage, &tubes, &roofs, 0, &bases);
            (runs, vec![realization])
        }
        None => {
            let runs = run_members_with(stage, orders, overlaps);
            let realizations = distinct(&runs).into_iter().cloned().collect();
            (runs, realizations)
        }
    }
}

/// Where a member's drawing goes.
enum Sink<'a> {
    /// Into the member's own realization.
    Members,
    /// Into the shared union of `run_union`.
    Union(&'a Union),
}

/// The union's bitmaps and the roof pieces of every family that drew a graph.
struct Union {
    slices: UnionSlices,
    roofs: Mutex<BTreeMap<ArcFamily, Vec<RoofPiece>>>,
}

fn into_inner<T>(mutex: Mutex<T>) -> T {
    mutex
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn walk_axes(
    stage: &StageA,
    orders: &[MergeOrder],
    overlaps: &[Overlap],
    sink: &Sink,
) -> Vec<MemberRun> {
    warm_volumes(stage);
    let claims = Claims::default();
    // The tips depend on the family alone: seeded once, not once per member.
    let seeded: Vec<(ArcFamily, Seeds)> = families(stage)
        .into_par_iter()
        .map(|family| (family, seed_tips(stage, family)))
        .collect();
    let axes: Vec<(&(ArcFamily, Seeds), MergeOrder, Overlap)> = seeded
        .iter()
        .flat_map(|seeds| orders.iter().map(move |&order| (seeds, order)))
        .flat_map(|(seeds, order)| {
            overlaps
                .iter()
                .map(move |&overlap| (seeds, order, overlap))
        })
        .collect();
    axes.into_par_iter()
        .flat_map(|((family, seeds), order, overlap)| {
            walk_classes(stage, &claims, sink, *family, seeds, order, overlap)
        })
        .collect::<Vec<_>>()
}

/// Computes the avoidance volumes every member asks for, once, in parallel over (key, speed):
/// each is a sequential recurrence over the layers, so members started cold would all
/// recompute the same chain at once.
fn warm_volumes(stage: &StageA) {
    let Some(top) = stage.overhangs.highest_layer() else {
        return;
    };
    let keys = stage
        .clearance
        .precalculated_keys(&stage.derived, stage.derived.tip_layers + 4);
    let jobs: Vec<(i64, Avoidance)> = keys
        .iter()
        .flat_map(|&key| {
            [Avoidance::Slow, Avoidance::FastSafe, Avoidance::Fast]
                .into_iter()
                .map(move |kind| (key, kind))
        })
        .collect();
    jobs.into_par_iter().for_each(|(key, kind)| {
        stage.clearance.avoidance(key, top, kind, true);
    });
}

/// The class walk of one (family, order, overlap): the descent tree of `descend_classes`,
/// one member per class. A fixed-four build has one class.
fn walk_classes(
    stage: &StageA,
    claims: &Claims,
    sink: &Sink,
    family: ArcFamily,
    seeds: &Seeds,
    order: MergeOrder,
    overlap: Overlap,
) -> Vec<MemberRun> {
    let fixed = stage.flavour.merge_bucket() == MergeBucket::FixedFour;
    let class = ThreadClass {
        threads: 1,
        order,
        overlap,
    };
    let member = |class_descent: ClassDescent| {
        let ClassDescent {
            low,
            high,
            descended,
        } = class_descent;
        let started = std::time::Instant::now();
        let layers = place(descended.layers);
        let graph = BranchGraph {
            layers,
            roofs: seeds.roofs.clone(),
            merge_values: descended.merge_values,
            seconds: [
                0.0,
                descended.move_seconds,
                descended.merge_seconds,
                started.elapsed().as_secs_f64(),
            ],
        };
        let class = if fixed {
            class_of(&graph.merge_values, 1)
        } else {
            (low - 1, high)
        };
        debug_assert!(fixed || class == class_of(&graph.merge_values, low));
        let hash = graph_hash(&graph);
        // Whoever reaches a graph first draws it; which member that is does not change the set.
        let realization = if !claims.claim(hash) {
            None
        } else if let Sink::Union(union) = sink {
            let smoothed = smooth(stage, &graph.layers);
            union
                .slices
                .add(stage, &slices(stage, &graph.layers, &smoothed.centres));
            union
                .roofs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entry(family)
                .or_insert_with(|| graph.roofs.clone());
            None
        } else {
            Some(realization(stage, &graph, 0))
        };
        MemberRun {
            family,
            order,
            overlap,
            threads: low,
            class,
            hash,
            realization,
        }
    };
    descend_classes(stage, seeds.tips.clone(), class, &member)
}

/// The graphs already taken by a member.
#[derive(Default)]
struct Claims(std::sync::Mutex<std::collections::BTreeSet<u64>>);

impl Claims {
    fn claim(&self, hash: u64) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(hash)
    }
}

/// The thread counts (a, b] that decide like `threads` on every merge (spec 02 §4.7).
pub fn class_of(values: &[usize], threads: usize) -> (usize, Option<usize>) {
    let below = values.iter().copied().filter(|&m| m < threads).max();
    let above = values.iter().copied().filter(|&m| m >= threads).min();
    (below.unwrap_or(0), above)
}

/// FNV-1a over what drawing reads: every node's position, radius state and parents, and the
/// roof pieces.
fn graph_hash(graph: &BranchGraph) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut feed = |value: i64| {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for layer in &graph.layers {
        feed(-1);
        for node in layer {
            let centre = node.centre.unwrap_or_default();
            feed(i64::from(centre.x));
            feed(i64::from(centre.y));
            feed(node.rdepth);
            feed(node.depth);
            feed(node.foot.to_bits() as i64);
            for &parent in &node.parents {
                feed(parent as i64);
            }
            feed(-3);
        }
    }
    for piece in &graph.roofs {
        feed(-2);
        feed(piece.layer as i64);
        feed(i64::from(piece.interface));
        for point in piece.rings.iter().flatten() {
            feed(i64::from(point.x));
            feed(i64::from(point.y));
        }
    }
    hash
}

/// One realization per distinct graph, in the order of each graph's first member in run
/// order (which member actually drew it depends on timing, so the order must not).
pub fn distinct(runs: &[MemberRun]) -> Vec<&Realization> {
    let mut seen = std::collections::BTreeSet::new();
    runs.iter()
        .filter(|run| seen.insert(run.hash))
        .filter_map(|run| {
            runs.iter()
                .filter(|other| other.hash == run.hash)
                .find_map(|other| other.realization.as_ref())
        })
        .collect()
}

/// Serializable summary for reports.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MemberSummary {
    pub family: String,
    pub order: String,
    #[serde(default)]
    pub overlap: String,
    pub threads: usize,
    pub class_low: usize,
    pub class_high: Option<usize>,
    pub hash: String,
}

impl MemberRun {
    pub fn summary(&self) -> MemberSummary {
        MemberSummary {
            family: format!("{:?}", self.family),
            order: format!("{:?}", self.order),
            overlap: format!("{:?}", self.overlap),
            threads: self.threads,
            class_low: self.class.0,
            class_high: self.class.1,
            hash: format!("{:016x}", self.hash),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_class_is_the_interval_around_the_thread_count_between_the_merge_values() {
        let values = [2, 5, 5, 3, 9];
        assert_eq!(class_of(&values, 1), (0, Some(2)));
        assert_eq!(class_of(&values, 4), (3, Some(5)));
        assert_eq!(class_of(&values, 3), (2, Some(3)));
        assert_eq!(class_of(&values, 10), (9, None));
        // Every thread count in the interval makes the same decisions.
        for threads in 4..=5 {
            let decide = |t: usize| values.iter().map(|&m| m >= t).collect::<Vec<_>>();
            assert_eq!(decide(threads), decide(4));
        }
    }
}

//! The ensemble of realizations the slicer can print for one object (design §5.4): every
//! thread class of the merge bucket rule (nondeterminism S2, spec 02 §4.7) for every arc
//! family of the line connector (S9, spec 01 §5.5), under the flavour's `Overlap` readings of the
//! pair test (marginal merges, `merge::Overlap`). The classes are found exactly: a run at
//! thread count T makes the decision "m ≥ T" on every merge, and all T that make the same
//! decisions give the same run, so the class of a run is the interval of T around it.
//!
//! The walk is parallel by speculation: runs for T = 1..=K are independent, so they run
//! together on rayon, and only T beyond K (a layer with more candidates than K buckets) wait
//! for the next batch. The result does not depend on the order runs finish in: it is grouped
//! and ordered by (family, T) afterwards.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use super::bands::realization;
use super::clearance::Avoidance;
use super::flavour::MergeBucket;
use super::graph::{BranchGraph, branch_graph};
use super::merge::{MergeOrder, Overlap, ThreadClass};
use super::stage_a::StageA;
use super::tips::ArcFamily;
use crate::object_print::Realization;

/// Thread counts tried in the first batch; a batch covers up to this many new values.
const FIRST_BATCH: usize = 16;

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
    warm_volumes(stage);
    let claims = Claims::default();
    let axes: Vec<(ArcFamily, MergeOrder, Overlap)> = families(stage)
        .into_iter()
        .flat_map(|family| orders.iter().map(move |&order| (family, order)))
        .flat_map(|(family, order)| {
            overlaps
                .iter()
                .map(move |&overlap| (family, order, overlap))
        })
        .collect();
    axes.into_par_iter()
        .flat_map(|(family, order, overlap)| walk_classes(stage, &claims, family, order, overlap))
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

/// The class walk of one (family, order, overlap).
fn walk_classes(
    stage: &StageA,
    claims: &Claims,
    family: ArcFamily,
    order: MergeOrder,
    overlap: Overlap,
) -> Vec<MemberRun> {
    let fixed = stage.flavour.merge_bucket() == MergeBucket::FixedFour;
    let mut runs: Vec<MemberRun> = Vec::new();
    let mut next = 1;
    let mut batch = if fixed { 1 } else { FIRST_BATCH };
    loop {
        let fresh: Vec<MemberRun> = (next..next + batch)
            .into_par_iter()
            .map(|threads| run_one(stage, claims, family, order, overlap, threads))
            .collect();
        runs.extend(fresh);
        runs.sort_by_key(|run| run.threads);
        let mut covered = 1;
        let mut finished = fixed;
        let mut kept: Vec<MemberRun> = Vec::new();
        for run in std::mem::take(&mut runs) {
            // A run inside an earlier run's class repeats it (same decisions): the same graph,
            // so it hands over a realization it drew first.
            if run.threads < covered {
                if let Some(last) = kept.last_mut()
                    && last.realization.is_none()
                    && last.hash == run.hash
                {
                    last.realization = run.realization;
                }
                continue;
            }
            if run.threads > covered {
                // A gap in the classes: the thread counts in it are not covered by any run.
                break;
            }
            covered = run.class.1.map_or(usize::MAX, |upper| upper + 1);
            finished |= run.class.1.is_none();
            kept.push(run);
        }
        runs = kept;
        if finished {
            return runs;
        }
        next = covered;
        batch = FIRST_BATCH;
    }
}

fn run_one(
    stage: &StageA,
    claims: &Claims,
    family: ArcFamily,
    order: MergeOrder,
    overlap: Overlap,
    threads: usize,
) -> MemberRun {
    let graph = branch_graph(
        stage,
        family,
        ThreadClass {
            threads,
            order,
            overlap,
        },
    );
    let class = class_of(&graph.merge_values, threads);
    let hash = graph_hash(&graph);
    // Whoever reaches a graph first draws it; which member that is does not change the set.
    let realization = claims.claim(hash).then(|| realization(stage, &graph, 0));
    MemberRun {
        family,
        order,
        overlap,
        threads,
        class,
        hash,
        realization,
    }
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

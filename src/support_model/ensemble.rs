//! The ensemble of realizations the slicer can print for one object (design §5.4): every
//! thread class of the merge bucket rule (nondeterminism S2, spec 02 §4.7) for every arc
//! family of the line connector (S9, spec 01 §5.5). The classes are found exactly: a run at
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
use super::graph::branch_graph;
use super::merge::{MergeOrder, ThreadClass};
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
    /// A representative thread count of the class.
    pub threads: usize,
    /// The thread counts (a, b] that give this very run; `None` = unbounded.
    pub class: (usize, Option<usize>),
    pub realization: Realization,
    /// Hash of the realization's footprints at 1 µm: equal hashes are one realization.
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

/// Every member of the core ensemble, in (family, thread count) order, duplicates kept (the
/// caller dedups by `hash`).
pub fn run_members(stage: &StageA) -> Vec<MemberRun> {
    warm_volumes(stage);
    let axes: Vec<(ArcFamily, MergeOrder)> = families(stage)
        .into_iter()
        .flat_map(|family| [MergeOrder::Along, MergeOrder::Against].map(|order| (family, order)))
        .collect();
    axes.into_par_iter()
        .flat_map(|(family, order)| walk_classes(stage, family, order))
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

/// The class walk of one (family, order).
fn walk_classes(stage: &StageA, family: ArcFamily, order: MergeOrder) -> Vec<MemberRun> {
    let fixed = stage.flavour.merge_bucket() == MergeBucket::FixedFour;
    let mut runs: Vec<MemberRun> = Vec::new();
    let mut next = 1;
    let mut batch = if fixed { 1 } else { FIRST_BATCH };
    loop {
        let fresh: Vec<MemberRun> = (next..next + batch)
            .into_par_iter()
            .map(|threads| run_one(stage, family, order, threads))
            .collect();
        runs.extend(fresh);
        runs.sort_by_key(|run| run.threads);
        let mut covered = 1;
        let mut finished = fixed;
        let mut kept: Vec<MemberRun> = Vec::new();
        for run in std::mem::take(&mut runs) {
            // A run inside an earlier run's class repeats it (same decisions).
            if run.threads < covered {
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

fn run_one(stage: &StageA, family: ArcFamily, order: MergeOrder, threads: usize) -> MemberRun {
    let graph = branch_graph(stage, family, ThreadClass { threads, order });
    let class = class_of(&graph.merge_values, threads);
    let member = realization(stage, &graph, threads as u32);
    MemberRun {
        family,
        order,
        threads,
        class,
        hash: hash_of(&member),
        realization: member,
    }
}

/// The thread counts (a, b] that decide like `threads` on every merge (spec 02 §4.7).
pub fn class_of(values: &[usize], threads: usize) -> (usize, Option<usize>) {
    let below = values.iter().copied().filter(|&m| m < threads).max();
    let above = values.iter().copied().filter(|&m| m >= threads).min();
    (below.unwrap_or(0), above)
}

/// FNV-1a over the loops of the realization rounded to 1 µm.
fn hash_of(member: &Realization) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut feed = |value: i64| {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    let kinds = [&member.above_base, &member.support, &member.interface];
    for loops in std::iter::once(&member.base).chain(kinds.iter().flat_map(|bands| bands.iter())) {
        feed(-1);
        for ring in loops {
            feed(-2);
            for point in ring {
                feed(libm::round(point.x * 1000.0) as i64);
                feed(libm::round(point.y * 1000.0) as i64);
            }
        }
    }
    hash
}

/// The distinct realizations of `runs`, first of each hash, in run order.
pub fn distinct(runs: &[MemberRun]) -> Vec<&MemberRun> {
    let mut seen = std::collections::BTreeSet::new();
    runs.iter().filter(|run| seen.insert(run.hash)).collect()
}

/// Serializable summary for reports.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MemberSummary {
    pub family: String,
    pub order: String,
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

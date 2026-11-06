//! Stage B2: influence areas layer by layer downward (spec 02 §3). Every node of layer l
//! steps down to layer l − 1 (`evaluate`, with `speeds`, the stepped offset `soi` and the
//! trial order of `settings`); the nodes that can still meet are merged (`merge`); the
//! survivors join the tips already seeded on that layer. Nodes that cannot step down end
//! their branch, which removes everything above them at placement.

pub(crate) mod evaluate;
mod settings;
pub(crate) mod soi;
mod speeds;

use super::flavour::MergeBucket;
use super::memo::Fingerprint;
use super::merge::{ThreadClass, merge_layer};
use super::node::Node;
use super::stage_a::StageA;
use crate::polygon_ops::{IntBox, Rings};

/// The memo kind of a step.
const STEP_KEY: u8 = 1;

/// A node after its move: the node (influence area in `node.area`), whether it skips the
/// merge, the part of its area from which the bed is reachable (P_bp) and the box around it.
#[derive(Clone, Debug, PartialEq)]
pub struct Moved {
    pub node: Node,
    pub bypass: bool,
    pub plate: Rings,
    pub bounds: Option<IntBox>,
}

/// The graph after descent, bottom layer first, and the merge decisions the run made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Descended {
    pub layers: Vec<Vec<Node>>,
    /// m = ⌊(n + 2) / 4⌋ of every merge call: a member's thread class is exact if it makes
    /// the same decision `m ≥ T` on all of them (spec 02 §4.7).
    pub merge_values: Vec<usize>,
    /// Seconds spent moving nodes down and merging (diagnostics).
    pub move_seconds: f64,
    pub merge_seconds: f64,
}

/// Runs the descent from the highest tip layer down to layer 1.
pub fn descend(stage: &StageA, layers: Vec<Vec<Node>>, class: ThreadClass) -> Descended {
    let mut run = start(layers);
    while let Some(step) = run.step_down(stage) {
        run.merge(stage, step, class);
    }
    run.descended
}

/// One thread class of a class walk: the thread counts [`low`, `high`] all make the same
/// decisions (`high` = `None`: unbounded), and its descent.
pub struct ClassDescent {
    pub low: usize,
    pub high: Option<usize>,
    pub descended: Descended,
}

/// Every thread class of the bucket rule for one (order, overlap), as a tree of descents.
/// The thread count enters only the bucket size of a merge (4 when m ≥ T, else 2), so the
/// runs of two classes are the same down to the first merge their decisions differ on: the
/// walk splits the interval of thread counts there and both halves go on from the shared
/// state (in parallel), instead of each class descending from the top again. The leaves are
/// the classes of `ensemble::class_of`, lowest thread counts first, and each descent equals
/// `descend` at any thread count of its class.
pub fn descend_classes<T: Send>(
    stage: &StageA,
    layers: Vec<Vec<Node>>,
    class: ThreadClass,
    leaf: &(dyn Fn(ClassDescent) -> T + Sync),
) -> Vec<T> {
    walk(stage, start(layers), None, 1, None, class, leaf)
}

fn walk<T: Send>(
    stage: &StageA,
    mut run: Run,
    mut pending: Option<Step>,
    low: usize,
    high: Option<usize>,
    class: ThreadClass,
    leaf: &(dyn Fn(ClassDescent) -> T + Sync),
) -> Vec<T> {
    let class = ThreadClass {
        threads: low,
        ..class
    };
    while let Some(step) = pending.take().or_else(|| run.step_down(stage)) {
        if let Some(value) = step.decision_value()
            && stage.flavour.merge_bucket() == MergeBucket::ThreadCount
            && low <= value
            && high.is_none_or(|high| value < high)
        {
            // [low, value] merges in fours, [value + 1, high] in twos.
            let (run_twos, step_twos) = (run.clone(), step.clone());
            let (mut fours, twos) = rayon::join(
                || walk(stage, run, Some(step), low, Some(value), class, leaf),
                || walk(stage, run_twos, Some(step_twos), value + 1, high, class, leaf),
            );
            fours.extend(twos);
            return fours;
        }
        run.merge(stage, step, class);
    }
    vec![leaf(ClassDescent {
        low,
        high,
        descended: run.descended,
    })]
}

/// A descent in progress: the layers below `layer` still hold only their tips.
#[derive(Clone)]
struct Run {
    descended: Descended,
    layer: usize,
}

/// The moved nodes of one step, before the merge.
#[derive(Clone)]
struct Step {
    direct: Vec<Node>,
    meeting: Vec<Moved>,
}

impl Step {
    /// m of the merge this step makes, `None` without one (see `merge_layer`).
    fn decision_value(&self) -> Option<usize> {
        (self.meeting.len() >= 2).then(|| ThreadClass::decision_value(self.meeting.len()))
    }
}

fn start(layers: Vec<Vec<Node>>) -> Run {
    let top = layers.iter().rposition(|layer| !layer.is_empty());
    Run {
        descended: Descended {
            layers,
            ..Descended::default()
        },
        // Layer 0 has no step below it: an empty graph makes none.
        layer: top.unwrap_or(0),
    }
}

impl Run {
    /// Moves every node of the current layer one layer down (`None` once at layer 0).
    fn step_down(&mut self, stage: &StageA) -> Option<Step> {
        let layer = self.layer;
        if layer == 0 {
            return None;
        }
        let started = std::time::Instant::now();
        let layers = &mut self.descended.layers;
        let mut direct: Vec<Node> = Vec::new();
        let mut meeting: Vec<Moved> = Vec::new();
        for index in 0..layers[layer].len() {
            let parent = layers[layer][index].clone();
            let key = Fingerprint::new(STEP_KEY)
                .int(layer as i64)
                .node(&parent)
                .finish();
            // The step does not read `parents`; they are set here.
            let step = stage
                .memo
                .steps
                .get_or(key, || evaluate::step_down(stage, &parent, layer));
            match step {
                Some(mut moved) => {
                    moved.node.parents = vec![index];
                    if moved.bypass || moved.plate.is_empty() {
                        direct.push(moved.node);
                    } else {
                        meeting.push(moved);
                    }
                }
                None => {
                    // A dead end: the branch above it is deleted at placement.
                    layers[layer][index].centre = None;
                    layers[layer][index].graceful = false;
                }
            }
        }
        self.descended.move_seconds += started.elapsed().as_secs_f64();
        Some(Step { direct, meeting })
    }

    /// Merges the step's candidates into the layer below and moves there.
    fn merge(&mut self, stage: &StageA, step: Step, class: ThreadClass) {
        let below = self.layer - 1;
        let started = std::time::Instant::now();
        let (merged, value) = merge_layer(stage, below, step.meeting, class);
        let descended = &mut self.descended;
        descended.merge_seconds += started.elapsed().as_secs_f64();
        descended.merge_values.extend(value);
        descended.layers[below].extend(step.direct);
        descended.layers[below].extend(merged.into_iter().map(|moved| moved.node));
        self.layer = below;
    }
}

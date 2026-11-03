//! Stage B2: influence areas layer by layer downward (spec 02 §3). Every node of layer l
//! steps down to layer l − 1 (`evaluate`, with `speeds`, the stepped offset `soi` and the
//! trial order of `settings`); the nodes that can still meet are merged (`merge`); the
//! survivors join the tips already seeded on that layer. Nodes that cannot step down end
//! their branch, which removes everything above them at placement.

pub(crate) mod evaluate;
mod settings;
pub(crate) mod soi;
mod speeds;

use super::merge::{ThreadClass, merge_layer};
use super::node::Node;
use super::stage_a::StageA;
use crate::polygon_ops::{IntBox, Rings};

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
}

/// Runs the descent from the highest tip layer down to layer 1.
pub fn descend(stage: &StageA, mut layers: Vec<Vec<Node>>, class: ThreadClass) -> Descended {
    let mut merge_values = Vec::new();
    let Some(top) = layers.iter().rposition(|layer| !layer.is_empty()) else {
        return Descended {
            layers,
            merge_values,
        };
    };
    for layer in (1..=top).rev() {
        let mut direct: Vec<Node> = Vec::new();
        let mut meeting: Vec<Moved> = Vec::new();
        for index in 0..layers[layer].len() {
            let parent = layers[layer][index].clone();
            match evaluate::step_down(stage, &parent, layer) {
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
        let (merged, value) = merge_layer(stage, layer - 1, meeting, class);
        merge_values.extend(value);
        layers[layer - 1].extend(direct);
        layers[layer - 1].extend(merged.into_iter().map(|moved| moved.node));
    }
    Descended {
        layers,
        merge_values,
    }
}

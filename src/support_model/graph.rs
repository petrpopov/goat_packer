//! The branch graph of one ensemble member: tips seeded with an arc family, the descent and
//! merges under a thread class, and placed nodes (stages B1-B4). Deterministic and
//! single-threaded: the member's identity is (family, thread class).

use super::descend::descend;
use super::merge::ThreadClass;
use super::node::Node;
use super::place::place;
use super::stage_a::StageA;
use super::tips::{ArcFamily, RoofPiece, seed_tips};

/// Placed branch nodes per layer (index = layer, parents index into the layer above) and the
/// roof polygons the tips stand under.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BranchGraph {
    pub layers: Vec<Vec<Node>>,
    pub roofs: Vec<RoofPiece>,
    /// The merge decision values of the run (see `Descended::merge_values`).
    pub merge_values: Vec<usize>,
}

/// B1-B4 for one member.
pub fn branch_graph(stage: &StageA, family: ArcFamily, class: ThreadClass) -> BranchGraph {
    let seeds = seed_tips(stage, family);
    let descended = descend(stage, seeds.tips, class);
    BranchGraph {
        layers: place(descended.layers),
        roofs: seeds.roofs,
        merge_values: descended.merge_values,
    }
}

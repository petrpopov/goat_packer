//! Stage A4: overhang detection (spec 01 §3), the polygons the tree generator grows tips
//! from. One pipeline with the flavour switches of spec 01 §7:
//!
//! 1. `pass`: per layer, the plain overhang P_k = X_k \ (X_{k−1} ⊕□ d_k) (B: minus grown
//!    blockers, plus the enforced overhang), sharp-tail seeds and cantilevers (§3.1-§3.3);
//! 2. `tails`: sharp tails grown bottom-up and the every-0.5-mm-ish tail supports (§3.4, §3.7);
//! 3. `clusters`: overhang islands grouped across layers, small clusters removed (§3.5-§3.6);
//! 4. `assemble`: the per-layer typed overhangs H_k with the critical-only rule and the
//!    hand-off polygons (sharp tails grown 0.2 mm, vertical enforcer discs; §3.7-§3.8).
//!
//! Upstream runs step 1 in parallel; the only shared state is the guard of §3.9 (S3). Its
//! island trigger is evaluated exactly and returns `Unsupported` (design §5.7); its clock
//! trigger is not modelled. Bridge removal (`max_bridge_length` > 0, §3.7.1) is not modelled
//! and is reported (evidence of no effect on the goat: U1, spec 01 §3.7.1).

mod assemble;
mod clusters;
mod context;
mod pass;
mod tails;

use serde::{Deserialize, Serialize};

use super::SupportError;
use super::flavour::Flavour;
use super::layers::LayerGrid;
use super::sections::LayerSections;
use super::settings::SupportSettings;
use crate::polygon_ops::{Island, Rings};

pub use context::PaintInput;

/// The guard of spec 01 §3.9 trips when one layer's overhang has more islands than this.
pub const GUARD_ISLANDS: usize = 100;

/// Why a polygon is an overhang; several may hold (the slicers keep a bit set).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct OverhangFlags {
    pub cantilever: bool,
    pub sharp_tail: bool,
    /// The lowest piece of a sharp tail (accumulated height 0; B only).
    pub lowest_tail: bool,
    pub small: bool,
    /// B: cluster outline above 100 mm² (only matters for critical-only at enforcer layers).
    pub big_flat: bool,
    /// Added from painted or volume enforcers.
    pub enforced: bool,
}

/// One overhang island of H_k with its type.
#[derive(Clone, Debug, PartialEq)]
pub struct TypedOverhang {
    pub island: Island,
    pub flags: OverhangFlags,
}

/// What detection found, per layer of the grid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overhangs {
    /// H_k: typed overhang islands per layer (spec 01 §3.7), in emission order.
    pub layers: Vec<Vec<TypedOverhang>>,
    /// What the tree generator receives per layer (§3.8): H_k's rings, sharp-tail ones grown
    /// by 0.2 mm, plus the vertical-enforcer discs. Layers 0-2 are never used for tips.
    pub handoff: Vec<Rings>,
    pub stats: OverhangStats,
}

/// Diagnostics of one detection run (spec 01 §9 A4 hooks).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct OverhangStats {
    /// Islands of the plain overhang per layer (the guard's count).
    pub islands: Vec<usize>,
    pub cantilevers: Vec<usize>,
    /// Sharp-tail islands per layer (seeds and grown tails).
    pub tails: Vec<usize>,
    pub clusters: usize,
    pub small_clusters: usize,
    /// `max_bridge_length` > 0: bridged areas were not removed (§3.7.1).
    pub bridge_removal_skipped: bool,
}

impl Overhangs {
    /// The highest layer with an overhang, if any.
    pub fn highest_layer(&self) -> Option<usize> {
        self.layers.iter().rposition(|layer| !layer.is_empty())
    }
}

/// Detects the overhangs of one object (stage A4).
pub fn detect_overhangs(
    grid: &LayerGrid,
    sections: &LayerSections,
    paint: &PaintInput,
    settings: &SupportSettings,
    flavour: Flavour,
) -> Result<Overhangs, SupportError> {
    let count = sections.extrudable.len();
    if !settings.enable_support || count == 0 {
        return Ok(Overhangs {
            layers: vec![Vec::new(); count],
            handoff: vec![Rings::new(); count],
            ..Overhangs::default()
        });
    }
    let context = context::Context::new(grid, sections, paint, settings, flavour);
    let passes = pass::run(&context)?;
    let tails = tails::grow(&context, &passes);
    let clustered = clusters::group(&context, &passes, &tails);
    let layers = assemble::typed_layers(&context, &passes, &tails, &clustered);
    let handoff = assemble::handoff(&context, &layers);
    let stats = OverhangStats {
        islands: passes.iter().map(|layer| layer.overhangs.len()).collect(),
        cantilevers: passes.iter().map(|layer| layer.cantilevers.len()).collect(),
        tails: tails.iter().map(Vec::len).collect(),
        clusters: clustered.clusters.len(),
        small_clusters: clustered
            .clusters
            .iter()
            .filter(|cluster| cluster.small)
            .count(),
        bridge_removal_skipped: settings.max_bridge_length > 0.0,
    };
    Ok(Overhangs {
        layers,
        handoff,
        stats,
    })
}

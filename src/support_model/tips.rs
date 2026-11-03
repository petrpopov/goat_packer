//! Stage B1: tip seeding (spec 01 §5). Every overhang layer k with hand-off polygons is a
//! task that seeds tips on layer m = k − z_delta (and up to R layers lower under roofs) and
//! records the roof polygons: `areas` splits the overhang into roof candidates and the
//! outset fringe, `stack` samples one area (roof stack, line pattern, resampling), `lines`
//! and `resample` are the geometry helpers, `classify` decides whether a point can be a tip
//! and builds it.
//!
//! The tip list of a layer is the order the merge stage sees (spec 02 §4.2): tasks run in
//! ascending k, each task's tips in full before the next (the one-thread order, canonical
//! ensemble order), the first tip in a 40 µm dedup cell wins.

mod areas;
mod classify;
pub mod lines;
pub mod resample;
mod stack;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use super::clearance::Avoidance;
use super::node::Node;
use super::stage_a::StageA;
use crate::polygon_ops::{FillRule, Join, Rings, offset, resolve};

/// The two families of the support-base line connector's process-static arc thresholds
/// (spec 01 §5.5 item 4, nondeterminism S9): initialised from the roof spacing or from the
/// base spacing of whichever fill ran first; Orca 2.5 evaluates them per call.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ArcFamily {
    Roof,
    Base,
    PerCall,
}

/// A roof (support interface) polygon of one layer; `interface` = below the contact layer.
#[derive(Clone, Debug, PartialEq)]
pub struct RoofPiece {
    pub layer: usize,
    pub interface: bool,
    pub rings: Rings,
}

/// Tip lists per layer and roof pieces, the input of the descent (spec 01 §5.8).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seeds {
    pub tips: Vec<Vec<Node>>,
    pub roofs: Vec<RoofPiece>,
}

/// The 5 µm that keeps boundary points out of the forbidden region (spec 01 §5.2).
const FORBIDDEN_MARGIN_MM: f64 = 0.005;

/// Mitre limit the avoidance queries ask for (acts as 2).
const QUERY_MITER_LIMIT: f64 = 1.2;

/// Shared by the tasks of one member.
pub(crate) struct Context<'a> {
    pub stage: &'a StageA,
    pub family: ArcFamily,
    forbidden: RefCell<BTreeMap<usize, Rings>>,
    safe: RefCell<BTreeMap<usize, Rings>>,
}

impl<'a> Context<'a> {
    fn new(stage: &'a StageA, family: ArcFamily) -> Context<'a> {
        Context {
            stage,
            family,
            forbidden: RefCell::new(BTreeMap::new()),
            safe: RefCell::new(BTreeMap::new()),
        }
    }

    /// F_l = AvoidFast(r_min, l) grown by 5 µm.
    pub fn forbidden(&self, layer: usize) -> Rings {
        let mut cache = self.forbidden.borrow_mut();
        cache
            .entry(layer)
            .or_insert_with(|| {
                let derived = &self.stage.derived;
                let avoid = self.stage.clearance.avoidance(
                    derived.tip_radius,
                    layer,
                    Avoidance::Fast,
                    true,
                );
                let margin = derived.grid.scaled(FORBIDDEN_MARGIN_MM);
                offset(&avoid, margin, Join::Miter(QUERY_MITER_LIMIT))
            })
            .clone()
    }

    /// AvoidSafe(r_min, l): where a tip could get trapped above a hole.
    pub fn safe_avoidance(&self, layer: usize) -> Rings {
        let mut cache = self.safe.borrow_mut();
        cache
            .entry(layer)
            .or_insert_with(|| {
                let derived = &self.stage.derived;
                self.stage
                    .clearance
                    .avoidance(derived.tip_radius, layer, Avoidance::FastSafe, true)
                    .as_ref()
                    .clone()
            })
            .clone()
    }

    pub fn fast_avoidance(&self, layer: usize) -> Rings {
        let derived = &self.stage.derived;
        self.stage
            .clearance
            .avoidance(derived.tip_radius, layer, Avoidance::Fast, true)
            .as_ref()
            .clone()
    }
}

/// Seeds the tips and roofs of one ensemble member (arc family `family`).
pub fn seed_tips(stage: &StageA, family: ArcFamily) -> Seeds {
    let layers = stage.grid.len();
    let context = Context::new(stage, family);
    let mut seeds = Seeds {
        tips: vec![Vec::new(); layers],
        ..Seeds::default()
    };
    let mut cells: Vec<BTreeSet<(i64, i64)>> = vec![BTreeSet::new(); layers];
    let delta = stage.derived.z_delta;
    for (k, handoff) in stage.overhangs.handoff.iter().enumerate() {
        let Some(m) = k.checked_sub(delta) else {
            continue;
        };
        if handoff.is_empty() || m < 1 || m + 3 > layers {
            continue;
        }
        let overhang = resolve(handoff, FillRule::NonZero);
        let task = areas::task_areas(&context, m, &overhang);
        for candidate in task {
            let sampled = stack::sample(&context, &candidate.island, m, candidate.roof);
            seeds.roofs.extend(sampled.roofs);
            for tip in sampled.tips {
                let cell = dedup_cell(tip.hint, stage.derived.tip_dedup_cell);
                if cells[tip.layer].insert(cell) {
                    seeds.tips[tip.layer].push(tip);
                }
            }
        }
    }
    seeds
}

/// The 40 µm cell of the first-come dedup; truncation toward zero makes the cells that
/// straddle an axis twice as wide (spec 01 §5.7).
fn dedup_cell(point: crate::polygon_ops::IntPoint, cell: i64) -> (i64, i64) {
    (i64::from(point.x) / cell, i64::from(point.y) / cell)
}

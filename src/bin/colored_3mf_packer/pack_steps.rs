//! The packing stage in sub-steps for the progress events (`progress::StageDetail`): every
//! foreign-gap rung at the coarse resolution, then the finer levels of the ladder for the
//! rungs `gaps::refine_rungs` picks, one level per step. The webapp shows them as one
//! «Раскладка» stage with a live sub-label and a bar moving by steps.

use goat_packer::{ForeignGaps, pack::PackOutcome, progress::StageDetail};

use crate::args::Args;
use crate::plan::PackPlan;

/// Most rungs that climb the finer levels (`gaps::refine_rungs`).
const REFINE_SLOTS: usize = 2;

/// The resolution ladder of `args`, coarse first, each level once (as `pack::ladder` reads it).
pub fn ladder(args: &Args) -> Vec<f64> {
    let mut levels: Vec<f64> = Vec::new();
    for &resolution in &args.resolutions {
        if !levels.contains(&resolution) {
            levels.push(resolution);
        }
    }
    if levels.is_empty() {
        levels.push(args.resolution);
    }
    levels
}

/// Counts the packing sub-steps: per clip configuration, one coarse pack per rung plus the
/// finer levels of at most `REFINE_SLOTS` rungs. A configuration that refines fewer rungs
/// leaves its steps unused (the bar jumps), so the total is an upper bound.
#[derive(Clone, Debug)]
pub struct PackSteps {
    configurations: usize,
    rungs: usize,
    levels: Vec<f64>,
    /// The clip configuration being packed (0-based).
    pub configuration: usize,
}

impl PackSteps {
    pub fn new(configurations: usize, rungs: usize, levels: Vec<f64>) -> PackSteps {
        PackSteps {
            configurations: configurations.max(1),
            rungs: rungs.max(1),
            levels,
            configuration: 0,
        }
    }

    fn finer(&self) -> usize {
        self.levels.len().saturating_sub(1)
    }

    fn per_configuration(&self) -> usize {
        self.rungs + self.rungs.min(REFINE_SLOTS) * self.finer()
    }

    pub fn total(&self) -> usize {
        self.configurations * self.per_configuration()
    }

    fn detail(
        &self,
        step: usize,
        phase: &str,
        rung: usize,
        level: usize,
        resolution_mm: f64,
        foreign: ForeignGaps,
    ) -> StageDetail {
        StageDetail::new(
            self.configuration * self.per_configuration() + step,
            self.total(),
        )
        .with("phase", phase)
        .with("variant", self.configuration * self.rungs + rung + 1)
        .with("variants", self.configurations * self.rungs)
        .with("base_mm", foreign.base_mm)
        .with("band_mm", foreign.band_mm)
        .with("resolution_mm", resolution_mm)
        .with("level", level + 1)
        .with("levels", self.levels.len())
    }

    /// The coarse pack of `rung` (0-based) at the ladder's first `resolution_mm`.
    pub fn coarse(&self, rung: usize, resolution_mm: f64, foreign: ForeignGaps) -> StageDetail {
        self.detail(rung, "coarse", rung, 0, resolution_mm, foreign)
    }

    /// Level `level` (1-based within the ladder, ≥ 1) of the `slot`-th refined rung.
    pub fn refine(
        &self,
        rung: usize,
        slot: usize,
        level: usize,
        resolution_mm: f64,
        foreign: ForeignGaps,
    ) -> StageDetail {
        let step = self.rungs + slot.min(REFINE_SLOTS - 1) * self.finer() + level.saturating_sub(1);
        self.detail(step, "refine", rung, level, resolution_mm, foreign)
    }
}

/// Climbs the finer levels of the ladder from `coarse` one level at a time, announcing each
/// (`announce(level, resolution_mm)`, level 1-based past the coarse one). Each call continues
/// from the outcome of the levels before it, which `pack::refine` makes equal to one call over
/// the whole ladder (a seeded run skips the levels its seed ran). A failing level keeps the
/// layout so far, as the single call kept the coarse one.
pub fn refine_by_level(
    args: &Args,
    plan: &PackPlan,
    coarse: PackOutcome,
    mut announce: impl FnMut(usize, f64),
) -> PackOutcome {
    let levels = ladder(args);
    let mut outcome = coarse;
    for level in 1..levels.len() {
        announce(level, levels[level]);
        let upto = Args {
            resolutions: levels[..=level].to_vec(),
            ..args.clone()
        };
        match plan.refine(&upto, outcome.clone()) {
            Ok(refined) => outcome = refined,
            Err(error) => {
                eprintln!(
                    "finer resolution {} mm failed, the layout so far kept: {error}",
                    levels[level]
                );
                break;
            }
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gaps() -> ForeignGaps {
        ForeignGaps {
            base_mm: 0.0,
            band_mm: 0.5,
        }
    }

    #[test]
    fn steps_count_every_coarse_rung_and_two_refined_rungs_per_configuration() {
        let steps = PackSteps::new(1, 4, vec![0.25, 0.2, 0.15]);
        assert_eq!(steps.total(), 4 + 2 * 2);
        let coarse = steps.coarse(2, 0.25, gaps());
        assert_eq!((coarse.step, coarse.steps), (2, 8));
        assert_eq!(coarse.fields["phase"], "coarse");
        assert_eq!(coarse.fields["variant"], 3);
        assert_eq!(coarse.fields["variants"], 4);
        assert_eq!(coarse.fields["level"], 1);
        let refine = steps.refine(3, 1, 2, 0.15, gaps());
        assert_eq!(
            (refine.step, refine.fields["level"].as_u64()),
            (4 + 2 + 1, Some(3))
        );
        assert_eq!(refine.fields["phase"], "refine");
        assert_eq!(refine.fields["resolution_mm"], 0.15);
    }

    #[test]
    fn a_floor_only_ladder_counts_one_rung_and_its_levels() {
        let steps = PackSteps::new(1, 1, vec![0.25, 0.2]);
        assert_eq!(steps.total(), 2);
        assert_eq!(steps.coarse(0, 0.25, gaps()).step, 0);
        assert_eq!(steps.refine(0, 0, 1, 0.2, gaps()).step, 1);
    }

    #[test]
    fn clip_configurations_continue_the_count() {
        let mut steps = PackSteps::new(5, 1, vec![0.25, 0.2]);
        steps.configuration = 3;
        let detail = steps.coarse(0, 0.25, gaps());
        assert_eq!((detail.step, detail.steps), (6, 10));
        assert_eq!(
            (
                detail.fields["variant"].as_u64(),
                detail.fields["variants"].as_u64()
            ),
            (Some(4), Some(5))
        );
    }
}

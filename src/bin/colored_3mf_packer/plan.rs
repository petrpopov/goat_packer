//! What a layout is packed against, one packing pass, and the state a packed plate leaves
//! behind for `--finish-state`.

use goat_packer::{
    ForeignGaps, PackOptions, PlateRules, Point, PrintPlacement,
    object_print::ObjectPrint,
    pack::{LevelReport, PackOutcome, TowerSpec, pack, refine},
    print_time::TimeSample,
    keepout::clip::check_first_copies, validate_vector, LayoutRules,
};
use serde::{Deserialize, Serialize};

use crate::AppResult;
use crate::args::Args;
use crate::gaps::GapVariant;
use crate::model::ClassSummary;
use crate::order::Canonical;

/// The state file format. Version 1 held a slicer-learned `Footprint`; its plan, support rule
/// and sample slice no longer exist.
pub const STATE_VERSION: u32 = 2;

/// Everything one packing pass depends on; the verify loop widens it between passes.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackPlan {
    /// One print per rotation class, class index = position.
    pub prints: Vec<ObjectPrint>,
    pub rules: PlateRules,
    pub tower: Option<TowerSpec>,
    /// Support-to-neighbour-body gaps this plan packs and validates with. A state written
    /// before they were options loads with the former fixed values.
    #[serde(default)]
    pub foreign_gaps: ForeignGaps,
}

/// What `--finish-state` reads back: the plan, the layout in 3MF order and what the time
/// window needs again.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackState {
    pub version: u32,
    pub plan: PackPlan,
    /// In object order of the written 3MF: canonical copies first, centre-out on window plates.
    pub placements: Vec<PrintPlacement>,
    pub tower_anchor: Option<Point>,
    pub centre: Option<(f64, f64)>,
    pub probe_seconds: f64,
    pub probe_sample: Option<TimeSample>,
    pub summary: PackSummary,
}

fn probe_source() -> String {
    "probe".to_owned()
}

/// What the run found besides the layout, for the report.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PackSummary {
    pub probes: Vec<ClassSummary>,
    /// Seconds the support model took (0 with probes).
    #[serde(default)]
    pub model_seconds: f64,
    /// Where the supports of the plan came from (`probe` or `model`): kept with the plan so a
    /// resumed `--finish-state` run reports the plate's own source, not its own arguments'.
    #[serde(default = "probe_source")]
    pub support_source: String,
    /// The packing member that won and the best count each member reached.
    pub member: String,
    pub member_counts: Vec<MemberResult>,
    pub canonical: Vec<Canonical>,
    /// Every (own gap, foreign pair) variant packed, in packing order.
    #[serde(default)]
    pub gap_variants: Vec<GapVariant>,
    /// The chosen layout's resolution (mm), the upper bound at it (any mix of classes, then
    /// per class) and every ladder level that produced it.
    #[serde(default)]
    pub resolution_mm: f64,
    #[serde(default)]
    pub upper_bound: Option<usize>,
    #[serde(default)]
    pub class_bounds: Vec<Option<usize>>,
    #[serde(default)]
    pub levels: Vec<LevelReport>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MemberResult {
    pub member: String,
    pub count: usize,
}

impl PackState {
    /// Parses a state file, naming the problem for one that predates version 2.
    pub fn from_json(bytes: &[u8]) -> AppResult<PackState> {
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        let version = value.get("version").and_then(serde_json::Value::as_u64);
        if version != Some(u64::from(STATE_VERSION)) {
            return Err(format!(
                "pack state version {version:?} is not readable: only version {STATE_VERSION} \
                 (probe-based supports) is; pack the plate again"
            )
            .into());
        }
        Ok(serde_json::from_value(value)?)
    }
}

impl PackPlan {
    /// The same prints packed against another foreign-gap pair: only the masks change.
    pub fn with_foreign_gaps(&self, foreign_gaps: ForeignGaps) -> PackPlan {
        PackPlan { foreign_gaps, ..self.clone() }
    }

    /// The packing options of a run. The bed fields belong to the legacy search; the plate
    /// comes from `rules`.
    pub fn options(&self, args: &Args) -> PackOptions {
        let printable = self.rules.printable_area;
        PackOptions {
            bed: printable,
            body_bed: None,
            exclusions: Vec::new(),
            body_exclusions: Vec::new(),
            resolution: args.resolution,
            resolutions: args.resolutions.clone(),
            clearance: args.clearance,
            angle_step_deg: args.angle_step,
            attempts: args.attempts,
            candidates_per_step: args.candidates,
            stall_attempts: args.stall,
            max_objects: args.max_objects,
            lock_orientation: false,
            seed: args.seed,
            support_overlap_mm: args.support_overlap_mm,
            foreign_gaps: self.foreign_gaps,
            plate_rules: Some(self.rules.clone()),
        }
    }

    /// One packing pass of the portfolio over every class.
    pub fn pack(&self, args: &Args) -> AppResult<PackOutcome> {
        Ok(pack(&self.prints, &self.rules, self.tower.as_ref(), &self.options(args))?)
    }

    /// The first level of the resolution ladder only: the pass over every foreign-gap rung.
    pub fn pack_coarse(&self, args: &Args) -> AppResult<PackOutcome> {
        let coarse = Args {
            resolutions: vec![args.resolution],
            ..args.clone()
        };
        self.pack(&coarse)
    }

    /// Climbs the rest of the ladder from a `pack_coarse` outcome of this plan.
    pub fn refine(&self, args: &Args, coarse: PackOutcome) -> AppResult<PackOutcome> {
        Ok(refine(&self.prints, &self.rules, self.tower.as_ref(), &self.options(args), coarse)?)
    }

    /// The exact check of a layout, the gate every plate passes before it is written: the
    /// plate's zones plus the tower at `anchor` against every copy.
    pub fn validate(
        &self,
        args: &Args,
        placements: &[PrintPlacement],
        anchor: Option<Point>,
    ) -> AppResult<()> {
        let mut rules = LayoutRules::new(
            &self.rules,
            None,
            args.clearance,
            self.foreign_gaps,
            args.support_overlap_mm,
        )?;
        if let (Some(spec), Some(anchor)) = (&self.tower, anchor) {
            rules.zones.extend(spec.zones(anchor));
        }
        validate_vector(&self.prints, placements, &rules)?;
        let order: Vec<(usize, f64, f64)> =
            placements.iter().map(|p| (p.print, p.x_mm, p.y_mm)).collect();
        check_first_copies(
            &self.prints,
            &order,
            self.rules.printable_area,
            self.rules.support_clip_mm,
        )?;
        Ok(())
    }

    /// The hard zones' barrier list for the canonical-copy order: the plate and the tower.
    pub fn zones(&self, anchor: Option<Point>) -> AppResult<Vec<goat_packer::kind_masks::PlateZone>> {
        let mut zones = goat_packer::kind_masks::plate_zones(&self.rules, None)?;
        if let (Some(spec), Some(anchor)) = (&self.tower, anchor) {
            zones.extend(spec.zones(anchor));
        }
        Ok(zones)
    }
}

/// The plan of a job's modelled classes and tower.
pub fn plan_of(job: &crate::job::Job, prints: Vec<ObjectPrint>, tower: Option<TowerSpec>) -> PackPlan {
    PackPlan {
        prints,
        rules: job.rules.clone(),
        tower,
        foreign_gaps: ForeignGaps::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{test_job, test_plan, test_state};

    #[test]
    fn a_state_of_another_version_is_refused_with_the_way_out() {
        let error = PackState::from_json(br#"{"plan": {}, "packed": {}}"#).unwrap_err().to_string();
        assert!(error.contains("pack the plate again"), "{error}");
    }

    #[test]
    fn a_state_written_before_the_gap_options_loads_with_the_former_values() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let mut value = serde_json::to_value(test_state(test_plan(&job, 1))).unwrap();
        let plan = value["plan"].as_object_mut().unwrap();
        plan.remove("foreign_gaps");
        value["summary"].as_object_mut().unwrap().remove("gap_variants");
        let back = PackState::from_json(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(back.plan.foreign_gaps, ForeignGaps::default());
        assert_eq!(back.plan.foreign_gaps.band_mm, 0.5);
        assert!(back.summary.gap_variants.is_empty());
    }

    #[test]
    fn the_foreign_gaps_of_a_plan_reach_the_options_and_the_state_file() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let gaps = ForeignGaps { base_mm: 0.25, band_mm: 0.75 };
        let state = test_state(test_plan(&job, 1).with_foreign_gaps(gaps));
        assert_eq!(state.plan.options(&job.args).foreign_gaps, gaps);
        let back = PackState::from_json(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(back.plan.foreign_gaps, gaps);
    }

    #[test]
    fn a_state_survives_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let mut state = test_state(test_plan(&job, 2));
        let outcome = state.plan.pack(&job.args).unwrap();
        state.placements = outcome.placements;
        let back = PackState::from_json(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(back.placements, state.placements);
        assert_eq!(back.plan.prints, state.plan.prints);
    }

    #[test]
    fn two_classes_fill_a_small_plate_and_pass_the_exact_check() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let plan = test_plan(&job, 2);
        let outcome = plan.pack(&job.args).unwrap();
        // 60 x 60 mm, 10 mm squares with 0.5 mm clearance: at least a 5 x 5 grid.
        assert!(outcome.placements.len() >= 25, "{}", outcome.placements.len());
        assert!(outcome.placements.iter().all(|placement| placement.print < 2));
        plan.validate(&job.args, &outcome.placements, outcome.tower_anchor).unwrap();
    }

    #[test]
    fn a_layout_with_a_doubled_copy_fails_the_exact_check() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let plan = test_plan(&job, 1);
        let mut placements = plan.pack(&job.args).unwrap().placements;
        placements.push(placements[0]);
        assert!(plan.validate(&job.args, &placements, None).is_err());
    }
}

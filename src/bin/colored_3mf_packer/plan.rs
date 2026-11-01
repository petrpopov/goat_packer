//! The plan a layout is packed against, and one packing pass.

use goat_packer::{
    Footprint, PackOptions, PackResult, Rect, colored_packer::{PrimeTowerBounds, PrinterProfile},
    grid_layout, grow_beyond, pack_footprint, plate_centre::RelocatedTower, print_time::TimeSample,
    validate_result,
};
use serde::{Deserialize, Serialize};

use crate::AppResult;
use crate::args::{Args, LayoutMode};
use crate::assembly::build_colored_project;
use crate::job::Job;
use crate::placement::layout_world_bounds;

/// Which pair rule the packer applies between two objects' supports and bodies.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportRule {
    /// A foreign patch must not touch the first-layer body (`support_landing_loops`);
    /// silhouettes keep clearance from each other.
    Landing,
    /// The plain two-zone rule: a foreign patch must not touch the whole silhouette. Used
    /// for support-free footprints and as the fallback when the landing rule fails the
    /// final G-code check twice.
    Silhouette,
}

/// What `--no-final-slice` leaves behind (`<output>.state.json`) and `--finish-state` reads
/// back: the packed plate and everything the final slice's checks and fallbacks need.
#[derive(Deserialize, Serialize)]
pub struct PackState {
    pub plan: PackPlan,
    pub packed: PackResult,
    pub probe_tower: Option<PrimeTowerBounds>,
    pub probe_seconds: f64,
    pub tower_from_contest: bool,
    /// The centre-out target and the probe's time sample (time-window plates).
    pub centre: Option<(f64, f64)>,
    pub probe_sample: Option<TimeSample>,
}

/// Everything one packing pass depends on; the verify loop mutates it between passes
/// (grown tower, learned support patch, strict rule).
#[derive(Deserialize, Serialize)]
pub struct PackPlan {
    pub footprint: Footprint,
    pub lock_orientation: bool,
    /// Bed exclusion corners and purge-line strips: the full-extent bbox must clear them.
    pub fixed_keepouts: Vec<Rect>,
    /// The reserved prime-tower area: only the model body must clear it (supports purge with
    /// the tower area, see `PackOptions::body_exclusions`).
    pub tower_keepout: Option<Rect>,
    /// Set when the tower moves (centre-out plate or a better bed corner): the keepout the
    /// final slice is checked against and the `wipe_tower_x/y` shift.
    pub relocation: Option<RelocatedTower>,
    pub rule: SupportRule,
    pub patch_learned: bool,
}

/// How much farther than the learned patch a later slice may print a support, at the bed
/// edges and exclusion areas (`PackPlan::rule_footprint`). The patch comes from the probe and
/// the 3-object sample; a 44-goat P1S plate packed with its patch flush on the bed edge
/// (x 255.84 of 256) printed every goat's first-layer support base 0.5 mm wider in the full
/// plate's slice, and the base's G3 arcs reached x = 256.34: BambuStudio's GUI refused the
/// plate ("G-code path goes beyond plate boundaries") on the three rightmost goats. The CLI
/// has no such check. Three times the measured drift; bodies keep the plain edge margin.
const SUPPORT_EDGE_RESERVE_MM: f64 = 1.5;

impl PackPlan {
    /// The footprint every layout is packed and validated against: the learned one, plus the
    /// strict rule's empty landing (the landing zone is the whole silhouette again) and the
    /// bed-edge reserve for supports. The reserve lives only here, never in `self.footprint`:
    /// learning a patch unions `bed_loops` into the collision patch, and a grown bed zone must
    /// not leak into object-vs-object clearance.
    pub fn rule_footprint(&self) -> Footprint {
        let mut footprint = self.footprint.clone();
        if self.rule == SupportRule::Silhouette {
            footprint.support_landing_loops.clear();
        }
        if self.lock_orientation && !footprint.body_loops.is_empty() {
            let bed = footprint.bed_loops.clone().unwrap_or_else(|| footprint.loops.clone());
            footprint.bed_loops =
                Some(grow_beyond(&bed, &footprint.body_loops, SUPPORT_EDGE_RESERVE_MM));
        }
        footprint
    }

    fn keepouts(&self) -> Vec<Rect> {
        self.fixed_keepouts.iter().copied().chain(self.tower_keepout).collect()
    }

    pub fn pack(&self, job: &Job) -> AppResult<PackResult> {
        pack(
            &job.args,
            &job.profile,
            &self.rule_footprint(),
            self.lock_orientation,
            &self.fixed_keepouts,
            self.tower_keepout,
            true,
        )
    }

    /// Writes the 3MF for `packed` to the job's output path.
    pub fn build(&self, job: &Job, packed: &PackResult) -> AppResult<()> {
        build_colored_project(
            job,
            &self.footprint,
            packed,
            &self.keepouts(),
            self.relocation.map(|relocated| relocated.shift),
        )
    }
}

/// The packing options of one pass; also what a nudged layout is validated against.
pub fn pack_options(
    args: &Args,
    profile: &PrinterProfile,
    lock_orientation: bool,
    fixed_keepouts: &[Rect],
    tower_keepout: Option<Rect>,
) -> AppResult<PackOptions> {
    // A profile with its own support margin lets the patch use more of the bed than the
    // model: the edge margin then binds the body only (`PackOptions::body_bed`).
    let (bed, body_bed) = match profile.support_edge_margin_mm {
        Some(_) => (profile.support_bed()?, Some(profile.usable_bed()?)),
        None => (profile.usable_bed()?, None),
    };
    Ok(PackOptions {
        body_bed,
        body_exclusions: tower_keepout.into_iter().collect(),
        bed,
        exclusions: fixed_keepouts.to_vec(),
        resolution: args.resolution,
        clearance: args.clearance,
        angle_step_deg: args.angle_step,
        attempts: args.attempts,
        candidates_per_step: args.candidates,
        max_objects: args.max_objects,
        // The probe-derived patch is only valid at the orientation it was sliced at; rotating
        // it would let the slicer's rebuilt tree supports wander outside the reserved area.
        lock_orientation,
        stall_attempts: args.stall,
        seed: args.seed,
        // Only bites when the footprint has a support zone (the locked patch ∪ silhouette
        // branch); a silhouette footprint has body == full and the option is a no-op.
        support_overlap_mm: args.support_overlap_mm,
    })
}

/// One packing pass: the deterministic grid as a floor, then (`--layout search`) the search.
/// `verbose` = log the grid baseline and layout bounds (off for the tower-position contest,
/// whose candidates print one line each).
pub fn pack(
    args: &Args,
    profile: &PrinterProfile,
    footprint: &Footprint,
    lock_orientation: bool,
    fixed_keepouts: &[Rect],
    tower_keepout: Option<Rect>,
    verbose: bool,
) -> AppResult<PackResult> {
    let options = pack_options(args, profile, lock_orientation, fixed_keepouts, tower_keepout)?;
    // The deterministic grid runs first as a floor score: small models tile as densely as
    // their bounding boxes allow, and the deterministic search only wins when it beats that.
    // Taking the grid on a tie keeps the layout reproducible. `--layout grid` stops right
    // here — no deterministic search at all.
    let grid = grid_layout(footprint, &options);
    let result = match args.layout {
        LayoutMode::Grid => {
            if verbose {
                eprintln!(
                    "grid layout: {} objects (deterministic search skipped)",
                    grid.placements.len()
                );
            }
            grid
        }
        LayoutMode::Search => {
            if verbose {
                eprintln!(
                    "grid baseline: {} objects; deterministic search needs more to win",
                    grid.placements.len()
                );
            }
            let searched = pack_footprint(footprint, &options);
            if searched.placements.len() > grid.placements.len() {
                searched
            } else {
                grid
            }
        }
    };
    validate_result(footprint, &options, &result)?;
    if verbose {
        log_layout(args, footprint, &options, &result);
    }
    Ok(result)
}

fn log_layout(args: &Args, footprint: &Footprint, options: &PackOptions, result: &PackResult) {
    let bounds = layout_world_bounds(footprint, result);
    eprintln!(
        "layout: {} objects within x [{:.1}, {:.1}], y [{:.1}, {:.1}] (bed {:.1}..{:.1} x \
         {:.1}..{:.1}); per-object positions: {}",
        result.placements.len(),
        bounds.min_x,
        bounds.max_x,
        bounds.min_y,
        bounds.max_y,
        options.bed.min_x,
        options.bed.max_x,
        options.bed.min_y,
        options.bed.max_y,
        args.work_dir.join("placements.json").display(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{test_args, test_profile};
    use goat_packer::extract_footprint;

    #[test]
    fn pack_returns_a_layout_that_independently_passes_validate_result() {
        let dir = tempfile::tempdir().unwrap();
        let stl = crate::test_support::square_stl(dir.path());
        let profile = test_profile();
        let args = test_args(dir.path());

        let footprint = extract_footprint(&stl).unwrap();
        let result = pack(&args, &profile, &footprint, false, &[], None, true).unwrap();
        assert!(!result.placements.is_empty());

        let footprint = extract_footprint(&stl).unwrap();
        let options = PackOptions {
            body_bed: None,
            body_exclusions: Vec::new(),
            bed: profile.usable_bed().unwrap(),
            exclusions: vec![],
            resolution: args.resolution,
            clearance: args.clearance,
            angle_step_deg: args.angle_step,
            attempts: args.attempts,
            candidates_per_step: args.candidates,
            max_objects: 0,
            lock_orientation: false,
            stall_attempts: args.stall,
            seed: args.seed,
            support_overlap_mm: None,
        };
        validate_result(&footprint, &options, &result).unwrap();
    }
}

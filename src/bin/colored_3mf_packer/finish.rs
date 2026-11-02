//! The final slice and its checks: slice the assembled plate, confirm the tower, look for
//! supports touching foreign bodies, and repack until the plate is clean.

use goat_packer::{
    PackResult,
    colored_packer::{PrimeTowerBounds, parse_prime_tower_bounds},
    gcode_collisions::{PlateAnalysis, SupportCollision, analyze_plate},
    loops_touch_rect,
    plate_centre::{TOWER_TARGET_TOLERANCE_MM, tower_corner_error_mm},
    print_time::TimeSample,
    progress::{
        self, LayoutSource, RepackReason, Stage, expected_reslice_seconds, expected_slice_seconds,
    },
};

use crate::AppResult;
use crate::job::Job;
use crate::placement::{order_centre_out, placement_body_loops};
use crate::plan::{PackPlan, PackState, SupportRule};
use crate::report::{ColoredPackReport, write_final_report, write_unsliced_plate};
use crate::slicing::{SliceOutput, slice_project};
use crate::support_learning::{
    MAX_NUDGE_ROUNDS, describe_collisions, learn_support_patch, nudge_touching,
};
use crate::time_report::report_time;
use crate::tower::{TowerOutcome, clear_tower_of_patches, evaluate_final_tower};

/// The G-code check is quick next to a slice; the progress bar expects this long.
const EXPECTED_GCODE_CHECK_SECONDS: f64 = 5.0;

/// A packed plate on its way through the final slice. `finish` runs the checks and writes the
/// report; the fields are the verify loop's memory between passes.
pub struct PlateRun<'a> {
    pub job: &'a Job,
    pub plan: PackPlan,
    pub packed: PackResult,
    pub probe_tower: Option<PrimeTowerBounds>,
    probe_seconds: f64,
    pub probe_sample: Option<TimeSample>,
    centre: Option<(f64, f64)>,
    tower_from_contest: bool,
    /// Set once the slicer rejected a moved tower: from then on it stays at the source.
    tower_pinned: bool,
    tower_regrown: bool,
    repacked: bool,
    nudge_rounds: usize,
    /// The webapp scene draws every winning layout; the first one is "pack", later ones "repack".
    layout_source: LayoutSource,
    /// (seconds, objects) of the last full-plate slice: later slices are expected to scale it.
    measured_slice: Option<(f64, usize)>,
}

/// What the passing final slice showed.
pub struct Verified {
    pub slice: SliceOutput,
    pub tower: Option<PrimeTowerBounds>,
    pub tower_relocation_error_mm: Option<f64>,
    pub support_collisions: Vec<SupportCollision>,
}

/// How a check of the final slice ended.
enum Checked {
    /// The plate stands as sliced.
    Done(Verified),
    /// The layout changed; build and slice it again.
    Repacked,
}

impl<'a> PlateRun<'a> {
    pub fn new(job: &'a Job, state: PackState) -> Self {
        Self {
            job,
            plan: state.plan,
            packed: state.packed,
            probe_tower: state.probe_tower,
            probe_seconds: state.probe_seconds,
            probe_sample: state.probe_sample,
            centre: state.centre,
            tower_from_contest: state.tower_from_contest,
            tower_pinned: false,
            tower_regrown: false,
            repacked: false,
            nudge_rounds: 0,
            layout_source: LayoutSource::Pack,
            measured_slice: None,
        }
    }

    fn into_state(self) -> PackState {
        PackState {
            plan: self.plan,
            packed: self.packed,
            probe_tower: self.probe_tower,
            probe_seconds: self.probe_seconds,
            tower_from_contest: self.tower_from_contest,
            centre: self.centre,
            probe_sample: self.probe_sample,
        }
    }

    /// Slices and checks the plate, then writes the report (or, with `--no-final-slice`, the
    /// state a later `--finish-state` run continues from).
    pub fn finish(mut self) -> AppResult<()> {
        let args = &self.job.args;
        let skip_final_slice =
            !args.final_slice && args.finish_state.is_none() && self.job.window_s.is_none();
        if !args.final_slice && self.job.window_s.is_some() {
            println!("note: a time window needs the plate slices; --no-final-slice is ignored");
        }
        let Some(mut verified) = self.verify_until_clean(skip_final_slice)? else {
            let (job, repacked) = (self.job, self.repacked);
            return write_unsliced_plate(job, self.into_state(), repacked);
        };
        let time = report_time(&mut self, &mut verified)?;
        let report = self.final_report(verified, time)?;
        write_final_report(self.job, &report)
    }

    /// Builds and slices the plate until the checks accept it. `None`: `skip_final_slice` and
    /// the 3MF is built.
    fn verify_until_clean(&mut self, skip_final_slice: bool) -> AppResult<Option<Verified>> {
        loop {
            self.prepare_layout()?;
            let count = self.packed.placements.len();
            progress::stage(
                Stage::Assembling,
                &format!("packed {count} objects; assembling 3MF..."),
                None,
                Some(count),
            );
            self.plan.build(self.job, &self.packed)?;
            if skip_final_slice {
                return Ok(None);
            }
            let Some(slice) = self.final_slice(count)? else {
                continue;
            };
            let tower = parse_prime_tower_bounds(
                &slice.gcode,
                self.job.profile.tower_line_width_mm,
                self.job.args.clearance,
            );
            let tower_relocation_error_mm = self.confirm_relocated_tower(tower)?;
            if self.tower_outgrown(tower)? {
                continue;
            }
            match self.check_supports(slice, tower, tower_relocation_error_mm)? {
                Checked::Done(verified) => return Ok(Some(verified)),
                Checked::Repacked => continue,
            }
        }
    }

    /// Orders the layout and settles the tower before each build.
    fn prepare_layout(&mut self) -> AppResult<()> {
        if let Some(centre) = self.centre {
            order_centre_out(&self.plan.footprint, &mut self.packed, centre);
        }
        // Supports may enter the tower area (`body_exclusions`), but when the plate has a free
        // spot the tower goes there instead: same objects, no scrap printed into the tower.
        // Centre-out plates keep their tower at the target.
        if let (None, Some(probe), false) = (self.centre, self.probe_tower, self.tower_pinned)
            && let Some(relocated) = clear_tower_of_patches(
                &self.job.args,
                &self.job.profile,
                &self.plan,
                &self.packed,
                probe.keepout,
                self.job
                    .gui_tower
                    .map(|range| range.keepout_min_range(probe.keepout)),
            )?
        {
            self.plan.tower_keepout = Some(relocated.keepout);
            self.plan.relocation = Some(relocated);
            self.tower_from_contest = true;
        }
        self.show_layout(self.layout_source)?;
        self.layout_source = LayoutSource::Repack;
        Ok(())
    }

    /// Tells the webapp which layout is being built.
    pub fn show_layout(&self, source: LayoutSource) -> AppResult<()> {
        progress::layout(
            source,
            &self.plan.footprint.loops,
            &self.packed.placements,
            self.job.profile.usable_bed()?,
            &self.plan.fixed_keepouts,
            self.plan.tower_keepout,
        );
        Ok(())
    }

    /// How long a slice of `count` objects should take, for the progress bar.
    pub fn expected_slice_seconds(&self, count: usize) -> f64 {
        match self.measured_slice {
            Some((seconds, objects)) => expected_reslice_seconds(seconds, objects, count),
            None => expected_slice_seconds(self.probe_seconds, count),
        }
    }

    /// Slices the assembled plate. `None`: a tower the contest moved to a bed corner is the
    /// one untested input here, and when the slicer rejects that plate the source position
    /// (which the probe sliced fine) gets one repack before the error is final.
    fn final_slice(&mut self, count: usize) -> AppResult<Option<SliceOutput>> {
        progress::stage(
            Stage::FinalSlice,
            &format!("slicing final plate ({count} objects)..."),
            Some(self.expected_slice_seconds(count)),
            Some(count),
        );
        let label = if self.repacked {
            "final_repacked"
        } else {
            "final_first"
        };
        let started = std::time::Instant::now();
        let job = self.job;
        match slice_project(
            &job.profile,
            &job.args.output,
            count,
            &job.args.work_dir,
            label,
        ) {
            Ok(slice) => {
                self.measured_slice = Some((started.elapsed().as_secs_f64(), count));
                Ok(Some(slice))
            }
            Err(error) if self.tower_from_contest => {
                println!(
                    "final slice failed with the tower moved off its source position; \
                     repacking with the tower at its source position\n  {}",
                    error
                        .to_string()
                        .lines()
                        .take(2)
                        .collect::<Vec<_>>()
                        .join("\n  ")
                );
                self.tower_from_contest = false;
                self.tower_pinned = true;
                self.plan.relocation = None;
                self.plan.tower_keepout = self.probe_tower.map(|tower| tower.keepout);
                self.repack(RepackReason::TowerCornerFailed)?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Packs the plan again and tells the progress bar why.
    fn repack(&mut self, reason: RepackReason) -> AppResult<()> {
        self.repacked = true;
        let args = &self.job.args;
        progress::repack(
            reason,
            &format!(
                "re-packing ({} attempts, seed {})...",
                args.attempts, args.seed
            ),
        );
        self.packed = self.plan.pack(self.job)?;
        Ok(())
    }

    /// Checked before the tower evaluation: a slicer that silently dropped the relocation
    /// (e.g. a setting reset to the system value because it is not listed in
    /// `different_settings_to_system`) prints the tower at the source corner, which would
    /// read as a "grown" tower and trigger a pointless repack instead of naming the cause.
    /// Returns the measured distance of the printed tower from the relocation target.
    pub(crate) fn confirm_relocated_tower(
        &self,
        final_tower: Option<PrimeTowerBounds>,
    ) -> AppResult<Option<f64>> {
        let (Some(relocated), Some(tower)) = (self.plan.relocation, final_tower) else {
            return Ok(None);
        };
        let error = tower_corner_error_mm(tower.keepout, relocated.keepout);
        let (mx, my) = (tower.keepout.min_x, tower.keepout.min_y);
        if error > TOWER_TARGET_TOLERANCE_MM {
            return Err(format!(
                "slicer ignored the relocated wipe tower (wipe_tower_x/y): final tower corner \
                 ({mx:.1}, {my:.1}) is {error:.1} mm from the target ({:.1}, {:.1})",
                relocated.keepout.min_x, relocated.keepout.min_y,
            )
            .into());
        }
        println!(
            "relocated prime tower confirmed in the final slice: corner ({mx:.1}, {my:.1}), \
             {error:.1} mm from the target"
        );
        Ok(Some(error))
    }

    /// The final tower against the probe's. `true`: it outgrew the keepout and reaches a
    /// placement, so the plate was repacked (once).
    fn tower_outgrown(&mut self, final_tower: Option<PrimeTowerBounds>) -> AppResult<bool> {
        let outcome = evaluate_final_tower(self.probe_tower, final_tower, self.plan.tower_keepout)?;
        if outcome != TowerOutcome::RepackRequired {
            return Ok(false);
        }
        let grown_keepout = final_tower.unwrap().keepout;
        // The probe tower is sized by ONE object's colour changes; a full plate's tower grows.
        // But only growth that physically reaches a placement's body invalidates the layout
        // (supports may enter the tower area, see `body_exclusions`): repacking on bare
        // keepout arithmetic cost a real object (29 → 28) on plates where the grown strip hit
        // nothing at all. The final slice already ran clean on this exact file, so an
        // untouched layout is already validated.
        let collision = self.packed.placements.iter().position(|placement| {
            loops_touch_rect(
                &placement_body_loops(&self.plan.footprint, placement),
                grown_keepout,
            )
        });
        let Some(index) = collision else {
            println!(
                "final tower outgrew the probe keepout (y up to {:.1} mm) but reaches no \
                 placement; keeping the layout",
                grown_keepout.max_y
            );
            return Ok(false);
        };
        if self.tower_regrown {
            return Err("prime tower changed again after the permitted re-pack".into());
        }
        self.tower_regrown = true;
        println!(
            "final tower outgrew the probe keepout and reaches placement {index}; repacking once"
        );
        self.plan.tower_keepout = Some(grown_keepout);
        self.repack(RepackReason::TowerGrown)?;
        Ok(true)
    }

    /// The layer-by-layer G-code check: does any support touch a foreign body, or print past
    /// the plate? Repacks (or nudges) when it does and there is something to learn.
    fn check_supports(
        &mut self,
        slice: SliceOutput,
        tower: Option<PrimeTowerBounds>,
        tower_relocation_error_mm: Option<f64>,
    ) -> AppResult<Checked> {
        let count = self.packed.placements.len();
        progress::stage(
            Stage::GcodeCheck,
            &format!("checking supports layer by layer ({count} objects)..."),
            Some(EXPECTED_GCODE_CHECK_SECONDS),
            Some(count),
        );
        let analysis = analyze_plate(&slice.gcode);
        let support_collisions = analysis.collisions.clone();
        let done = |slice: SliceOutput, support_collisions: Vec<SupportCollision>| {
            Ok(Checked::Done(Verified {
                slice,
                tower,
                tower_relocation_error_mm,
                support_collisions,
            }))
        };
        analysis.require_object_coverage(count)?;
        if self.handle_off_plate(&analysis)? {
            return Ok(Checked::Repacked);
        }
        if support_collisions.is_empty() {
            println!(
                "support check: {} layers, no support touches a foreign body",
                analysis.layers
            );
            return done(slice, support_collisions);
        }
        describe_collisions(&support_collisions);
        if !self.plan.lock_orientation {
            // No support zone was reserved at all (support-free probe): nothing to learn.
            return Err(
                "final slice has support/body collisions without a learned support zone".into(),
            );
        }
        if self.nudge_touching_objects(&analysis)? {
            return Ok(Checked::Repacked);
        }
        if self.learn_or_tighten(&analysis)? {
            return Ok(Checked::Repacked);
        }
        Err(format!("final slice still has {} support/body collision(s) after repair; refusing unsafe result", support_collisions.len()).into())
    }

    /// The slicer GUI refuses a plate whose lines leave the printable area; the CLI that
    /// produced this slice does not check. Supports drift between slices past the bed-edge
    /// reserve too (`SUPPORT_EDGE_RESERVE_MM`), so the realized first layer is learned and
    /// the plate repacked once; still outside after that is an error, not a silent file.
    /// `true`: the plate was repacked.
    fn handle_off_plate(&mut self, analysis: &PlateAnalysis) -> AppResult<bool> {
        let bed = self.job.profile.printable_bed();
        let off_plate = analysis.off_plate(bed);
        if off_plate.is_empty() {
            return Ok(false);
        }
        for (label, extent) in &off_plate {
            println!(
                "plate check: object {label} prints x [{:.2}, {:.2}] y [{:.2}, {:.2}] — past \
                 the printable area {:.0}..{:.0} x {:.0}..{:.0}",
                extent.min_x,
                extent.max_x,
                extent.min_y,
                extent.max_y,
                bed.min_x,
                bed.max_x,
                bed.min_y,
                bed.max_y
            );
        }
        if !self.plan.lock_orientation || self.plan.patch_learned {
            return Err(format!(
                "{} object(s) print past the plate boundary in the final slice even after \
                 learning the realized supports; the slicer GUI would refuse this plate",
                off_plate.len()
            )
            .into());
        }
        let learned = learn_support_patch(&self.plan.footprint, &self.packed, analysis)?;
        println!(
            "plate check: learned the final slice's support patch ({:.1}x{:.1} mm, probe \
             {:.1}x{:.1} mm); repacking",
            learned.width, learned.height, self.plan.footprint.width, self.plan.footprint.height
        );
        self.plan.footprint = learned;
        self.plan.patch_learned = true;
        self.repack(RepackReason::SupportOffPlate)?;
        Ok(true)
    }

    /// A contact is a few tenths of a millimetre between one or two pairs; a repack lays out
    /// the whole plate again under a stricter zone and cost a P1S goat plate up to 5 objects.
    /// Moving just the touching objects apart keeps every object; the next slice checks the
    /// moved plate again. `true`: the objects were moved.
    fn nudge_touching_objects(&mut self, analysis: &PlateAnalysis) -> AppResult<bool> {
        if self.nudge_rounds >= MAX_NUDGE_ROUNDS {
            return Ok(false);
        }
        let Some(nudged) = nudge_touching(self.job, &self.plan, &self.packed, analysis)? else {
            println!("support check: no small move separates the touching objects; repacking");
            return Ok(false);
        };
        self.nudge_rounds += 1;
        self.repacked = true;
        progress::repack(
            RepackReason::SupportNudge,
            &format!(
                "moving {} touching objects apart (round {} of {MAX_NUDGE_ROUNDS})...",
                analysis.collisions.len(),
                self.nudge_rounds
            ),
        );
        self.packed.placements = nudged;
        Ok(true)
    }

    /// The slicer re-generated bigger supports than the probe showed (the realization drifts
    /// between slices). First learn what it really prints and repack; if that is not enough
    /// either, fall back to the strict rule (supports kept off the whole foreign silhouette) —
    /// what the packer always did before the landing rule. `false`: nothing left to try.
    fn learn_or_tighten(&mut self, analysis: &PlateAnalysis) -> AppResult<bool> {
        let reason = if !self.plan.patch_learned {
            let learned = learn_support_patch(&self.plan.footprint, &self.packed, analysis)?;
            println!(
                "support check: learned the final slice's support patch ({:.1}x{:.1} mm, \
                 probe {:.1}x{:.1} mm); repacking",
                learned.width,
                learned.height,
                self.plan.footprint.width,
                self.plan.footprint.height
            );
            self.plan.footprint = learned;
            self.plan.patch_learned = true;
            RepackReason::SupportContact
        } else if self.plan.rule == SupportRule::Landing {
            println!(
                "support check: still touching after learning the patch; repacking with the \
                 strict rule (supports kept off foreign silhouettes)"
            );
            self.plan.rule = SupportRule::Silhouette;
            RepackReason::SupportStrict
        } else {
            eprintln!(
                "supports still touch foreign bodies under the strict rule; rejecting the layout"
            );
            return Ok(false);
        };
        self.repack(reason)?;
        Ok(true)
    }

    fn final_report(
        &self,
        verified: Verified,
        time: Option<crate::report::TimeReport>,
    ) -> AppResult<ColoredPackReport> {
        let job = self.job;
        let heat_zone = self.heat_zone();
        let mut report = ColoredPackReport::of_packed_plate(
            job,
            &self.plan,
            &self.packed,
            self.probe_tower,
            self.repacked,
        )?;
        report.final_tower = verified.tower;
        report.time = time;
        report.heat_zone = heat_zone;
        report.tower_relocation_error_mm = verified.tower_relocation_error_mm;
        report.support_collisions = verified.support_collisions;
        report.final_sliced = true;
        Ok(report)
    }

    /// Zone report for window plates only: a max-count plate fills the bed by construction.
    fn heat_zone(&self) -> Option<String> {
        self.job.window_s?;
        let profile = &self.job.profile;
        let (bbox, zone) = crate::placement::heat_zone_of(
            profile,
            &self.plan.footprint,
            &self.packed,
            self.plan.tower_keepout,
        );
        if let (false, Some(bbox)) = (profile.heat_zones.is_empty(), bbox) {
            println!(
                "heat zone: {} (objects + tower x [{:.1}, {:.1}], y [{:.1}, {:.1}])",
                zone.as_deref().unwrap_or("none"),
                bbox.min_x,
                bbox.max_x,
                bbox.min_y,
                bbox.max_y
            );
        }
        zone
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    use crate::test_support::{test_job, test_plan};

    fn rejects(gcode: &[u8], strict_exhausted: bool, expected: &str) {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let mut plan = test_plan(&job);
        let packed = plan.pack(&job).unwrap();
        let mut packed = packed;
        packed.placements.truncate(2);
        plan.lock_orientation = strict_exhausted;
        plan.patch_learned = strict_exhausted;
        let state = PackState {
            plan,
            packed,
            probe_tower: None,
            probe_seconds: 0.0,
            tower_from_contest: false,
            centre: None,
            probe_sample: None,
        };
        let mut run = PlateRun::new(&job, state);
        run.nudge_rounds = MAX_NUDGE_ROUNDS;
        let slice = SliceOutput {
            gcode: gcode.to_vec(),
            estimate: Err("not needed".into()),
        };
        let error = match run.check_supports(slice, None, None) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("unsafe/unverifiable slice was accepted"),
        };
        assert!(error.contains(expected), "{error}");
        assert!(!job.args.output.exists());
    }

    const COLLISION: &[u8] = b"; CHANGE_LAYER\n; start printing object, unique label id: 1\n; FEATURE: Support\nG1 X4 Y5\nG1 X7 Y5 E1\n; stop printing object, unique label id: 1\n; start printing object, unique label id: 2\n; FEATURE: Outer wall\nG1 X4 Y5.1\nG1 X7 Y5.1 E1\n";

    #[test]
    fn unsupported_collision_is_an_error_not_a_successful_report() {
        rejects(
            COLLISION,
            false,
            "collisions without a learned support zone",
        );
    }

    #[test]
    fn exhausted_collision_repairs_are_an_error_not_a_warning() {
        rejects(COLLISION, true, "refusing unsafe result");
    }

    #[test]
    fn missing_object_labels_cannot_report_a_verified_plate() {
        rejects(
            b"; CHANGE_LAYER\nG1 X1 Y1\nG1 X2 Y1 E1\n",
            false,
            "cannot verify slice",
        );
    }
}

//! The verification report written next to the packed 3MF.

use std::{fs, path::PathBuf};

use goat_packer::{
    PackResult,
    colored_packer::PrimeTowerBounds,
    gcode_collisions::SupportCollision,
    plate_centre::RelocatedTower,
    print_time::TimeSample,
    progress::{self, Stage},
};
use serde::Serialize;

use crate::AppResult;
use crate::job::Job;
use crate::plan::{PackPlan, PackState, SupportRule};
use crate::source::paint_attribute_count;

#[derive(Debug, Serialize)]
pub struct ColoredPackReport {
    pub source_3mf: String,
    pub output_3mf: String,
    pub profile: String,
    pub object_count: usize,
    pub clearance_mm: f64,
    pub probe_tower: Option<PrimeTowerBounds>,
    pub final_tower: Option<PrimeTowerBounds>,
    pub repacked: bool,
    pub paint_color_attribute_mentions: usize,
    /// Corrected print time of the written plate; `None` only when no window was requested
    /// and the slicer estimate could not be read (the plate itself is still valid).
    pub time: Option<TimeReport>,
    /// Smallest profile heat zone holding the kept objects plus the tower keepout; `None`
    /// without a time window, without profile zones, or when no zone contains the plate.
    pub heat_zone: Option<String>,
    /// Centre of the relocated tower keepout (centre-out window plates with a tower).
    pub tower_relocated_to: Option<[f64; 2]>,
    /// Measured distance of the first final slice's tower keepout min corner from the relocated
    /// keepout's min corner (the corner `wipe_tower_x/y` anchors; growth is one-sided).
    pub tower_relocation_error_mm: Option<f64>,
    /// The probe's first-layer patch reaches past the mesh silhouette (supports/brim), so the
    /// orientation is locked to the sliced one.
    pub supports_detected: bool,
    /// Echo of `--support-overlap-mm`; `null` = unlimited.
    pub support_overlap_mm: Option<f64>,
    /// Which pair rule packed the written plate: `landing` (foreign supports may reach under
    /// a silhouette, not onto its first-layer body), `silhouette` (the strict two-zone rule:
    /// no supports, or the fallback after the landing rule failed the G-code check).
    pub support_rule: SupportRule,
    /// The reserved support patch was widened to what the final slice really printed.
    pub support_patch_learned: bool,
    /// Supports touching a foreign body (empty in successful reports; remaining contacts fail).
    pub support_collisions: Vec<SupportCollision>,
    /// The whole plate was sliced and checked. `false` (`--no-final-slice`): the plate is
    /// packed against the sample slice's supports, the final checks wait for `--finish-state`
    /// with `state_path`; no final tower, time or collisions yet.
    pub final_sliced: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_path: Option<String>,
}

/// `time` block of the verification report. `samples` lists every plate whose time was
/// measured: without a window just the final plate; with one the probe (1 object), the max
/// plate and each window slice in order. `chosen_count`/`corrected_s` describe the written plate.
#[derive(Debug, Serialize)]
pub struct TimeReport {
    pub window_hours: Option<f64>,
    pub margin_pct: f64,
    pub real_change_s: Option<f64>,
    pub slicer_change_s: f64,
    pub prep_s: f64,
    pub samples: Vec<TimeSample>,
    pub chosen_count: usize,
    pub corrected_s: f64,
}

impl ColoredPackReport {
    /// The report of a packed plate whose final slice has not happened (yet): everything the
    /// slice would measure is empty. The sliced plate's report fills those fields in.
    pub fn of_packed_plate(
        job: &Job,
        plan: &PackPlan,
        packed: &PackResult,
        probe_tower: Option<PrimeTowerBounds>,
        repacked: bool,
    ) -> AppResult<Self> {
        let args = &job.args;
        Ok(Self {
            source_3mf: args.source_3mf.display().to_string(),
            output_3mf: args.output.display().to_string(),
            profile: job.profile.id.clone(),
            object_count: packed.placements.len(),
            clearance_mm: args.clearance,
            probe_tower,
            final_tower: None,
            repacked,
            paint_color_attribute_mentions: paint_attribute_count(&args.source_3mf)?,
            time: None,
            heat_zone: None,
            tower_relocated_to: plan.relocation.map(tower_centre),
            tower_relocation_error_mm: None,
            // lock_orientation is set exactly in the "patch past silhouette (supports/brim)"
            // branch: the patch carries a support zone beyond the body.
            supports_detected: plan.lock_orientation,
            support_overlap_mm: args.support_overlap_mm,
            support_rule: plan.rule,
            support_patch_learned: plan.patch_learned,
            support_collisions: Vec::new(),
            final_sliced: false,
            state_path: None,
        })
    }
}

fn tower_centre(relocated: RelocatedTower) -> [f64; 2] {
    [
        (relocated.keepout.min_x + relocated.keepout.max_x) * 0.5,
        (relocated.keepout.min_y + relocated.keepout.max_y) * 0.5,
    ]
}

fn report_path(job: &Job) -> PathBuf {
    job.args.output.with_extension("verification.json")
}

/// `--no-final-slice`: the packed 3MF is already written; saves the state for `--finish-state`
/// and a report that says the plate was not sliced.
pub fn write_unsliced_plate(job: &Job, state: PackState, repacked: bool) -> AppResult<()> {
    progress::stage(
        Stage::Writing,
        "writing the packed plate (final slice skipped)...",
        None,
        None,
    );
    let state_path = job.args.output.with_extension("state.json");
    fs::write(&state_path, serde_json::to_vec(&state)?)?;
    let mut report = ColoredPackReport::of_packed_plate(
        job,
        &state.plan,
        &state.packed,
        state.probe_tower,
        repacked,
    )?;
    report.state_path = Some(state_path.display().to_string());
    let report_path = report_path(job);
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "note: geometry checked against measured support envelopes; a future slicer may generate different supports or tower geometry"
    );
    println!(
        "packed {} colored objects without the final slice; wrote {}\nreport: {}\nstate: {}",
        report.object_count,
        job.args.output.display(),
        report_path.display(),
        state_path.display()
    );
    Ok(())
}

/// The report of a sliced and checked plate.
pub fn write_final_report(job: &Job, report: &ColoredPackReport) -> AppResult<()> {
    progress::stage(
        Stage::Writing,
        "writing the verification report...",
        None,
        None,
    );
    let report_path = report_path(job);
    fs::write(&report_path, serde_json::to_vec_pretty(report)?)?;
    println!(
        "packed {} colored objects; wrote {}\nreport: {}",
        report.object_count,
        job.args.output.display(),
        report_path.display()
    );
    Ok(())
}

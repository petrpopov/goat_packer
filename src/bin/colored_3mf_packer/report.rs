//! The report written next to the packed 3MF (`<output>.verification.json`) and the state a
//! later `--finish-state` run continues from (`<output>.state.json`).

use std::{fs, path::PathBuf};

use goat_packer::{
    ForeignGaps, Point, Rect,
    gcode_collisions::SupportCollision,
    pack::{LevelReport, TowerSpec},
    print_time::TimeSample,
    progress::{self, Stage},
};
use serde::Serialize;

use crate::AppResult;
use crate::args::SupportSource;
use crate::gaps::{GapVariant, OwnGap, floor_warning};
use crate::job::Job;
use crate::margins::{DELTA_BAND_MM, DELTA_FIRST_LAYER_MM};
use crate::order::Canonical;
use crate::plan::{MemberResult, PackState};
use crate::model::ClassSummary;
use crate::source::paint_attribute_count;

#[derive(Debug, Serialize)]
pub struct ColoredPackReport {
    pub source_3mf: String,
    pub output_3mf: String,
    pub profile: String,
    pub object_count: usize,
    pub clearance_mm: f64,
    pub paint_color_attribute_mentions: usize,
    /// `probe`: slicer probes of one object per rotation class (a cache hit counts as one).
    pub support_source: String,
    pub slicer_flavour: String,
    /// The slicer build(s) the plate is computed for, as the user knows them
    /// (`Bambu Studio 02.09.01.70`; the webapp shows it with the result).
    pub target_slicer: String,
    pub flavour_approximated: bool,
    /// Per rotation class: runs sliced, whether the last runs added nothing, cache hit.
    pub probes: Vec<ClassSummary>,
    /// Seconds the support model took (0 with probes).
    pub model_seconds: f64,
    pub margins_mm: Margins,
    /// The slicer's support gap to the support's own model (project settings; the probes
    /// sliced with it and the output 3MF carries the same values, which assembly checks).
    pub own_support_gap: OwnGap,
    /// The packer-only support gaps to a neighbour's body that the layout was packed with.
    pub chosen_gaps: ChosenGaps,
    /// The least a foreign-gap rung could be (`--foreign-gap-floor`).
    pub foreign_gap_floor: ForeignGaps,
    /// Every foreign-gap variant packed: count, wall time and any failure.
    pub gap_variants: Vec<GapVariant>,
    /// Raster resolution of the published layout, mm (the ladder's winning level).
    pub resolution_mm: f64,
    /// The most copies the plate can hold at that resolution (Oler bound, `pack::bound`):
    /// any mix of classes, then each class alone. A ceiling, not a target.
    pub upper_bound: Option<usize>,
    pub class_bounds: Vec<Option<usize>>,
    /// The ladder's levels of the chosen layout, coarse first, with wall time each.
    pub levels: Vec<LevelReport>,
    /// Things the operator should know before trusting the layout (a lowered floor).
    pub warnings: Vec<String>,
    /// Copies per rotation class, in `--rotations` order.
    pub class_counts: Vec<ClassCount>,
    /// The packing member that won and the best count each member reached.
    pub member: String,
    pub member_counts: Vec<MemberResult>,
    pub tower: Option<TowerReport>,
    pub canonical_objects: Vec<Canonical>,
    pub canonical_near_edge: bool,
    pub bed_rules: Vec<BedRule>,
    /// The supports of a first-layer probe reach past the mesh outline.
    pub supports_detected: bool,
    /// Echo of `--support-overlap-mm`; `null` = unlimited.
    pub support_overlap_mm: Option<f64>,
    /// Supports touching a foreign body (empty in successful reports; remaining contacts fail).
    pub support_collisions: Vec<SupportCollision>,
    /// The plate was repacked once after a verification learned the realized geometry.
    pub repacked: bool,
    /// The assembled plate was sliced and checked (`--verify`, a time window).
    pub final_sliced: bool,
    pub verification: Option<Verification>,
    /// Corrected print time of the plate; with a window, the window search.
    pub time: Option<TimeReport>,
    /// Smallest profile heat zone holding the kept objects plus the tower (window plates).
    pub heat_zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_path: Option<String>,
}

/// The foreign gaps the layout was packed with.
#[derive(Debug, Serialize)]
pub struct ChosenGaps {
    pub foreign_base_mm: f64,
    pub foreign_band_mm: f64,
}

#[derive(Debug, Serialize)]
pub struct Margins {
    pub first_layer: f64,
    pub bands: f64,
}

#[derive(Debug, Serialize)]
pub struct ClassCount {
    pub rotation_deg: f64,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct TowerReport {
    pub anchor: Point,
    /// Column size, mm.
    pub size: [f64; 2],
    pub brim_mm: f64,
    pub chamfer_rings: usize,
    /// The union of the probes' towers and the GUI's tower model.
    pub source: &'static str,
}

#[derive(Debug, Serialize)]
pub struct BedRule {
    pub id: String,
    pub class: String,
}

/// What the slicer and the GUI-equivalent checks said about the assembled plate.
#[derive(Debug, Serialize)]
pub struct Verification {
    pub slicer: String,
    pub layers: usize,
    /// Tower extent of the slice, plate coordinates.
    pub tower: Option<Rect>,
    pub gui_checks: &'static str,
    /// Smallest edge gap between a support above the first layer and a foreign body among
    /// pairs closer than 3 mm (`null` = none that close; negative = overlap). A statistic to
    /// read against `chosen_gaps.foreign_band_mm`, never a failure.
    pub min_support_body_gap_mm: Option<f64>,
}

/// `time` block of the report. `samples` lists every plate whose time was measured: without
/// a window just the plate; with one the probe (1 object), the max plate and each window
/// slice in order. `chosen_count`/`corrected_s` describe the written plate.
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
    /// The report of a packed plate that has not been sliced; verification fills in the rest.
    pub fn of_packed_plate(job: &Job, state: &PackState) -> AppResult<Self> {
        let args = &job.args;
        let summary = &state.summary;
        let class_counts = state
            .plan
            .prints
            .iter()
            .enumerate()
            .map(|(class, print)| ClassCount {
                rotation_deg: print.rotation_deg,
                count: state.placements.iter().filter(|p| p.print == class).count(),
            })
            .collect();
        Ok(Self {
            source_3mf: args.source_3mf.display().to_string(),
            output_3mf: args.output.display().to_string(),
            profile: job.profile.id.clone(),
            object_count: state.placements.len(),
            clearance_mm: args.clearance,
            paint_color_attribute_mentions: paint_attribute_count(&args.source_3mf)?,
            support_source: if summary.support_source.is_empty() {
                "probe".to_owned()
            } else {
                summary.support_source.clone()
            },
            slicer_flavour: job.flavour.flavour.name().to_owned(),
            target_slicer: job
                .flavour
                .support_builds()
                .iter()
                .map(|build| build.display_name())
                .collect::<Vec<_>>()
                .join(" + "),
            flavour_approximated: job.flavour.approximated,
            probes: summary.probes.clone(),
            model_seconds: summary.model_seconds,
            margins_mm: margins_of(state),
            own_support_gap: job.own_gap,
            chosen_gaps: ChosenGaps {
                foreign_base_mm: state.plan.foreign_gaps.base_mm,
                foreign_band_mm: state.plan.foreign_gaps.band_mm,
            },
            foreign_gap_floor: args.foreign_gap_floor,
            gap_variants: summary.gap_variants.clone(),
            resolution_mm: summary.resolution_mm,
            upper_bound: summary.upper_bound,
            class_bounds: summary.class_bounds.clone(),
            levels: summary.levels.clone(),
            warnings: crate::job::flavour_warning(args, &job.flavour)
                .into_iter()
                .chain(floor_warning(args.foreign_gap_floor))
                .chain(fallback_warning(args.support_source, &summary.support_source))
                .collect(),
            class_counts,
            member: summary.member.clone(),
            member_counts: summary.member_counts.clone(),
            tower: tower_report(state.tower_anchor, state.plan.tower.as_ref()),
            canonical_near_edge: summary.canonical.iter().any(|c| c.near_edge),
            canonical_objects: summary.canonical.clone(),
            bed_rules: bed_rules(job),
            supports_detected: state.plan.prints.iter().any(|p| !p.support_base.is_empty()),
            support_overlap_mm: args.support_overlap_mm,
            support_collisions: Vec::new(),
            repacked: false,
            final_sliced: false,
            verification: None,
            time: None,
            heat_zone: None,
            state_path: None,
        })
    }
}

fn tower_report(anchor: Option<Point>, spec: Option<&TowerSpec>) -> Option<TowerReport> {
    let (anchor, spec) = (anchor?, spec?);
    Some(TowerReport {
        anchor,
        size: [spec.body.max_x - spec.body.min_x, spec.body.max_y - spec.body.min_y],
        brim_mm: spec.brim_l0_mm,
        chamfer_rings: spec.chamfer.len(),
        source: "probe+model",
    })
}

fn bed_rules(job: &Job) -> Vec<BedRule> {
    job.rules
        .keepouts
        .iter()
        .map(|keepout| BedRule {
            id: keepout.id.clone(),
            class: format!("{:?}", keepout.class).to_lowercase(),
        })
        .collect()
}

fn report_path(job: &Job) -> PathBuf {
    job.args.output.with_extension("verification.json")
}

pub fn state_path(job: &Job) -> PathBuf {
    job.args.output.with_extension("state.json")
}

/// Writes the state a later `--finish-state` run reads.
pub fn write_state(job: &Job, state: &PackState) -> AppResult<PathBuf> {
    let path = state_path(job);
    fs::write(&path, serde_json::to_vec(state)?)?;
    Ok(path)
}

/// Writes the report and tells the operator where everything is.
pub fn write_report(job: &Job, report: &ColoredPackReport) -> AppResult<()> {
    progress::stage(Stage::Writing, "writing the verification report...", None, None);
    let path = report_path(job);
    fs::write(&path, serde_json::to_vec_pretty(report)?)?;
    let verified = if report.final_sliced { "verified by slicing" } else { "without slicing the plate" };
    println!(
        "packed {} colored objects {verified}; wrote {}\nreport: {}",
        report.object_count,
        job.args.output.display(),
        path.display()
    );
    if let Some(state) = &report.state_path {
        println!("state: {state}");
    }
    if !report.final_sliced {
        println!(
            "note: the layout is safe against the measured support envelope plus its margins; a \
             later slice may print different supports. --verify slices the plate and checks it"
        );
    }
    Ok(())
}

/// The margins the plan's supports were grown by (the first class's: every class shares them).
fn margins_of(state: &PackState) -> Margins {
    state
        .plan
        .prints
        .first()
        .map_or(
            Margins {
                first_layer: DELTA_FIRST_LAYER_MM,
                bands: DELTA_BAND_MM,
            },
            |print| Margins {
                first_layer: print.provenance.margins.first_layer_mm,
                bands: print.provenance.margins.band_mm,
            },
        )
}

/// `Auto` that ended in probes: the model refused the project (the log names the setting).
fn fallback_warning(asked: SupportSource, used: &str) -> Option<String> {
    (asked == SupportSource::Auto && used == SupportSource::Probe.as_str()).then(|| {
        "the support model does not cover this project's settings; supports were probed with the slicer".to_owned()
    })
}

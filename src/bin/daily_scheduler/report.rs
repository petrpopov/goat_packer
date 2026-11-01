//! The planner's output: schedule CSV, JSON report and console summary.

use std::{collections::HashSet, fs, io::Write, path::Path};

use goat_packer::scheduler::{
    BatchTiming, OptimizedSchedule, PlateSpec, ScheduleConfig, optimize_schedule,
};
use serde::{Deserialize, Serialize};

use crate::AppResult;
use crate::args::{Args, BenchMode, PrinterInput};
use crate::bench::{BenchmarkRecord, ChangeCorrection, corrected_slicer_seconds, to_timing};

const SECONDS_PER_DAY: u64 = 86_400;
/// Turnaround times (minutes between jobs) swept in the sensitivity table.
const SENSITIVITY_TURNAROUNDS: [u64; 5] = [0, 5, 10, 15, 20];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SensitivityRow {
    printer: String,
    turnaround_minutes: u64,
    preparation_minutes: u64,
    total_objects: usize,
    jobs: usize,
    completion_seconds: u64,
    batch_sequence: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrinterAnalysis {
    pub printer: String,
    pub source_3mf: String,
    pub variable_batch_schedule: OptimizedSchedule,
    pub full_plate_only_schedule: OptimizedSchedule,
    pub gain_vs_full_plate_only: isize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlannerReport {
    pub assumptions: serde_json::Value,
    pub research: serde_json::Value,
    pub benchmarks: Vec<BenchmarkRecord>,
    pub printers: Vec<PrinterAnalysis>,
    pub sensitivity: Vec<SensitivityRow>,
    pub combined_objects_per_day: usize,
}

impl PlannerReport {
    pub fn new(
        args: &Args,
        changes: &[ChangeCorrection],
        benchmarks: Vec<BenchmarkRecord>,
        printers: Vec<PrinterAnalysis>,
        sensitivity: Vec<SensitivityRow>,
    ) -> Self {
        let combined_objects_per_day = printers
            .iter()
            .map(|printer| printer.variable_batch_schedule.total_objects)
            .sum();
        Self {
            assumptions: assumptions(args, changes),
            research: research_notes(),
            benchmarks,
            printers,
            sensitivity,
            combined_objects_per_day,
        }
    }

    pub fn analysis_of(&self, name: &str) -> AppResult<&PrinterAnalysis> {
        self.printers
            .iter()
            .find(|printer| printer.printer == name)
            .ok_or_else(|| format!("no schedule computed for printer {name}").into())
    }
}

fn assumptions(args: &Args, changes: &[ChangeCorrection]) -> serde_json::Value {
    serde_json::json!({
        "day_start": clock_label(args.day_start),
        "fixed_night_start": clock_label(args.night_start),
        "horizon_end_next_day": clock_label(args.horizon_end),
        "turnaround_minutes": args.turnaround_minutes,
        "bench_mode": match args.bench_mode {
            BenchMode::Adaptive => "adaptive",
            BenchMode::Full => "full",
        },
        "printer_preparation_minutes": args.printers.iter().zip(changes).map(|(printer, change)| serde_json::json!({
            "printer": printer.profile.printer_name,
            "preparation_minutes": printer.preparation_minutes,
            "preparation_sweep_minutes": printer.preparation_sweep_minutes,
            "slicer_change_seconds": change.slicer_change_s,
            "real_change_seconds": change.real_change_s,
        })).collect::<Vec<_>>(),
        "schedule_semantics": "Day jobs and their final turnaround must finish by the night start. Exactly one final overnight job starts at night start; no jobs start during the night. The overnight job must finish by the horizon end (next day)."
    })
}

fn research_notes() -> serde_json::Value {
    serde_json::json!({
        "snapmaker_official_faq": "https://support.snapmaker.com/hc/en-us/articles/33344279396759-Snapmaker-U1-FAQ",
        "official_dynamic_flow_calibration_one_head_minutes": 3.5,
        "official_dynamic_flow_calibration_four_heads_minutes": 14.6667,
        "official_complete_per_print_preparation_minutes": null,
        "default_u1_preparation_basis": "11 minutes: user's observed 10-12 minute range; sensitivity covers 6-15 minutes when swept via prep-sweep. The official FAQ publishes component calibration times but no single full per-print startup duration."
    })
}

/// The variable-batch schedule of a printer next to the full-plate-only one.
pub fn analyze_printer(
    printer: &PrinterInput,
    max_count: usize,
    benchmarks: &[BenchmarkRecord],
    config: &ScheduleConfig,
) -> AppResult<PrinterAnalysis> {
    let name = printer.profile.printer_name.as_str();
    let timings: Vec<_> = benchmarks
        .iter()
        .filter(|record| record.printer == name)
        .map(to_timing)
        .collect();
    let variable = optimize_schedule(name, &timings, config)?;
    let full_timing: Vec<_> = timings
        .iter()
        .filter(|timing| timing.batch_size == max_count)
        .cloned()
        .collect();
    let full = optimize_schedule(name, &full_timing, config)?;
    Ok(PrinterAnalysis {
        printer: name.to_owned(),
        source_3mf: printer.source.display().to_string(),
        gain_vs_full_plate_only: variable.total_objects as isize - full.total_objects as isize,
        variable_batch_schedule: variable,
        full_plate_only_schedule: full,
    })
}

pub fn build_sensitivity(
    benchmarks: &[BenchmarkRecord],
    printers: &[PrinterInput],
    counts: &[usize],
    args: &Args,
) -> AppResult<Vec<SensitivityRow>> {
    let mut rows = Vec::new();
    for turnaround in SENSITIVITY_TURNAROUNDS {
        let config = standard_config(turnaround, args);
        for (printer, &max_count) in printers.iter().zip(counts) {
            let name = printer.profile.printer_name.as_str();
            for &preparation in &printer.preparation_sweep_minutes {
                let timings: Vec<_> = benchmarks
                    .iter()
                    .filter(|record| record.printer == name)
                    .map(|record| {
                        BatchTiming::new(
                            record.batch_size,
                            corrected_slicer_seconds(record),
                            preparation * 60,
                            record.filament_grams,
                        )
                    })
                    .collect();
                debug_assert_eq!(timings.iter().map(|b| b.batch_size).max(), Some(max_count));
                rows.push(sensitivity_row(
                    name,
                    turnaround,
                    preparation,
                    optimize_schedule(name, &timings, &config)?,
                ));
            }
        }
    }
    Ok(rows)
}

fn sensitivity_row(
    printer: &str,
    turnaround_minutes: u64,
    preparation_minutes: u64,
    schedule: OptimizedSchedule,
) -> SensitivityRow {
    SensitivityRow {
        printer: printer.to_owned(),
        turnaround_minutes,
        preparation_minutes,
        total_objects: schedule.total_objects,
        jobs: schedule.jobs.len(),
        completion_seconds: schedule.completion_seconds,
        batch_sequence: schedule
            .jobs
            .iter()
            .map(|job| job.batch_size.to_string())
            .collect::<Vec<_>>()
            .join(" + "),
    }
}

pub fn standard_config(turnaround_minutes: u64, args: &Args) -> ScheduleConfig {
    ScheduleConfig {
        day_start_seconds: args.day_start,
        night_start_seconds: args.night_start,
        horizon_end_seconds: args.horizon_end,
        turnaround_seconds: turnaround_minutes * 60,
    }
}

/// Every distinct batch size in `schedule`'s job sequence, in first-appearance order, turned into
/// one [`PlateSpec`] each — a batch size that recurs (e.g. two identical day jobs) becomes one
/// plate, not one per run, since it's the same physical layout either way. Labeled with the batch
/// size and the clock time of that job's first run, so the plate list in BambuStudio reads as the
/// actual production plan (e.g. "22 pcs @ 23:00").
pub fn plate_specs_for_schedule(schedule: &OptimizedSchedule) -> Vec<PlateSpec> {
    let mut seen = HashSet::new();
    let mut specs = Vec::new();
    for job in &schedule.jobs {
        if seen.insert(job.batch_size) {
            specs.push(PlateSpec {
                label: format!(
                    "{} pcs @ {}",
                    job.batch_size,
                    clock_label(job.start_seconds)
                ),
                object_count: job.batch_size,
            });
        }
    }
    specs
}

/// Writes just the one artifact daily_scheduler's caller actually wants day to day: the resolved
/// per-printer job sequence. Benchmarks/sensitivity are only written when `--output-report` is
/// given (see `run`) — most invocations don't need that level of detail.
pub fn write_schedule_csv(path: &Path, report: &PlannerReport) -> AppResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut schedule = fs::File::create(path)?;
    writeln!(
        schedule,
        "printer,sequence,batch_size,start_seconds,end_seconds,start_clock,end_clock,slicer_seconds,preparation_seconds,filament_grams,cumulative_objects,overnight"
    )?;
    for printer in &report.printers {
        for job in &printer.variable_batch_schedule.jobs {
            writeln!(
                schedule,
                "{},{},{},{},{},{},{},{},{},{:.4},{},{}",
                printer.printer,
                job.sequence,
                job.batch_size,
                job.start_seconds,
                job.end_seconds,
                clock_label(job.start_seconds),
                clock_label(job.end_seconds),
                job.slicer_seconds,
                job.preparation_seconds,
                job.filament_grams,
                job.cumulative_objects,
                job.overnight
            )?;
        }
    }
    Ok(())
}

pub fn print_summary(report: &PlannerReport) {
    println!("\nOptimized 24-hour production cycle:");
    for printer in &report.printers {
        let sequence = printer
            .variable_batch_schedule
            .jobs
            .iter()
            .map(|job| format!("{}@{}", job.batch_size, clock_label(job.start_seconds)))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{}: {} objects, {} jobs [{}], gain vs full plates: {:+}",
            printer.printer,
            printer.variable_batch_schedule.total_objects,
            printer.variable_batch_schedule.jobs.len(),
            sequence,
            printer.gain_vs_full_plate_only
        );
    }
    println!("Combined: {} objects/day", report.combined_objects_per_day);
}

pub fn clock_label(seconds: u64) -> String {
    let day = seconds / SECONDS_PER_DAY;
    let within = seconds % SECONDS_PER_DAY;
    let hours = within / 3600;
    let minutes = (within % 3600) / 60;
    if day == 0 {
        format!("{hours:02}:{minutes:02}")
    } else {
        format!("D+{day} {hours:02}:{minutes:02}")
    }
}

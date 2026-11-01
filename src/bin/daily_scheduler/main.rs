//! Plans a 24-hour production cycle for one or more printers from a packed 3MF: benchmarks
//! batch sizes with the slicer, then picks the day/night job sequence that prints the most.
//!
//! Modules: `args` (command line), `bench` (slicing and timing batches), `report` (schedule,
//! sensitivity table, output files).

mod args;
mod bench;
mod report;

use std::{error::Error, fs, path::Path};

use goat_packer::{
    print_time::slicer_change_seconds,
    scheduler::{build_schedule_plates_3mf, object_count_3mf, plate_count_3mf},
};

use args::{Args, parse_args};
use bench::{
    BenchTarget, BenchmarkRecord, ChangeCorrection, apply_change_correction, benchmark_printer,
    reused_rows_lack_change_counts, sha256_file, slugify,
};
use report::{
    PlannerReport, analyze_printer, build_sensitivity, plate_specs_for_schedule, print_summary,
    standard_config, write_schedule_csv,
};

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let args = parse_args()?;
    // Slicing benchmarks and intermediate subset .3mf copies are scratch: nothing outside this
    // process needs them once the schedule is computed, so they live in a TempDir that cleans
    // itself up (including on early return via `?`) instead of a directory the caller has to
    // remember to delete.
    let work_dir_guard = tempfile::tempdir()?;
    let counts = source_object_counts(&args)?;
    let changes = change_corrections(&args)?;
    let benchmarks = if args.reuse_benchmarks {
        reused_benchmarks(&args, &changes)?
    } else {
        run_benchmarks(&args, &counts, &changes, work_dir_guard.path())?
    };

    let config = standard_config(args.turnaround_minutes, &args);
    let printers_out = args
        .printers
        .iter()
        .zip(&counts)
        .map(|(printer, &max_count)| analyze_printer(printer, max_count, &benchmarks, &config))
        .collect::<AppResult<Vec<_>>>()?;
    let sensitivity = build_sensitivity(&benchmarks, &args.printers, &counts, &args)?;
    let report = PlannerReport::new(&args, &changes, benchmarks, printers_out, sensitivity);

    write_outputs(&args, &report)?;
    print_summary(&report);
    Ok(())
}

/// The object count of every printer's source, which must be a single dense plate.
fn source_object_counts(args: &Args) -> AppResult<Vec<usize>> {
    let mut counts = Vec::with_capacity(args.printers.len());
    for printer in &args.printers {
        let plates = plate_count_3mf(&printer.source)?;
        if plates != 1 {
            return Err(format!(
                "{}: source {} has {plates} plates; daily_scheduler needs a single-plate, \
                 densely-packed source (e.g. produced by colored_3mf_packer) — a multi-plate file \
                 slices batches whose later objects silently belong to a different plate",
                printer.profile.printer_name,
                printer.source.display()
            )
            .into());
        }
        let count = object_count_3mf(&printer.source)?;
        println!("{}: {count} objects", printer.profile.printer_name);
        counts.push(count);
    }
    Ok(counts)
}

/// Read once per printer: subsets copy project_settings.config, so the source's per-change
/// assumption holds for every benchmark slice.
fn change_corrections(args: &Args) -> AppResult<Vec<ChangeCorrection>> {
    let mut changes = Vec::with_capacity(args.printers.len());
    for printer in &args.printers {
        let change = ChangeCorrection {
            slicer_change_s: slicer_change_seconds(&source_project_settings(&printer.source)?),
            real_change_s: printer.real_change_seconds(args.ams_correction),
        };
        if args.ams_correction && change.real_change_s.is_none() {
            println!(
                "{}: profile has no filament_change_seconds (tool changer); no AMS correction",
                printer.profile.printer_name
            );
        }
        changes.push(change);
    }
    Ok(changes)
}

/// `--reuse-benchmarks`: the rows of a prior `--output-report`, re-corrected for this run.
fn reused_benchmarks(args: &Args, changes: &[ChangeCorrection]) -> AppResult<Vec<BenchmarkRecord>> {
    let report_path = args.output_report.as_deref().ok_or(
        "--reuse-benchmarks requires --output-report <path> to read prior benchmarks from",
    )?;
    let previous: PlannerReport = serde_json::from_slice(&fs::read(report_path)?)?;
    println!("Reusing {} prior benchmark rows", previous.benchmarks.len());
    let correction_active = changes.iter().any(|change| change.real_change_s.is_some());
    if reused_rows_lack_change_counts(&previous.benchmarks, correction_active) {
        println!(
            "note: every reused benchmark row has 0 filament changes; reports written before \
             the AMS correction carry no change counts, so no AMS correction is applied — \
             re-benchmark without --reuse-benchmarks to correct for the real change time"
        );
    }
    let mut benchmarks = previous.benchmarks;
    // The correction is a policy over measured change counts, not a measurement: re-derive it so
    // reused rows follow this run's --ams-correction/change-s settings.
    for (printer, change) in args.printers.iter().zip(changes) {
        for record in benchmarks
            .iter_mut()
            .filter(|record| record.printer == printer.profile.printer_name)
        {
            apply_change_correction(record, *change);
        }
    }
    Ok(benchmarks)
}

/// Slices every printer's source and records the timings.
fn run_benchmarks(
    args: &Args,
    counts: &[usize],
    changes: &[ChangeCorrection],
    work_dir: &Path,
) -> AppResult<Vec<BenchmarkRecord>> {
    let mut benchmarks = Vec::new();
    for ((printer, &count), change) in args.printers.iter().zip(counts).zip(changes) {
        let hash = sha256_file(&printer.source)?;
        let target = BenchTarget {
            printer: &printer.profile.printer_name,
            slug: slugify(&printer.profile.printer_name),
            source: &printer.source,
            max_count: count,
            slicer: Path::new(&printer.profile.slicer_path),
            slicer_kind: printer.profile.slicer_kind.as_str(),
            blank_start_gcode: printer.profile.blank_start_gcode_for_cli,
            preparation_seconds: printer.preparation_minutes * 60,
            change: *change,
            source_hash: &hash,
        };
        benchmarks.extend(benchmark_printer(&target, args.bench_mode, work_dir)?);
    }
    Ok(benchmarks)
}

/// The schedule CSV, the optional JSON report, and each printer's multi-plate 3MF.
fn write_outputs(args: &Args, report: &PlannerReport) -> AppResult<()> {
    write_schedule_csv(&args.output_schedule, report)?;
    if let Some(report_path) = &args.output_report {
        fs::write(report_path, serde_json::to_vec_pretty(report)?)?;
    }
    for printer in &args.printers {
        let Some(plates_output) = &printer.plates_output else {
            continue;
        };
        let analysis = report.analysis_of(&printer.profile.printer_name)?;
        let specs = plate_specs_for_schedule(&analysis.variable_batch_schedule);
        println!(
            "{}: writing {} plate(s) to {}",
            printer.profile.printer_name,
            specs.len(),
            plates_output.display()
        );
        let bed = printer.profile.printable_bed();
        build_schedule_plates_3mf(
            &printer.source,
            plates_output,
            &specs,
            bed.max_x - bed.min_x,
            bed.max_y - bed.min_y,
            printer.profile.blank_start_gcode_for_cli,
        )?;
    }
    Ok(())
}

fn source_project_settings(source: &Path) -> AppResult<serde_json::Value> {
    Ok(serde_json::Value::Object(
        goat_packer::orca_project::project_settings(source)?,
    ))
}

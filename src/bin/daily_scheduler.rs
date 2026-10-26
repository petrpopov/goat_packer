use std::{
    collections::HashSet,
    error::Error,
    ffi::OsStr,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use goat_packer::{
    colored_packer::{PrinterProfile, load_printer_profile},
    scheduler::{
        BatchCurve, BatchTiming, OptimizedSchedule, PlateSpec, ScheduleConfig, SliceEstimate,
        build_schedule_plates_3mf, least_squares_line, next_refinement_count, object_count_3mf,
        optimize_schedule, parse_bambu_result, parse_orca_gcode, plate_count_3mf, subset_3mf,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Relative disagreement between the piecewise-linear fit and the affine least-squares fit at
/// which the adaptive benchmark stops adding slices. 2% is far below the granularity the DP acts
/// on (batch decisions jump in whole objects; a 2% timing error rarely flips a batch size).
const REFINE_TOLERANCE: f64 = 0.02;

/// Adaptive benchmarking measures at most this many sizes (initial samples + refinements). For
/// small plates the budget covers everything and the mode degenerates to the full sweep.
fn adaptive_budget(max_count: usize) -> usize {
    (max_count / 3).max(6).min(max_count)
}

#[derive(Clone, Debug)]
struct PrinterInput {
    profile: PrinterProfile,
    source: PathBuf,
    preparation_minutes: u64,
    preparation_sweep_minutes: Vec<u64>,
    plates_output: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BenchMode {
    /// Slice only a sample of batch sizes and interpolate the rest (default).
    Adaptive,
    /// Slice every size from 1 to N — the original behaviour, kept for validating interpolation.
    Full,
}

#[derive(Debug)]
struct Args {
    printers: Vec<PrinterInput>,
    output_schedule: PathBuf,
    output_report: Option<PathBuf>,
    turnaround_minutes: u64,
    day_start: u64,
    night_start: u64,
    horizon_end: u64,
    bench_mode: BenchMode,
    reuse_benchmarks: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum BenchmarkOrigin {
    Measured,
    Interpolated,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct BenchmarkRecord {
    printer: String,
    batch_size: usize,
    model_seconds: u64,
    slicer_total_seconds: u64,
    preparation_seconds: u64,
    job_seconds: u64,
    filament_grams: f64,
    units_per_job_hour: f64,
    source_3mf: String,
    source_sha256: String,
    slicer: String,
    /// Whether this row came from a real slice or from interpolation between two real slices.
    /// Absent in reports written before adaptive benchmarking existed; those rows are all
    /// measured, which is what the default encodes.
    #[serde(default = "measured_origin")]
    origin: BenchmarkOrigin,
}

fn measured_origin() -> BenchmarkOrigin {
    BenchmarkOrigin::Measured
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SensitivityRow {
    printer: String,
    turnaround_minutes: u64,
    preparation_minutes: u64,
    total_objects: usize,
    jobs: usize,
    completion_seconds: u64,
    batch_sequence: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PrinterAnalysis {
    printer: String,
    source_3mf: String,
    variable_batch_schedule: OptimizedSchedule,
    full_plate_only_schedule: OptimizedSchedule,
    gain_vs_full_plate_only: isize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PlannerReport {
    assumptions: serde_json::Value,
    research: serde_json::Value,
    benchmarks: Vec<BenchmarkRecord>,
    printers: Vec<PrinterAnalysis>,
    sensitivity: Vec<SensitivityRow>,
    combined_objects_per_day: usize,
}

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
    let work_dir = work_dir_guard.path();

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

    let mut benchmarks = if args.reuse_benchmarks {
        let report_path = args.output_report.as_deref().ok_or(
            "--reuse-benchmarks requires --output-report <path> to read prior benchmarks from",
        )?;
        let previous: PlannerReport = serde_json::from_slice(&fs::read(report_path)?)?;
        println!("Reusing {} prior benchmark rows", previous.benchmarks.len());
        previous.benchmarks
    } else {
        Vec::new()
    };
    if !args.reuse_benchmarks {
        for (printer, &count) in args.printers.iter().zip(&counts) {
            let hash = sha256_file(&printer.source)?;
            benchmarks.extend(benchmark_printer(
                &printer.profile.printer_name,
                &slugify(&printer.profile.printer_name),
                &printer.source,
                count,
                Path::new(&printer.profile.slicer_path),
                printer.profile.slicer_kind.as_str(),
                printer.profile.blank_start_gcode_for_cli,
                printer.preparation_minutes * 60,
                &hash,
                args.bench_mode,
                work_dir,
            )?);
        }
    }

    let config = ScheduleConfig {
        day_start_seconds: args.day_start,
        night_start_seconds: args.night_start,
        horizon_end_seconds: args.horizon_end,
        turnaround_seconds: args.turnaround_minutes * 60,
    };
    let mut printers_out = Vec::new();
    for (printer, &max_count) in args.printers.iter().zip(&counts) {
        let name = printer.profile.printer_name.as_str();
        let records: Vec<_> = benchmarks
            .iter()
            .filter(|record| record.printer == name)
            .collect();
        let timings: Vec<_> = records.iter().map(|record| to_timing(record)).collect();
        let variable = optimize_schedule(name, &timings, &config)?;
        let full_timing: Vec<_> = timings
            .iter()
            .filter(|timing| timing.batch_size == max_count)
            .cloned()
            .collect();
        let full = optimize_schedule(name, &full_timing, &config)?;
        printers_out.push(PrinterAnalysis {
            printer: name.to_owned(),
            source_3mf: printer.source.display().to_string(),
            gain_vs_full_plate_only: variable.total_objects as isize - full.total_objects as isize,
            variable_batch_schedule: variable,
            full_plate_only_schedule: full,
        });
    }

    let sensitivity = build_sensitivity(&benchmarks, &args.printers, &counts, &args)?;
    let combined_objects_per_day = printers_out
        .iter()
        .map(|printer| printer.variable_batch_schedule.total_objects)
        .sum();
    let report = PlannerReport {
        assumptions: serde_json::json!({
            "day_start": clock_label(args.day_start),
            "fixed_night_start": clock_label(args.night_start),
            "horizon_end_next_day": clock_label(args.horizon_end),
            "turnaround_minutes": args.turnaround_minutes,
            "bench_mode": match args.bench_mode {
                BenchMode::Adaptive => "adaptive",
                BenchMode::Full => "full",
            },
            "printer_preparation_minutes": args.printers.iter().map(|printer| serde_json::json!({
                "printer": printer.profile.printer_name,
                "preparation_minutes": printer.preparation_minutes,
                "preparation_sweep_minutes": printer.preparation_sweep_minutes,
            })).collect::<Vec<_>>(),
            "schedule_semantics": "Day jobs and their final turnaround must finish by the night start. Exactly one final overnight job starts at night start; no jobs start during the night. The overnight job must finish by the horizon end (next day)."
        }),
        research: serde_json::json!({
            "snapmaker_official_faq": "https://support.snapmaker.com/hc/en-us/articles/33344279396759-Snapmaker-U1-FAQ",
            "official_dynamic_flow_calibration_one_head_minutes": 3.5,
            "official_dynamic_flow_calibration_four_heads_minutes": 14.6667,
            "official_complete_per_print_preparation_minutes": null,
            "default_u1_preparation_basis": "11 minutes: user's observed 10-12 minute range; sensitivity covers 6-15 minutes when swept via prep-sweep. The official FAQ publishes component calibration times but no single full per-print startup duration."
        }),
        benchmarks,
        printers: printers_out,
        sensitivity,
        combined_objects_per_day,
    };
    write_schedule_csv(&args.output_schedule, &report)?;
    if let Some(report_path) = &args.output_report {
        fs::write(report_path, serde_json::to_vec_pretty(&report)?)?;
    }
    for printer in &args.printers {
        let Some(plates_output) = &printer.plates_output else {
            continue;
        };
        let analysis = printers_out_by_name(&report, &printer.profile.printer_name)?;
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
    print_summary(&report);
    Ok(())
}

/// Every distinct batch size in `schedule`'s job sequence, in first-appearance order, turned into
/// one [`PlateSpec`] each — a batch size that recurs (e.g. two identical day jobs) becomes one
/// plate, not one per run, since it's the same physical layout either way. Labeled with the batch
/// size and the clock time of that job's first run, so the plate list in BambuStudio reads as the
/// actual production plan (e.g. "22 pcs @ 23:00").
fn plate_specs_for_schedule(schedule: &OptimizedSchedule) -> Vec<PlateSpec> {
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

fn printers_out_by_name<'a>(
    report: &'a PlannerReport,
    name: &str,
) -> AppResult<&'a PrinterAnalysis> {
    report
        .printers
        .iter()
        .find(|printer| printer.printer == name)
        .ok_or_else(|| format!("no schedule computed for printer {name}").into())
}

#[allow(clippy::too_many_arguments)]
fn benchmark_printer(
    printer: &str,
    slug: &str,
    source: &Path,
    max_count: usize,
    slicer: &Path,
    slicer_kind: &str,
    blank_start_gcode: bool,
    preparation_seconds: u64,
    source_hash: &str,
    bench_mode: BenchMode,
    work_dir: &Path,
) -> AppResult<Vec<BenchmarkRecord>> {
    match bench_mode {
        BenchMode::Full => {
            let mut records = Vec::new();
            for count in 1..=max_count {
                println!("[{printer}] slicing batch {count}/{max_count}");
                let estimate = slice_one_count(
                    printer, slug, source, count, slicer, slicer_kind, blank_start_gcode, work_dir,
                )?;
                records.push(record_from_estimate(
                    printer,
                    count,
                    estimate,
                    preparation_seconds,
                    source,
                    source_hash,
                    slicer,
                ));
            }
            Ok(records)
        }
        BenchMode::Adaptive => adaptive_benchmarks(
            printer,
            slug,
            source,
            max_count,
            slicer,
            slicer_kind,
            blank_start_gcode,
            preparation_seconds,
            source_hash,
            work_dir,
        ),
    }
}

/// Adaptive benchmarking: slice a small sample of batch sizes (always 1 and the full plate, plus
/// a few interior sizes), then refine — slice one more size wherever the piecewise-linear fit
/// disagrees most with an affine least-squares fit, i.e. where real curvature is most likely
/// hiding — until the worst disagreement drops under [`REFINE_TOLERANCE`] or the slice budget
/// runs out. Print time versus batch size is close to affine (fixed startup + per-object cost),
/// so near-affine data stops refining immediately; curved data concentrates real slices where
/// they matter. Batch sizes that were never sliced get their timings interpolated between the
/// two nearest measured sizes and are marked `Interpolated` in the report.
#[allow(clippy::too_many_arguments)]
fn adaptive_benchmarks(
    printer: &str,
    slug: &str,
    source: &Path,
    max_count: usize,
    slicer: &Path,
    slicer_kind: &str,
    blank_start_gcode: bool,
    preparation_seconds: u64,
    source_hash: &str,
    work_dir: &Path,
) -> AppResult<Vec<BenchmarkRecord>> {
    let budget = adaptive_budget(max_count);

    let counts: Vec<usize> = initial_sample_counts(max_count, budget);
    let mut measured: Vec<(usize, SliceEstimate)> = Vec::new();
    let total_planned = counts.len();
    for (index, &count) in counts.iter().enumerate() {
        println!("[{printer}] slicing sample {}/{total_planned}: batch {count}/{max_count}", index + 1);
        let estimate = slice_one_count(
            printer, slug, source, count, slicer, slicer_kind, blank_start_gcode, work_dir,
        )?;
        measured.push((count, estimate));
    }

    loop {
        if measured.len() >= budget {
            break;
        }
        let seconds_curve = BatchCurve::new(
            &measured
                .iter()
                .map(|(count, estimate)| (*count, estimate.slicer_total_seconds as f64))
                .collect::<Vec<_>>(),
        )?;
        let line = least_squares_line(
            &measured
                .iter()
                .map(|(count, estimate)| (*count, estimate.slicer_total_seconds as f64))
                .collect::<Vec<_>>(),
        );
        let Some((count, deviation)) = next_refinement_count(
            &measured.iter().map(|(count, _)| *count).collect::<Vec<_>>(),
            max_count,
            &seconds_curve,
            line,
        ) else {
            break;
        };
        if deviation < REFINE_TOLERANCE {
            println!(
                "[{printer}] interpolation fits within {:.1}% — no more slices needed",
                deviation * 100.0
            );
            break;
        }
        println!(
            "[{printer}] refining: slicing batch {count} ({:.1}% curvature)",
            deviation * 100.0
        );
        let estimate =
            slice_one_count(printer, slug, source, count, slicer, slicer_kind, blank_start_gcode, work_dir)?;
        measured.push((count, estimate));
        measured.sort_by_key(|(count, _)| *count);
    }

    let counts: Vec<usize> = measured.iter().map(|(count, _)| *count).collect();
    let model_curve = BatchCurve::new(
        &measured
            .iter()
            .map(|(count, estimate)| (*count, estimate.model_seconds as f64))
            .collect::<Vec<_>>(),
    )?;
    let seconds_curve = BatchCurve::new(
        &measured
            .iter()
            .map(|(count, estimate)| (*count, estimate.slicer_total_seconds as f64))
            .collect::<Vec<_>>(),
    )?;
    let grams_curve = BatchCurve::new(
        &measured
            .iter()
            .map(|(count, estimate)| (*count, estimate.filament_grams))
            .collect::<Vec<_>>(),
    )?;

    let measured_count = counts.len();
    println!(
        "[{printer}] {measured_count} measured sizes cover batches 1..={max_count} ({:.0}% fewer slices than a full sweep)",
        (1.0 - measured_count as f64 / max_count as f64) * 100.0
    );

    let mut records = Vec::with_capacity(max_count);
    for count in 1..=max_count {
        if let Some((_, estimate)) = measured.iter().find(|(measured_count, _)| *measured_count == count) {
            records.push(record_from_estimate(
                printer,
                count,
                estimate.clone(),
                preparation_seconds,
                source,
                source_hash,
                slicer,
            ));
            continue;
        }
        let interpolated = SliceEstimate {
            object_count: count,
            model_seconds: model_curve.value(count).round() as u64,
            slicer_total_seconds: seconds_curve.value(count).round() as u64,
            filament_grams: grams_curve.value(count),
        };
        let mut record = record_from_estimate(
            printer,
            count,
            interpolated,
            preparation_seconds,
            source,
            source_hash,
            slicer,
        );
        record.origin = BenchmarkOrigin::Interpolated;
        records.push(record);
    }
    Ok(records)
}

/// Initial sample for the adaptive benchmark: the endpoints (1 and the full plate) plus evenly
/// spaced interior sizes, deduplicated and sorted, capped at the slice budget. When the budget
/// already covers every size, this is simply the full sweep.
fn initial_sample_counts(max_count: usize, budget: usize) -> Vec<usize> {
    if budget >= max_count {
        return (1..=max_count).collect();
    }
    let mut counts = vec![1, max_count];
    for fraction in [2, 5, 8] {
        let count = ((max_count as f64) * (fraction as f64) / 10.0).round() as usize;
        counts.push(count.clamp(1, max_count));
    }
    counts.sort_unstable();
    counts.dedup();
    if counts.len() > budget {
        // Keep the endpoints and drop interior sizes from the middle of the list.
        let drop = counts.len() - budget;
        counts.drain(1..1 + drop);
    }
    counts
}

#[allow(clippy::too_many_arguments)]
fn slice_one_count(
    printer: &str,
    slug: &str,
    source: &Path,
    count: usize,
    slicer: &Path,
    slicer_kind: &str,
    blank_start_gcode: bool,
    work_dir: &Path,
) -> AppResult<SliceEstimate> {
    let subset = work_dir.join(format!("{slug}_{count}.3mf"));
    subset_3mf(source, &subset, count, blank_start_gcode)?;
    let slice_dir = work_dir.join(format!("{slug}_{count}_slice"));
    fs::create_dir_all(&slice_dir)?;
    let output = goat_packer::process_output::bounded_output(
        &mut goat_packer::slicer_cli::slice_command(slicer, &[], &slice_dir, &subset),
    )?;
    fs::write(
        work_dir.join(format!("{slug}_{count}.log")),
        [&output.stdout[..], &output.stderr[..]].concat(),
    )?;
    if !output.status.success() {
        return Err(format!(
            "{printer} slicer failed for {count} objects with status {}: {}",
            output.status,
            String::from_utf8_lossy(&[output.stdout, output.stderr].concat())
        )
        .into());
    }
    let estimate = if slicer_kind == "bambu" {
        parse_bambu_result(&fs::read(slice_dir.join("result.json"))?)?
    } else {
        let gcode = find_extension(&slice_dir, "gcode")?;
        parse_orca_gcode(&fs::read(gcode)?, count)?
    };
    if estimate.object_count != count {
        return Err(format!(
            "{printer} subset {count} sliced as {} objects",
            estimate.object_count
        )
        .into());
    }
    Ok(estimate)
}

fn record_from_estimate(
    printer: &str,
    count: usize,
    estimate: SliceEstimate,
    preparation_seconds: u64,
    source: &Path,
    source_hash: &str,
    slicer: &Path,
) -> BenchmarkRecord {
    let job_seconds = estimate.slicer_total_seconds + preparation_seconds;
    BenchmarkRecord {
        printer: printer.to_owned(),
        batch_size: count,
        model_seconds: estimate.model_seconds,
        slicer_total_seconds: estimate.slicer_total_seconds,
        preparation_seconds,
        job_seconds,
        filament_grams: estimate.filament_grams,
        units_per_job_hour: count as f64 / (job_seconds as f64 / 3600.0),
        source_3mf: source.display().to_string(),
        source_sha256: source_hash.to_owned(),
        slicer: slicer.display().to_string(),
        origin: BenchmarkOrigin::Measured,
    }
}

fn to_timing(record: &BenchmarkRecord) -> BatchTiming {
    BatchTiming::new(
        record.batch_size,
        record.slicer_total_seconds,
        record.preparation_seconds,
        record.filament_grams,
    )
}

fn build_sensitivity(
    benchmarks: &[BenchmarkRecord],
    printers: &[PrinterInput],
    counts: &[usize],
    args: &Args,
) -> AppResult<Vec<SensitivityRow>> {
    let mut rows = Vec::new();
    for turnaround in [0, 5, 10, 15, 20] {
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
                            record.slicer_total_seconds,
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

fn standard_config(turnaround_minutes: u64, args: &Args) -> ScheduleConfig {
    ScheduleConfig {
        day_start_seconds: args.day_start,
        night_start_seconds: args.night_start,
        horizon_end_seconds: args.horizon_end,
        turnaround_seconds: turnaround_minutes * 60,
    }
}

/// Writes just the one artifact daily_scheduler's caller actually wants day to day: the resolved
/// per-printer job sequence. Benchmarks/sensitivity are only written when `--output-report` is
/// given (see `run`) — most invocations don't need that level of detail.
fn write_schedule_csv(path: &Path, report: &PlannerReport) -> AppResult<()> {
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

fn print_summary(report: &PlannerReport) {
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

fn clock_label(seconds: u64) -> String {
    let day = seconds / 86_400;
    let within = seconds % 86_400;
    let hours = within / 3600;
    let minutes = (within % 3600) / 60;
    if day == 0 {
        format!("{hours:02}:{minutes:02}")
    } else {
        format!("D+{day} {hours:02}:{minutes:02}")
    }
}

fn find_extension(directory: &Path, extension: &str) -> AppResult<PathBuf> {
    fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension() == Some(OsStr::new(extension)))
        .ok_or_else(|| format!("no .{extension} file in {}", directory.display()).into())
}

fn sha256_file(path: &Path) -> AppResult<String> {
    let mut hasher = Sha256::new();
    hasher.update(fs::read(path)?);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Turns a printer display name into a filesystem-safe slug for benchmark work-dir filenames
/// (e.g. "Bambu Lab P1S" -> "bambu_lab_p1s"): lowercased, non-alphanumeric runs collapsed to a
/// single `_`, no leading/trailing `_`.
fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_was_sep = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            slug.push('_');
            last_was_sep = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    slug
}

/// Parses one `--printer` spec: comma-separated `key=value` fields. Required: `profile=<path to a
/// PrinterProfile json>` and `source=<path to a packed .3mf for that printer>`. Optional:
/// `name=<override>` (defaults to the profile's `printer_name`, letting two `--printer` entries
/// share one profile with distinct labels/sources), `prep-min=<u64>` (default 0; extra per-job
/// dead time the slicer doesn't account for, e.g. multi-material filament switching),
/// `prep-sweep=<a:b:c:...>` (default `[prep-min]`; alternate preparation minutes to sweep in the
/// optional `--output-report`'s sensitivity table when that number is uncertain), and
/// `plates-3mf=<path>` (writes this printer's resolved schedule as one multi-plate .3mf: one
/// plate per distinct batch size, trimmed from `source`).
fn parse_printer_spec(spec: &str) -> AppResult<PrinterInput> {
    let mut profile_path = None;
    let mut source = None;
    let mut name_override = None;
    let mut preparation_minutes = 0u64;
    let mut preparation_sweep = None;
    let mut plates_output = None;
    for field in spec.split(',') {
        let (key, value) = field
            .split_once('=')
            .ok_or_else(|| format!("--printer field must be key=value: {field}"))?;
        match key {
            "profile" => profile_path = Some(PathBuf::from(value)),
            "source" => source = Some(PathBuf::from(value)),
            "name" => name_override = Some(value.to_owned()),
            "prep-min" => preparation_minutes = value.parse()?,
            "prep-sweep" => {
                preparation_sweep = Some(
                    value
                        .split(':')
                        .map(str::parse)
                        .collect::<Result<Vec<u64>, _>>()
                        .map_err(|_| format!("bad prep-sweep value: {value}"))?,
                )
            }
            "plates-3mf" => plates_output = Some(PathBuf::from(value)),
            other => return Err(format!("unknown --printer field: {other}").into()),
        }
    }
    let profile_path = profile_path.ok_or("--printer requires profile=<path>")?;
    let source = source.ok_or("--printer requires source=<path>")?;
    let mut profile = load_printer_profile(&profile_path)?;
    if let Some(name) = name_override {
        profile.printer_name = name;
    }
    let preparation_sweep_minutes = preparation_sweep.unwrap_or_else(|| vec![preparation_minutes]);
    Ok(PrinterInput {
        profile,
        source,
        preparation_minutes,
        preparation_sweep_minutes,
        plates_output,
    })
}

/// Parses `HH:MM` into seconds since the start of the planning day. Hours may run past 24 so the
/// horizon can reach into the next day (e.g. `34:00` = 10:00 the following morning).
fn parse_hhmm(text: &str) -> AppResult<u64> {
    let (hours, minutes) = text
        .split_once(':')
        .ok_or_else(|| format!("time must be HH:MM: {text}"))?;
    let hours: u64 = hours
        .parse()
        .map_err(|_| format!("time must be HH:MM: {text}"))?;
    let minutes: u64 = minutes
        .parse()
        .map_err(|_| format!("time must be HH:MM: {text}"))?;
    if hours > 47 || minutes > 59 {
        return Err(format!("time out of range: {text}").into());
    }
    Ok(hours * 3600 + minutes * 60)
}

fn parse_args() -> AppResult<Args> {
    let mut printers = Vec::new();
    let mut output_schedule = None;
    let mut output_report = None;
    let mut turnaround_minutes = 10u64;
    let mut day_start = 10 * 3600u64;
    let mut night_start = 23 * 3600u64;
    let mut horizon_end = 34 * 3600u64;
    let mut bench_mode = BenchMode::Adaptive;
    let mut reuse_benchmarks = false;
    let cli: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index < cli.len() {
        if cli[index] == "--reuse-benchmarks" {
            reuse_benchmarks = true;
            index += 1;
            continue;
        }
        if index + 1 >= cli.len() {
            return Err(format!("missing value for {}", cli[index]).into());
        }
        match cli[index].as_str() {
            "--printer" => printers.push(parse_printer_spec(&cli[index + 1])?),
            "--output-schedule" => output_schedule = Some(PathBuf::from(&cli[index + 1])),
            "--output-report" => output_report = Some(PathBuf::from(&cli[index + 1])),
            "--turnaround-min" => turnaround_minutes = cli[index + 1].parse()?,
            "--day-start" => day_start = parse_hhmm(&cli[index + 1])?,
            "--night-start" => night_start = parse_hhmm(&cli[index + 1])?,
            "--horizon-end" => horizon_end = parse_hhmm(&cli[index + 1])?,
            "--bench-mode" => {
                bench_mode = match cli[index + 1].as_str() {
                    "adaptive" => BenchMode::Adaptive,
                    "full" => BenchMode::Full,
                    other => return Err(format!("unknown --bench-mode: {other} (adaptive|full)").into()),
                }
            }
            option => return Err(format!("unknown option: {option}").into()),
        }
        index += 2;
    }
    if printers.is_empty() {
        return Err(
            "at least one --printer profile=<profile.json>,source=<packed.3mf> is required".into(),
        );
    }
    let output_schedule =
        output_schedule.ok_or("--output-schedule <path to schedule.csv> is required")?;
    if day_start >= night_start || night_start >= horizon_end {
        return Err(format!(
            "time windows must satisfy day-start < night-start < horizon-end (got {} < {} < {})",
            clock_label(day_start),
            clock_label(night_start),
            clock_label(horizon_end)
        )
        .into());
    }
    for printer in &printers {
        if !printer.source.exists() {
            return Err(format!("source does not exist: {}", printer.source.display()).into());
        }
        if !Path::new(&printer.profile.slicer_path).exists() {
            return Err(format!(
                "slicer does not exist: {}",
                printer.profile.slicer_path
            )
            .into());
        }
    }
    Ok(Args {
        printers,
        output_schedule,
        output_report,
        turnaround_minutes,
        day_start,
        night_start,
        horizon_end,
        bench_mode,
        reuse_benchmarks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_collapses_separators() {
        assert_eq!(slugify("Bambu Lab P1S"), "bambu_lab_p1s");
        assert_eq!(slugify("Snapmaker U1 #2"), "snapmaker_u1_2");
        assert_eq!(slugify("  leading/trailing  "), "leading_trailing");
    }

    #[test]
    fn parse_hhmm_accepts_next_day_hours() {
        assert_eq!(parse_hhmm("10:00").unwrap(), 36_000);
        assert_eq!(parse_hhmm("23:30").unwrap(), 84_600);
        assert_eq!(parse_hhmm("34:00").unwrap(), 122_400);
        assert_eq!(parse_hhmm("9:00").unwrap(), 32_400, "single-digit hour is fine");
        assert!(parse_hhmm("25:99").is_err());
        assert!(parse_hhmm("48:00").is_err());
        assert!(parse_hhmm("10am").is_err());
    }

    #[test]
    fn initial_sample_covers_endpoints_and_shrinks_with_budget() {
        assert_eq!(
            initial_sample_counts(30, 10),
            vec![1, 6, 15, 24, 30],
            "30-object plate samples 5 sizes: both endpoints and 3 interior"
        );
        assert_eq!(
            initial_sample_counts(5, 5),
            vec![1, 2, 3, 4, 5],
            "budget covering every size degenerates to the full sweep"
        );
        assert_eq!(
            initial_sample_counts(7, 3),
            vec![1, 6, 7],
            "tight budget keeps the endpoints and the best interior point"
        );
    }

    #[test]
    fn adaptive_budget_scales_with_plate_size() {
        assert_eq!(adaptive_budget(6), 6, "small plates measure everything");
        assert_eq!(adaptive_budget(20), 6);
        assert_eq!(adaptive_budget(30), 10, "a third of a 30-object plate");
    }

    #[test]
    fn batch_curve_interpolates_and_clamps_at_the_envelope() {
        let curve = BatchCurve::new(&[(1, 100.0), (10, 1000.0)]).unwrap();
        assert!((curve.value(1) - 100.0).abs() < 1e-9);
        assert!((curve.value(10) - 1000.0).abs() < 1e-9);
        assert!((curve.value(5) - 500.0).abs() < 1e-9);
        assert!((curve.value(0) - 100.0).abs() < 1e-9, "clamped below");
        assert!((curve.value(11) - 1000.0).abs() < 1e-9, "clamped above");

        let multi = BatchCurve::new(&[(30, 3000.0), (1, 100.0), (15, 1450.0)]).unwrap();
        // Between (1, 100) and (15, 1450): 100 + 1350 * 9/14.
        assert!((multi.value(10) - 967.8571428571429).abs() < 1e-9, "unsorted input");
    }

    #[test]
    fn refinement_targets_curvature_and_rests_on_affine_data() {
        // Perfectly affine data: the two fits agree everywhere, so no refinement is warranted.
        let affine: Vec<(usize, f64)> = (1..=30)
            .map(|count| (count, 600.0 + 120.0 * count as f64))
            .filter(|(count, _)| [1usize, 6, 15, 24, 30].contains(count))
            .collect();
        let curve = BatchCurve::new(&affine).unwrap();
        let line = least_squares_line(&affine);
        let (_, deviation) = next_refinement_count(&[1, 6, 15, 24, 30], 30, &curve, line).unwrap();
        assert!(
            deviation < REFINE_TOLERANCE,
            "affine data needs no refinement, deviation was {deviation}"
        );

        // A kink in the middle (tower flush makes the per-object cost jump after 15): the
        // refinement should point at an unmeasured count near the kink, not at a quiet region.
        let kinked: Vec<(usize, f64)> = [1usize, 6, 15, 24, 30]
            .iter()
            .map(|&count| {
                let value = if count <= 15 {
                    600.0 + 120.0 * count as f64
                } else {
                    600.0 + 120.0 * 15.0 + 200.0 * (count - 15) as f64
                };
                (count, value)
            })
            .collect();
        let curve = BatchCurve::new(&kinked).unwrap();
        let line = least_squares_line(&kinked);
        let (count, deviation) = next_refinement_count(&[1, 6, 15, 24, 30], 30, &curve, line).unwrap();
        assert!(deviation > REFINE_TOLERANCE, "curved data must want refinement");
        assert!(
            (7..=23).contains(&count),
            "refinement should target the kink region, got {count}"
        );
    }

    #[test]
    fn parse_printer_spec_reads_profile_and_defaults_name_and_prep() {
        let dir = tempfile::tempdir().unwrap();
        let profile_path = dir.path().join("p1s.json");
        fs::write(
            &profile_path,
            fs::read("profiles/p1s.json").unwrap(),
        )
        .unwrap();
        let spec = format!(
            "profile={},source=some.3mf",
            profile_path.display()
        );
        let printer = parse_printer_spec(&spec).unwrap();
        assert_eq!(printer.profile.printer_name, "Bambu Lab P1S");
        assert_eq!(printer.source, PathBuf::from("some.3mf"));
        assert_eq!(printer.preparation_minutes, 0);
        assert_eq!(printer.preparation_sweep_minutes, vec![0]);
    }

    #[test]
    fn parse_printer_spec_applies_name_override_and_prep_sweep() {
        let dir = tempfile::tempdir().unwrap();
        let profile_path = dir.path().join("u1.json");
        fs::write(
            &profile_path,
            fs::read("profiles/snapmaker_u1.json").unwrap(),
        )
        .unwrap();
        let spec = format!(
            "profile={},source=some.3mf,name=U1 #2,prep-min=11,prep-sweep=6:8:10:11:12:15",
            profile_path.display()
        );
        let printer = parse_printer_spec(&spec).unwrap();
        assert_eq!(printer.profile.printer_name, "U1 #2");
        assert_eq!(printer.preparation_minutes, 11);
        assert_eq!(
            printer.preparation_sweep_minutes,
            vec![6, 8, 10, 11, 12, 15]
        );
    }

    #[test]
    fn parse_printer_spec_rejects_missing_source() {
        let dir = tempfile::tempdir().unwrap();
        let profile_path = dir.path().join("p1s.json");
        fs::write(
            &profile_path,
            fs::read("profiles/p1s.json").unwrap(),
        )
        .unwrap();
        let spec = format!("profile={}", profile_path.display());
        let error = parse_printer_spec(&spec).unwrap_err();
        assert!(error.to_string().contains("source"));
    }
}

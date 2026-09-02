use std::{
    collections::HashSet,
    error::Error,
    ffi::OsStr,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use goat_packer::{
    colored_packer::{PrinterProfile, load_printer_profile},
    scheduler::{
        BatchTiming, OptimizedSchedule, PlateSpec, ScheduleConfig, SliceEstimate,
        build_schedule_plates_3mf, object_count_3mf, optimize_schedule, parse_bambu_result,
        parse_orca_gcode, plate_count_3mf, subset_3mf,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Debug)]
struct PrinterInput {
    profile: PrinterProfile,
    source: PathBuf,
    preparation_minutes: u64,
    preparation_sweep_minutes: Vec<u64>,
    plates_output: Option<PathBuf>,
}

#[derive(Debug)]
struct Args {
    printers: Vec<PrinterInput>,
    output_schedule: PathBuf,
    output_report: Option<PathBuf>,
    turnaround_minutes: u64,
    reuse_benchmarks: bool,
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
                work_dir,
            )?);
        }
    }

    let config = ScheduleConfig {
        day_start_seconds: 10 * 3600,
        night_start_seconds: 23 * 3600,
        horizon_end_seconds: 34 * 3600,
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

    let sensitivity = build_sensitivity(&benchmarks, &args.printers, &counts)?;
    let combined_objects_per_day = printers_out
        .iter()
        .map(|printer| printer.variable_batch_schedule.total_objects)
        .sum();
    let report = PlannerReport {
        assumptions: serde_json::json!({
            "day_start": "10:00",
            "fixed_night_start": "23:00",
            "horizon_end_next_day": "10:00",
            "turnaround_minutes": args.turnaround_minutes,
            "printer_preparation_minutes": args.printers.iter().map(|printer| serde_json::json!({
                "printer": printer.profile.printer_name,
                "preparation_minutes": printer.preparation_minutes,
                "preparation_sweep_minutes": printer.preparation_sweep_minutes,
            })).collect::<Vec<_>>(),
            "schedule_semantics": "Day jobs and their final turnaround must finish by 23:00. Exactly one final overnight job starts at 23:00; no jobs start during the night. The overnight job must finish by 10:00 next day."
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
/// plate, not one per run, since it's the same physical layout either way. Labeled `"N pcs"` to
/// match BambuStudio's own plate-name convention for multi-plate projects.
fn plate_specs_for_schedule(schedule: &OptimizedSchedule) -> Vec<PlateSpec> {
    let mut seen = HashSet::new();
    let mut specs = Vec::new();
    for job in &schedule.jobs {
        if seen.insert(job.batch_size) {
            specs.push(PlateSpec {
                label: format!("{} pcs", job.batch_size),
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
    work_dir: &Path,
) -> AppResult<Vec<BenchmarkRecord>> {
    let mut records = Vec::new();
    for count in 1..=max_count {
        println!("[{printer}] slicing batch {count}/{max_count}");
        let subset = work_dir.join(format!("{slug}_{count}.3mf"));
        subset_3mf(source, &subset, count, blank_start_gcode)?;
        let slice_dir = work_dir.join(format!("{slug}_{count}_slice"));
        fs::create_dir_all(&slice_dir)?;
        let output = Command::new(slicer)
            .arg("--slice")
            .arg("1")
            .arg("--outputdir")
            .arg(&slice_dir)
            .arg(&subset)
            .output()?;
        fs::write(
            work_dir.join(format!("{slug}_{count}.log")),
            [&output.stdout[..], &output.stderr[..]].concat(),
        )?;
        if !output.status.success() {
            return Err(format!(
                "{printer} slicer failed for {count} objects with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
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
) -> AppResult<Vec<SensitivityRow>> {
    let mut rows = Vec::new();
    for turnaround in [0, 5, 10, 15, 20] {
        let config = standard_config(turnaround);
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

fn standard_config(turnaround_minutes: u64) -> ScheduleConfig {
    ScheduleConfig {
        day_start_seconds: 10 * 3600,
        night_start_seconds: 23 * 3600,
        horizon_end_seconds: 34 * 3600,
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

fn parse_args() -> AppResult<Args> {
    let mut printers = Vec::new();
    let mut output_schedule = None;
    let mut output_report = None;
    let mut turnaround_minutes = 10u64;
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

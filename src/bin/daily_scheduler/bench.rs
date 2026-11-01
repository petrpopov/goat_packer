//! Benchmarking a printer: slicing batches of the packed source and recording their times.

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use goat_packer::{
    print_time::{TimeCorrection, corrected_seconds},
    scheduler::{
        BatchCurve, BatchTiming, SliceEstimate, least_squares_line, next_refinement_count,
        parse_bambu_result, parse_orca_gcode, subset_3mf,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AppResult;
use crate::args::BenchMode;

/// Relative disagreement between the piecewise-linear fit and the affine least-squares fit at
/// which the adaptive benchmark stops adding slices. 2% is far below the granularity the DP acts
/// on (batch decisions jump in whole objects; a 2% timing error rarely flips a batch size).
pub const REFINE_TOLERANCE: f64 = 0.02;

/// Adaptive benchmarking measures at most this many sizes (initial samples + refinements). For
/// small plates the budget covers everything and the mode degenerates to the full sweep.
pub fn adaptive_budget(max_count: usize) -> usize {
    (max_count / 3).max(6).min(max_count)
}

/// Interior sample sizes of the initial adaptive sweep, in tenths of the full plate.
const SAMPLE_TENTHS: [usize; 3] = [2, 5, 8];

/// Inputs of the AMS change correction for one printer: what the source project's slicer
/// settings assume per change, and the real duration (`None` = no correction).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChangeCorrection {
    pub slicer_change_s: f64,
    pub real_change_s: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum BenchmarkOrigin {
    Measured,
    Interpolated,
}

fn measured_origin() -> BenchmarkOrigin {
    BenchmarkOrigin::Measured
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BenchmarkRecord {
    pub printer: String,
    pub batch_size: usize,
    pub model_seconds: u64,
    pub slicer_total_seconds: u64,
    pub preparation_seconds: u64,
    pub job_seconds: u64,
    pub filament_grams: f64,
    pub units_per_job_hour: f64,
    pub source_3mf: String,
    pub source_sha256: String,
    pub slicer: String,
    /// Whether this row came from a real slice or from interpolation between two real slices.
    /// Absent in reports written before adaptive benchmarking existed; those rows are all
    /// measured, which is what the default encodes.
    #[serde(default = "measured_origin")]
    pub origin: BenchmarkOrigin,
    /// Filament changes in the slice (interpolated rows: interpolated count). Absent in reports
    /// written before the AMS correction existed; 0 means no correction can be derived.
    #[serde(default)]
    pub filament_changes: u64,
    /// `changes * (real - slicer assumption)`, already included in `job_seconds` and in the
    /// timing the optimiser sees; `slicer_total_seconds` stays the raw slicer value. Signed: a
    /// project assuming a slower AMS than the real one gets a negative correction.
    #[serde(default)]
    pub change_correction_seconds: i64,
}

/// `--reuse-benchmarks` with an active AMS correction over rows that all count 0 changes: a
/// report written before change counts existed deserializes them as 0 (`serde(default)`), so the
/// correction silently does nothing. Genuinely change-free plates (single colour) look the same,
/// hence a note, not an error.
pub fn reused_rows_lack_change_counts(rows: &[BenchmarkRecord], correction_active: bool) -> bool {
    correction_active && !rows.is_empty() && rows.iter().all(|row| row.filament_changes == 0)
}

/// What is benchmarked: one printer's packed source and the slicer that times it.
pub struct BenchTarget<'a> {
    pub printer: &'a str,
    /// Filesystem-safe printer name for the work-dir files.
    pub slug: String,
    pub source: &'a Path,
    /// Objects on the full plate.
    pub max_count: usize,
    pub slicer: &'a Path,
    pub slicer_kind: &'a str,
    pub blank_start_gcode: bool,
    pub preparation_seconds: u64,
    pub change: ChangeCorrection,
    pub source_hash: &'a str,
}

/// One record per batch size 1..=max_count (`Full`: every size sliced; `Adaptive`: a sample
/// sliced and the rest interpolated).
pub fn benchmark_printer(
    target: &BenchTarget,
    bench_mode: BenchMode,
    work_dir: &Path,
) -> AppResult<Vec<BenchmarkRecord>> {
    match bench_mode {
        BenchMode::Full => {
            let mut records = Vec::new();
            for count in 1..=target.max_count {
                println!(
                    "[{}] slicing batch {count}/{}",
                    target.printer, target.max_count
                );
                let estimate = slice_one_count(target, count, work_dir)?;
                records.push(record_from_estimate(target, count, estimate));
            }
            Ok(records)
        }
        BenchMode::Adaptive => adaptive_benchmarks(target, work_dir),
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
fn adaptive_benchmarks(target: &BenchTarget, work_dir: &Path) -> AppResult<Vec<BenchmarkRecord>> {
    let measured = measure_samples(target, work_dir)?;
    let measured_count = measured.len();
    println!(
        "[{}] {measured_count} measured sizes cover batches 1..={} ({:.0}% fewer slices than a full sweep)",
        target.printer,
        target.max_count,
        (1.0 - measured_count as f64 / target.max_count as f64) * 100.0
    );
    records_from_measured(target, &measured)
}

/// Slices the initial sample, then refines where the curve bends.
fn measure_samples(
    target: &BenchTarget,
    work_dir: &Path,
) -> AppResult<Vec<(usize, SliceEstimate)>> {
    let printer = target.printer;
    let max_count = target.max_count;
    let budget = adaptive_budget(max_count);
    let counts = initial_sample_counts(max_count, budget);
    let mut measured: Vec<(usize, SliceEstimate)> = Vec::new();
    for (index, &count) in counts.iter().enumerate() {
        println!(
            "[{printer}] slicing sample {}/{}: batch {count}/{max_count}",
            index + 1,
            counts.len()
        );
        measured.push((count, slice_one_count(target, count, work_dir)?));
    }

    while measured.len() < budget {
        let seconds: Vec<(usize, f64)> = measured
            .iter()
            .map(|(count, estimate)| (*count, estimate.slicer_total_seconds as f64))
            .collect();
        let line = least_squares_line(&seconds);
        let known: Vec<usize> = measured.iter().map(|(count, _)| *count).collect();
        let Some((count, deviation)) =
            next_refinement_count(&known, max_count, &BatchCurve::new(&seconds)?, line)
        else {
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
        measured.push((count, slice_one_count(target, count, work_dir)?));
        measured.sort_by_key(|(count, _)| *count);
    }
    Ok(measured)
}

/// A record for every batch size: measured ones as sliced, the others interpolated.
fn records_from_measured(
    target: &BenchTarget,
    measured: &[(usize, SliceEstimate)],
) -> AppResult<Vec<BenchmarkRecord>> {
    let curve = |value: fn(&SliceEstimate) -> f64| {
        BatchCurve::new(
            &measured
                .iter()
                .map(|(count, estimate)| (*count, value(estimate)))
                .collect::<Vec<_>>(),
        )
    };
    let model_curve = curve(|estimate| estimate.model_seconds as f64)?;
    let seconds_curve = curve(|estimate| estimate.slicer_total_seconds as f64)?;
    let grams_curve = curve(|estimate| estimate.filament_grams)?;
    let changes_curve = curve(|estimate| estimate.filament_changes as f64)?;

    let mut records = Vec::with_capacity(target.max_count);
    for count in 1..=target.max_count {
        if let Some((_, estimate)) = measured
            .iter()
            .find(|(measured_count, _)| *measured_count == count)
        {
            records.push(record_from_estimate(target, count, estimate.clone()));
            continue;
        }
        let interpolated = SliceEstimate {
            object_count: count,
            model_seconds: model_curve.value(count).round() as u64,
            slicer_total_seconds: seconds_curve.value(count).round() as u64,
            filament_grams: grams_curve.value(count),
            filament_changes: changes_curve.value(count).max(0.0).round() as u64,
        };
        let mut record = record_from_estimate(target, count, interpolated);
        record.origin = BenchmarkOrigin::Interpolated;
        records.push(record);
    }
    Ok(records)
}

/// Initial sample for the adaptive benchmark: the endpoints (1 and the full plate) plus evenly
/// spaced interior sizes, deduplicated and sorted, capped at the slice budget. When the budget
/// already covers every size, this is simply the full sweep.
pub fn initial_sample_counts(max_count: usize, budget: usize) -> Vec<usize> {
    if budget >= max_count {
        return (1..=max_count).collect();
    }
    let mut counts = vec![1, max_count];
    for tenths in SAMPLE_TENTHS {
        let count = ((max_count as f64) * (tenths as f64) / 10.0).round() as usize;
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

/// Slices the first `count` objects of the source and reads the slicer's estimate.
fn slice_one_count(
    target: &BenchTarget,
    count: usize,
    work_dir: &Path,
) -> AppResult<SliceEstimate> {
    let (printer, slug) = (target.printer, &target.slug);
    let subset = work_dir.join(format!("{slug}_{count}.3mf"));
    subset_3mf(target.source, &subset, count, target.blank_start_gcode)?;
    let slice_dir = work_dir.join(format!("{slug}_{count}_slice"));
    fs::create_dir_all(&slice_dir)?;
    let output = goat_packer::process_output::bounded_output(
        &mut goat_packer::slicer_cli::slice_command(target.slicer, &[], &slice_dir, &subset),
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
    let estimate = if target.slicer_kind == "bambu" {
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

pub fn record_from_estimate(
    target: &BenchTarget,
    count: usize,
    estimate: SliceEstimate,
) -> BenchmarkRecord {
    let mut record = BenchmarkRecord {
        printer: target.printer.to_owned(),
        batch_size: count,
        model_seconds: estimate.model_seconds,
        slicer_total_seconds: estimate.slicer_total_seconds,
        preparation_seconds: target.preparation_seconds,
        job_seconds: 0,
        filament_grams: estimate.filament_grams,
        units_per_job_hour: 0.0,
        source_3mf: target.source.display().to_string(),
        source_sha256: target.source_hash.to_owned(),
        slicer: target.slicer.display().to_string(),
        origin: BenchmarkOrigin::Measured,
        filament_changes: estimate.filament_changes,
        change_correction_seconds: 0,
    };
    apply_change_correction(&mut record, target.change);
    record
}

/// (Re)computes the change correction of `record` and everything derived from it. Margin and
/// prep stay out of `corrected_seconds` here: preparation is its own column the sensitivity
/// sweep varies, and the scheduler has no margin.
pub fn apply_change_correction(record: &mut BenchmarkRecord, change: ChangeCorrection) {
    let raw = record.slicer_total_seconds as f64;
    let corrected = corrected_seconds(
        raw,
        record.filament_changes,
        change.slicer_change_s,
        &TimeCorrection {
            real_change_s: change.real_change_s,
            prep_s: 0.0,
            margin: 0.0,
        },
    );
    record.change_correction_seconds = (corrected - raw).round() as i64;
    record.job_seconds = corrected_slicer_seconds(record) + record.preparation_seconds;
    record.units_per_job_hour = record.batch_size as f64 / (record.job_seconds as f64 / 3600.0);
}

/// Slicer time the optimiser must plan with: the raw estimate plus the change correction.
pub fn corrected_slicer_seconds(record: &BenchmarkRecord) -> u64 {
    record
        .slicer_total_seconds
        .saturating_add_signed(record.change_correction_seconds)
}

pub fn to_timing(record: &BenchmarkRecord) -> BatchTiming {
    BatchTiming::new(
        record.batch_size,
        corrected_slicer_seconds(record),
        record.preparation_seconds,
        record.filament_grams,
    )
}

fn find_extension(directory: &Path, extension: &str) -> AppResult<PathBuf> {
    fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension() == Some(OsStr::new(extension)))
        .ok_or_else(|| format!("no .{extension} file in {}", directory.display()).into())
}

pub fn sha256_file(path: &Path) -> AppResult<String> {
    let mut hasher = Sha256::new();
    hasher.update(fs::read(path)?);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Turns a printer display name into a filesystem-safe slug for benchmark work-dir filenames
/// (e.g. "Bambu Lab P1S" -> "bambu_lab_p1s"): lowercased, non-alphanumeric runs collapsed to a
/// single `_`, no leading/trailing `_`.
pub fn slugify(name: &str) -> String {
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
        assert!(
            (multi.value(10) - 967.8571428571429).abs() < 1e-9,
            "unsorted input"
        );
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
        let (count, deviation) =
            next_refinement_count(&[1, 6, 15, 24, 30], 30, &curve, line).unwrap();
        assert!(
            deviation > REFINE_TOLERANCE,
            "curved data must want refinement"
        );
        assert!(
            (7..=23).contains(&count),
            "refinement should target the kink region, got {count}"
        );
    }

    fn estimate(changes: u64) -> SliceEstimate {
        SliceEstimate {
            object_count: 5,
            model_seconds: 9_000,
            slicer_total_seconds: 10_000,
            filament_grams: 50.0,
            filament_changes: changes,
        }
    }

    fn record(changes: u64, change: ChangeCorrection) -> BenchmarkRecord {
        let target = BenchTarget {
            printer: "P",
            slug: "p".to_owned(),
            source: Path::new("a.3mf"),
            max_count: 5,
            slicer: Path::new("slicer"),
            slicer_kind: "bambu",
            blank_start_gcode: false,
            preparation_seconds: 600,
            change,
            source_hash: "hash",
        };
        record_from_estimate(&target, 5, estimate(changes))
    }

    #[test]
    fn reused_rows_without_change_counts_are_flagged_only_when_a_correction_applies() {
        let change = ChangeCorrection {
            slicer_change_s: 23.0,
            real_change_s: Some(95.0),
        };
        let old_report = vec![record(0, change), record(0, change)];
        assert!(reused_rows_lack_change_counts(&old_report, true));
        assert!(!reused_rows_lack_change_counts(&old_report, false));
        let new_report = vec![record(0, change), record(12, change)];
        assert!(!reused_rows_lack_change_counts(&new_report, true));
        assert!(!reused_rows_lack_change_counts(&[], true));
    }

    #[test]
    fn record_adds_change_correction_to_job_and_timing() {
        let change = ChangeCorrection {
            slicer_change_s: 23.0,
            real_change_s: Some(95.0),
        };
        let record = record(100, change);
        assert_eq!(record.filament_changes, 100);
        assert_eq!(record.change_correction_seconds, 7_200);
        assert_eq!(
            record.slicer_total_seconds, 10_000,
            "raw slicer value is kept"
        );
        assert_eq!(record.job_seconds, 10_000 + 7_200 + 600);
        let timing = to_timing(&record);
        assert_eq!(timing.slicer_seconds, 17_200);
        assert_eq!(timing.job_seconds(), 17_800);
    }

    #[test]
    fn record_without_correction_is_unchanged_and_negative_correction_applies() {
        let off = record(
            100,
            ChangeCorrection {
                slicer_change_s: 23.0,
                real_change_s: None,
            },
        );
        assert_eq!(off.change_correction_seconds, 0);
        assert_eq!(off.job_seconds, 10_600);
        let faster = record(
            10,
            ChangeCorrection {
                slicer_change_s: 57.0,
                real_change_s: Some(30.0),
            },
        );
        assert_eq!(faster.change_correction_seconds, -270);
        assert_eq!(faster.job_seconds, 10_330);
        assert_eq!(to_timing(&faster).slicer_seconds, 9_730);
    }

    #[test]
    fn reused_records_are_recorrected_with_current_settings() {
        let mut reused = record(
            100,
            ChangeCorrection {
                slicer_change_s: 23.0,
                real_change_s: None,
            },
        );
        apply_change_correction(
            &mut reused,
            ChangeCorrection {
                slicer_change_s: 23.0,
                real_change_s: Some(95.0),
            },
        );
        assert_eq!(reused.change_correction_seconds, 7_200);
        assert_eq!(reused.job_seconds, 17_800);
    }

    #[test]
    fn old_benchmark_records_without_change_fields_still_load() {
        let old = r#"{
            "printer": "Bambu Lab P1S", "batch_size": 3, "model_seconds": 100,
            "slicer_total_seconds": 120, "preparation_seconds": 60, "job_seconds": 180,
            "filament_grams": 1.5, "units_per_job_hour": 60.0, "source_3mf": "a.3mf",
            "source_sha256": "x", "slicer": "s"
        }"#;
        let record: BenchmarkRecord = serde_json::from_str(old).unwrap();
        assert_eq!(record.filament_changes, 0);
        assert_eq!(record.change_correction_seconds, 0);
        assert_eq!(record.origin, BenchmarkOrigin::Measured);
        assert_eq!(to_timing(&record).slicer_seconds, 120);
    }
}

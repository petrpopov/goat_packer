//! Command line of `daily_scheduler`.

use std::path::{Path, PathBuf};

use goat_packer::colored_packer::{PrinterProfile, load_printer_profile};

use crate::AppResult;
use crate::report::clock_label;

const SECONDS_PER_HOUR: u64 = 3600;
const DEFAULT_TURNAROUND_MINUTES: u64 = 10;
/// The planning day: day jobs from 10:00, the one overnight job at 23:00, everything done by
/// 10:00 the next morning (`34:00`).
const DEFAULT_DAY_START_HOURS: u64 = 10;
const DEFAULT_NIGHT_START_HOURS: u64 = 23;
const DEFAULT_HORIZON_END_HOURS: u64 = 34;
/// `HH:MM` hours may run into the next day, but not past this.
const MAX_CLOCK_HOURS: u64 = 47;
/// Upper bound of `change-s=`, as for the packer's `--change-seconds`.
const MAX_CHANGE_SECONDS: f64 = 600.0;

#[derive(Clone, Debug)]
pub struct PrinterInput {
    pub profile: PrinterProfile,
    pub source: PathBuf,
    pub preparation_minutes: u64,
    pub preparation_sweep_minutes: Vec<u64>,
    pub plates_output: Option<PathBuf>,
    /// `change-s=` override of the profile's `filament_change_seconds`.
    pub change_seconds: Option<f64>,
}

impl PrinterInput {
    /// Real change duration to use for this printer: the `change-s=` key wins (and, like the
    /// packer's `--change-seconds`, implies the correction); otherwise the profile value when
    /// `--ams-correction` is on. A profile without the value (tool changer) never corrects.
    pub fn real_change_seconds(&self, ams_correction: bool) -> Option<f64> {
        let profile_value = if ams_correction {
            self.profile.filament_change_seconds
        } else {
            None
        };
        self.change_seconds.or(profile_value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchMode {
    /// Slice only a sample of batch sizes and interpolate the rest (default).
    Adaptive,
    /// Slice every size from 1 to N — the original behaviour, kept for validating interpolation.
    Full,
}

#[derive(Debug)]
pub struct Args {
    pub printers: Vec<PrinterInput>,
    pub output_schedule: PathBuf,
    pub output_report: Option<PathBuf>,
    pub turnaround_minutes: u64,
    pub day_start: u64,
    pub night_start: u64,
    pub horizon_end: u64,
    pub bench_mode: BenchMode,
    pub reuse_benchmarks: bool,
    pub ams_correction: bool,
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
pub fn parse_printer_spec(spec: &str) -> AppResult<PrinterInput> {
    let mut profile_path = None;
    let mut source = None;
    let mut name_override = None;
    let mut preparation_minutes = 0u64;
    let mut preparation_sweep = None;
    let mut plates_output = None;
    let mut change_seconds = None;
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
            "change-s" => {
                let seconds: f64 = value
                    .parse()
                    .map_err(|_| format!("bad change-s value: {value}"))?;
                if !seconds.is_finite() || !(0.0..=MAX_CHANGE_SECONDS).contains(&seconds) {
                    return Err(format!("change-s must be in [0, 600]; got {value}").into());
                }
                change_seconds = Some(seconds);
            }
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
        change_seconds,
    })
}

/// Parses `HH:MM` into seconds since the start of the planning day. Hours may run past 24 so the
/// horizon can reach into the next day (e.g. `34:00` = 10:00 the following morning).
pub fn parse_hhmm(text: &str) -> AppResult<u64> {
    let (hours, minutes) = text
        .split_once(':')
        .ok_or_else(|| format!("time must be HH:MM: {text}"))?;
    let hours: u64 = hours
        .parse()
        .map_err(|_| format!("time must be HH:MM: {text}"))?;
    let minutes: u64 = minutes
        .parse()
        .map_err(|_| format!("time must be HH:MM: {text}"))?;
    if hours > MAX_CLOCK_HOURS || minutes > 59 {
        return Err(format!("time out of range: {text}").into());
    }
    Ok(hours * SECONDS_PER_HOUR + minutes * 60)
}

pub fn parse_args() -> AppResult<Args> {
    parse_args_from(std::env::args().skip(1))
}

pub fn parse_args_from(mut args: impl Iterator<Item = String>) -> AppResult<Args> {
    let mut printers = Vec::new();
    let mut output_schedule = None;
    let mut output_report = None;
    let mut turnaround_minutes = DEFAULT_TURNAROUND_MINUTES;
    let mut day_start = DEFAULT_DAY_START_HOURS * SECONDS_PER_HOUR;
    let mut night_start = DEFAULT_NIGHT_START_HOURS * SECONDS_PER_HOUR;
    let mut horizon_end = DEFAULT_HORIZON_END_HOURS * SECONDS_PER_HOUR;
    let mut bench_mode = BenchMode::Adaptive;
    let mut reuse_benchmarks = false;
    let mut ams_correction = false;
    while let Some(option) = args.next() {
        // The valueless switches; every other option consumes the next argument.
        if option == "--reuse-benchmarks" {
            reuse_benchmarks = true;
            continue;
        }
        if option == "--ams-correction" {
            ams_correction = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--printer" => printers.push(parse_printer_spec(&value)?),
            "--output-schedule" => output_schedule = Some(PathBuf::from(&value)),
            "--output-report" => output_report = Some(PathBuf::from(&value)),
            "--turnaround-min" => turnaround_minutes = value.parse()?,
            "--day-start" => day_start = parse_hhmm(&value)?,
            "--night-start" => night_start = parse_hhmm(&value)?,
            "--horizon-end" => horizon_end = parse_hhmm(&value)?,
            "--bench-mode" => {
                bench_mode = match value.as_str() {
                    "adaptive" => BenchMode::Adaptive,
                    "full" => BenchMode::Full,
                    other => {
                        return Err(format!("unknown --bench-mode: {other} (adaptive|full)").into());
                    }
                }
            }
            other => return Err(format!("unknown option: {other}").into()),
        }
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
            return Err(format!("slicer does not exist: {}", printer.profile.slicer_path).into());
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
        ams_correction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_hhmm_accepts_next_day_hours() {
        assert_eq!(parse_hhmm("10:00").unwrap(), 36_000);
        assert_eq!(parse_hhmm("23:30").unwrap(), 84_600);
        assert_eq!(parse_hhmm("34:00").unwrap(), 122_400);
        assert_eq!(
            parse_hhmm("9:00").unwrap(),
            32_400,
            "single-digit hour is fine"
        );
        assert!(parse_hhmm("25:99").is_err());
        assert!(parse_hhmm("48:00").is_err());
        assert!(parse_hhmm("10am").is_err());
    }

    #[test]
    fn parse_printer_spec_reads_profile_and_defaults_name_and_prep() {
        let dir = tempfile::tempdir().unwrap();
        let profile_path = write_profile(dir.path(), "corner_bed.json");
        let spec = format!("profile={},source=some.3mf", profile_path.display());
        let printer = parse_printer_spec(&spec).unwrap();
        assert_eq!(printer.profile.printer_name, "Test Corner Bed");
        assert_eq!(printer.source, PathBuf::from("some.3mf"));
        assert_eq!(printer.preparation_minutes, 0);
        assert_eq!(printer.preparation_sweep_minutes, vec![0]);
    }

    #[test]
    fn parse_printer_spec_applies_name_override_and_prep_sweep() {
        let dir = tempfile::tempdir().unwrap();
        let profile_path = write_profile(dir.path(), "margin_bed.json");
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
        let profile_path = write_profile(dir.path(), "corner_bed.json");
        let spec = format!("profile={}", profile_path.display());
        let error = parse_printer_spec(&spec).unwrap_err();
        assert!(error.to_string().contains("source"));
    }

    fn write_profile(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profiles").join(name);
        fs::write(&path, fs::read(fixture).unwrap()).unwrap();
        path
    }

    #[test]
    fn parse_printer_spec_reads_change_seconds_override() {
        let dir = tempfile::tempdir().unwrap();
        let profile_path = write_profile(dir.path(), "corner_bed.json");
        let plain = parse_printer_spec(&format!("profile={},source=a.3mf", profile_path.display()))
            .unwrap();
        assert_eq!(plain.change_seconds, None);
        let spec = format!(
            "profile={},source=a.3mf,change-s=80",
            profile_path.display()
        );
        assert_eq!(
            parse_printer_spec(&spec).unwrap().change_seconds,
            Some(80.0)
        );
        let bad = format!(
            "profile={},source=a.3mf,change-s=-5",
            profile_path.display()
        );
        assert!(parse_printer_spec(&bad).is_err());
    }

    #[test]
    fn real_change_seconds_follows_flag_key_and_profile() {
        let dir = tempfile::tempdir().unwrap();
        let profile_path = write_profile(dir.path(), "corner_bed.json");
        let plain = parse_printer_spec(&format!("profile={},source=a.3mf", profile_path.display()))
            .unwrap();
        assert_eq!(plain.real_change_seconds(false), None);
        assert_eq!(plain.real_change_seconds(true), Some(91.0));
        let keyed = parse_printer_spec(&format!(
            "profile={},source=a.3mf,change-s=70",
            profile_path.display()
        ))
        .unwrap();
        assert_eq!(keyed.real_change_seconds(true), Some(70.0));
        // The explicit key implies the correction, like the packer's --change-seconds.
        assert_eq!(keyed.real_change_seconds(false), Some(70.0));
        let u1 = write_profile(dir.path(), "margin_bed.json");
        let tool_changer =
            parse_printer_spec(&format!("profile={},source=a.3mf", u1.display())).unwrap();
        assert_eq!(tool_changer.real_change_seconds(true), None);
    }

    #[test]
    fn options_need_their_values_and_the_required_ones() {
        let parse = |list: &[&str]| parse_args_from(list.iter().map(|item| (*item).to_owned()));
        assert!(
            parse(&["--turnaround-min"])
                .unwrap_err()
                .to_string()
                .contains("missing value")
        );
        assert!(
            parse(&["--bogus", "1"])
                .unwrap_err()
                .to_string()
                .contains("unknown option")
        );
        assert!(parse(&[]).unwrap_err().to_string().contains("--printer"));
    }
}

//! Command line of `colored_3mf_packer`.

use std::path::PathBuf;

use goat_packer::{
    colored_packer::PrinterProfile, nozzle_clean::NozzleCleanSpec, print_time::TimeCorrection,
};

use crate::AppResult;

/// Default `--sample-objects`: the centre object and its two nearest neighbours.
pub const SAMPLE_OBJECTS: usize = 3;

const DEFAULT_CLEARANCE_MM: f64 = 0.7;
const DEFAULT_RESOLUTION_MM: f64 = 0.25;
const DEFAULT_ANGLE_STEP_DEG: f64 = 2.0;
const DEFAULT_ATTEMPTS: usize = 16_000;
const DEFAULT_CANDIDATES: usize = 2_400;
const DEFAULT_SEED: u64 = 20_270_831;

/// 10 % margin on the corrected time: the AMS calibration is a lower bound and the per-change
/// cost varies about ±15 % with the interval between changes.
const DEFAULT_TIME_MARGIN_PCT: f64 = 10.0;
/// Upper bounds of the time options (a typo guard, not a physical limit).
const MAX_CHANGE_SECONDS: f64 = 600.0;
const MAX_TIME_MARGIN_PCT: f64 = 50.0;

#[derive(Clone, Debug)]
pub struct Args {
    pub source_3mf: PathBuf,
    pub profile_path: PathBuf,
    pub output: PathBuf,
    pub work_dir: PathBuf,
    pub clearance: f64,
    pub resolution: f64,
    pub angle_step: f64,
    pub attempts: usize,
    pub candidates: usize,
    pub stall: usize,
    pub max_objects: usize,
    pub seed: u64,
    /// Overrides the profile's edge margin (Snapmaker Orca's spiral lifting warns unless the
    /// gap is strictly above 3.5 mm, which a 3.5 margin hits exactly).
    pub edge_margin: Option<f64>,
    /// `search` runs the full deterministic packing; `grid` places the deterministic grid only
    /// and skips the search — faster, perfectly uniform, a bit sparser.
    pub layout: LayoutMode,
    pub nozzle_clean: Option<NozzleCleanSpec>,
    pub time: TimeOptions,
    /// `--support-overlap-mm`: how far the supports of different objects may overlap
    /// (`PackOptions::support_overlap_mm`); `None` = unlimited, the plain two-zone rule.
    pub support_overlap_mm: Option<f64>,
    /// `--sample-objects`: how many objects of the first layout are sliced as the support
    /// sample (`Footprint::layer_zones`); 0 = no sample, the probe's zones only.
    pub sample_objects: usize,
    /// `--no-final-slice` clears it: write the packed 3MF and `<output>.state.json` without
    /// slicing the whole plate; a time window needs the slices and keeps it on.
    pub final_slice: bool,
    /// `--finish-state PATH`: skip probing and packing, slice and check the plate of a state.
    pub finish_state: Option<PathBuf>,
}

/// Time-model options (spec 2026-10-26-time-model-and-window-design). Without
/// `--time-window-hours` they only shape the corrected time written to the report.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeOptions {
    pub window_hours: Option<f64>,
    pub ams_correction: bool,
    /// Overrides the profile's `filament_change_seconds`; setting it implies `ams_correction`.
    pub change_seconds: Option<f64>,
    pub margin_pct: f64,
    pub prep_min: f64,
    /// `--no-centre`: keep the corner sweep and the source tower position on a time-window
    /// plate (spec 2026-10-26-centre-out-zones-tower-design).
    pub no_centre: bool,
}

impl Default for TimeOptions {
    fn default() -> Self {
        Self {
            window_hours: None,
            ams_correction: false,
            change_seconds: None,
            margin_pct: DEFAULT_TIME_MARGIN_PCT,
            prep_min: 0.0,
            no_centre: false,
        }
    }
}

impl TimeOptions {
    /// Centre-out ordering, heat zones and the tower relocation apply only to time-window
    /// plates: a max-count plate keeps today's corner sweep, since a centre tower fragments the
    /// free space and could cost objects.
    pub fn centre_out(&self) -> bool {
        self.window_hours.is_some() && !self.no_centre
    }

    /// The effective correction. A profile without `filament_change_seconds` (tool changer)
    /// has nothing to correct, so the change term stays off even when asked for.
    pub fn correction(&self, profile: &PrinterProfile) -> TimeCorrection {
        let real_change_s = if self.ams_correction {
            self.change_seconds.or(profile.filament_change_seconds)
        } else {
            None
        };
        TimeCorrection {
            real_change_s,
            prep_s: self.prep_min * 60.0,
            margin: self.margin_pct / 100.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayoutMode {
    Search,
    Grid,
}

pub fn parse_args() -> AppResult<Args> {
    parse_args_from(std::env::args().skip(1))
}

/// Parses a time option and rejects non-finite or out-of-range values up front: a NaN window
/// would compare false everywhere and silently disable the fit.
fn parse_time_value(
    option: &str,
    value: &str,
    min: f64,
    max: f64,
    min_exclusive: bool,
) -> AppResult<f64> {
    let parsed: f64 = value
        .parse()
        .map_err(|error| format!("invalid value for {option}: {value} ({error})"))?;
    let above_min = if min_exclusive { parsed > min } else { parsed >= min };
    if !parsed.is_finite() || !above_min || parsed > max {
        let low = if min_exclusive { "(" } else { "[" };
        return Err(format!("{option} must be in {low}{min}, {max}]; got {value}").into());
    }
    Ok(parsed)
}

pub fn parse_args_from(mut args: impl Iterator<Item = String>) -> AppResult<Args> {
    let mut source_3mf = None;
    let mut profile_path = None;
    let mut output = None;
    let mut work_dir = None;
    let mut clearance = DEFAULT_CLEARANCE_MM;
    let mut resolution = DEFAULT_RESOLUTION_MM;
    let mut angle_step = DEFAULT_ANGLE_STEP_DEG;
    let mut attempts = DEFAULT_ATTEMPTS;
    let mut candidates = DEFAULT_CANDIDATES;
    let mut stall = None;
    let mut max_objects = 0usize;
    let mut edge_margin: Option<f64> = None;
    let mut layout = LayoutMode::Search;
    let mut seed = DEFAULT_SEED;
    let mut nozzle_clean = None;
    let mut time = TimeOptions::default();
    let mut support_overlap_mm = None;
    let mut sample_objects = SAMPLE_OBJECTS;
    let mut final_slice = true;
    let mut finish_state = None;
    while let Some(option) = args.next() {
        // The valueless switches; every other option consumes the next argument.
        if option == "--ams-correction" {
            time.ams_correction = true;
            continue;
        }
        if option == "--no-centre" {
            time.no_centre = true;
            continue;
        }
        if option == "--no-final-slice" {
            final_slice = false;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--source" => source_3mf = Some(PathBuf::from(value)),
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--work-dir" => work_dir = Some(PathBuf::from(value)),
            "--clearance" => clearance = value.parse()?,
            "--resolution" => resolution = value.parse()?,
            "--angle-step" => angle_step = value.parse()?,
            "--attempts" => attempts = value.parse()?,
            "--candidates" => candidates = value.parse()?,
            "--stall" => stall = Some(value.parse()?),
            "--max-objects" => max_objects = value.parse()?,
            "--edge-margin" => edge_margin = Some(value.parse()?),
            "--layout" => {
                layout = match value.as_str() {
                    "search" => LayoutMode::Search,
                    "grid" => LayoutMode::Grid,
                    other => return Err(format!("unknown layout mode: {other}").into()),
                };
            }
            "--seed" => seed = value.parse()?,
            "--nozzle-clean" => {
                nozzle_clean = Some(NozzleCleanSpec::parse(&value)?);
            }
            "--time-window-hours" => {
                time.window_hours =
                    Some(parse_time_value(&option, &value, 0.0, f64::INFINITY, true)?);
            }
            "--change-seconds" => {
                time.change_seconds =
                    Some(parse_time_value(&option, &value, 0.0, MAX_CHANGE_SECONDS, false)?);
                time.ams_correction = true;
            }
            "--time-margin-pct" => {
                time.margin_pct = parse_time_value(&option, &value, 0.0, MAX_TIME_MARGIN_PCT, false)?;
            }
            "--prep-min" => {
                time.prep_min = parse_time_value(&option, &value, 0.0, f64::INFINITY, false)?;
            }
            "--sample-objects" => sample_objects = value.parse()?,
            "--finish-state" => finish_state = Some(PathBuf::from(value)),
            "--support-overlap-mm" => {
                support_overlap_mm =
                    Some(parse_time_value(&option, &value, 0.0, f64::INFINITY, false)?);
            }
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    let output = output.ok_or("--output is required")?;
    let work_dir = work_dir.unwrap_or_else(|| output.with_extension("work"));
    let attempts = attempts.max(1);
    // Compatibility option: the deterministic finite portfolio does not stop on random stalls.
    let stall = stall.unwrap_or(0);
    Ok(Args {
        source_3mf: source_3mf.ok_or("--source is required")?,
        profile_path: profile_path.ok_or("--profile is required")?,
        output,
        work_dir,
        clearance,
        resolution,
        angle_step,
        attempts,
        candidates,
        stall,
        max_objects,
        seed,
        edge_margin,
        layout,
        nozzle_clean,
        time,
        support_overlap_mm,
        sample_objects,
        final_slice,
        finish_state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_profile;

    fn parse(list: &[&str]) -> AppResult<Args> {
        parse_args_from(list.iter().map(|item| (*item).to_owned()))
    }

    const REQUIRED: [&str; 6] = ["--source", "s.3mf", "--profile", "p.json", "--output", "o.3mf"];

    fn parse_with(extra: &[&str]) -> AppResult<Args> {
        let mut list = REQUIRED.to_vec();
        list.extend_from_slice(extra);
        parse(&list)
    }

    #[test]
    fn time_options_default_to_no_window_and_no_correction() {
        let args = parse(&REQUIRED).unwrap();
        assert_eq!(args.time.window_hours, None);
        assert!(!args.time.ams_correction);
        assert_eq!(args.time.change_seconds, None);
        assert_eq!(args.time.margin_pct, 10.0);
        assert_eq!(args.time.prep_min, 0.0);
    }

    #[test]
    fn time_options_parse_including_valueless_ams_flag() {
        let args = parse_with(&[
            "--time-window-hours", "12", "--ams-correction", "--time-margin-pct", "15",
            "--prep-min", "5", "--clearance", "0.9",
        ])
        .unwrap();
        assert_eq!(args.time.window_hours, Some(12.0));
        assert!(args.time.ams_correction);
        assert_eq!(args.time.margin_pct, 15.0);
        assert_eq!(args.time.prep_min, 5.0);
        assert_eq!(args.clearance, 0.9);
    }

    #[test]
    fn support_overlap_defaults_to_unlimited_and_rejects_negatives() {
        assert_eq!(parse(&REQUIRED).unwrap().support_overlap_mm, None);
        assert_eq!(
            parse_with(&["--support-overlap-mm", "0"]).unwrap().support_overlap_mm,
            Some(0.0)
        );
        assert_eq!(
            parse_with(&["--support-overlap-mm", "1.5"]).unwrap().support_overlap_mm,
            Some(1.5)
        );
        for bad in ["-0.5", "NaN", "inf", "x"] {
            assert!(parse_with(&["--support-overlap-mm", bad]).is_err(), "{bad}");
        }
    }

    #[test]
    fn final_slice_and_sample_flags_parse() {
        let defaults = parse(&REQUIRED).unwrap();
        assert!(defaults.final_slice);
        assert_eq!(defaults.finish_state, None);
        assert_eq!(defaults.sample_objects, SAMPLE_OBJECTS);
        assert!(!parse_with(&["--no-final-slice"]).unwrap().final_slice);
        assert_eq!(
            parse_with(&["--finish-state", "out.state.json"]).unwrap().finish_state,
            Some(PathBuf::from("out.state.json"))
        );
        assert_eq!(parse_with(&["--sample-objects", "0"]).unwrap().sample_objects, 0);
        assert!(parse_with(&["--sample-objects", "x"]).is_err());
    }

    #[test]
    fn centre_out_only_with_a_window_and_without_no_centre() {
        assert!(!parse(&REQUIRED).unwrap().time.centre_out());
        assert!(!parse_with(&["--no-centre"]).unwrap().time.centre_out());
        assert!(parse_with(&["--time-window-hours", "12"]).unwrap().time.centre_out());
        let opted_out = parse_with(&["--no-centre", "--time-window-hours", "12"]).unwrap();
        assert!(opted_out.time.no_centre);
        assert!(!opted_out.time.centre_out());
    }

    #[test]
    fn change_seconds_implies_ams_correction() {
        let args = parse_with(&["--change-seconds", "80"]).unwrap();
        assert!(args.time.ams_correction);
        assert_eq!(args.time.change_seconds, Some(80.0));
    }

    #[test]
    fn time_options_reject_out_of_range_values() {
        for bad in [
            &["--time-window-hours", "0"][..],
            &["--time-window-hours", "-1"],
            &["--time-window-hours", "NaN"],
            &["--change-seconds", "-1"],
            &["--change-seconds", "601"],
            &["--time-margin-pct", "-1"],
            &["--time-margin-pct", "51"],
            &["--prep-min", "-1"],
        ] {
            assert!(parse_with(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn time_correction_uses_profile_or_override_only_when_enabled() {
        let mut profile = test_profile();
        profile.filament_change_seconds = Some(95.0);
        let off = parse(&REQUIRED).unwrap().time;
        assert_eq!(off.correction(&profile).real_change_s, None);
        let on = parse_with(&["--ams-correction", "--prep-min", "2"]).unwrap().time;
        let correction = on.correction(&profile);
        assert_eq!(correction.real_change_s, Some(95.0));
        assert_eq!(correction.prep_s, 120.0);
        assert!((correction.margin - 0.1).abs() < 1e-12);
        let overridden = parse_with(&["--change-seconds", "70"]).unwrap().time;
        assert_eq!(overridden.correction(&profile).real_change_s, Some(70.0));
        // A tool changer has no profile value: correction silently stays off.
        assert_eq!(on.correction(&test_profile()).real_change_s, None);
    }
}

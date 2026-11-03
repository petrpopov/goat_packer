//! Command line of `colored_3mf_packer`.

use std::path::PathBuf;

use goat_packer::{
    ForeignGaps,
    colored_packer::{PrinterProfile, SLICER_KINDS},
    nozzle_clean::NozzleCleanSpec,
    print_time::TimeCorrection,
    support_model::merge::MergeOrder,
};

use crate::AppResult;
use crate::gaps::{
    DEFAULT_FOREIGN_GAPS, check_ladder, parse_foreign_gaps, parse_foreign_pair,
};

/// Default `--rotations`: every goat prints one realization per rotation class, and the
/// half-turn class lets neighbours interlock (design §14 Q13).
const DEFAULT_ROTATIONS: &str = "0,180";
const DEFAULT_CLEARANCE_MM: f64 = 0.7;
/// The resolution ladder (`pack::ladder`): the coarse run, then finer ones. Research
/// 2026-11-03: 0.2 / 0.15 mm gained U1 +1 and A2L +3..4 copies, P1S stayed; the whole ladder
/// costs 3 / 3 / 10 s per foreign-gap rung on U1 / P1S / A2L (14 cores). Measured again
/// 2026-11-04 on the full pipeline (rotations 0/180, two rayon threads like a server job):
/// U1 55/56/56, P1S 45/46/46, A2L 79/77/78 at 0.25/0.2/0.15: the 0.15 mm level never added a
/// copy and was the slowest level (P1S 39 of 85 s of packing on the server's two cores), so
/// it is opt-in (`--resolutions 0.25,0.2,0.15`).
const DEFAULT_RESOLUTIONS_MM: [f64; 2] = [0.25, 0.2];
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

/// Where the supports of a plate come from (`--support-source`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SupportSource {
    /// Probe slices of one object per rotation class (the slicer is called).
    #[default]
    Probe,
    /// The clean-room model of the slicers' tree supports (no slicer call).
    Model,
}

impl SupportSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SupportSource::Probe => "probe",
            SupportSource::Model => "model",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Args {
    pub source_3mf: PathBuf,
    pub profile_path: PathBuf,
    pub output: PathBuf,
    pub work_dir: PathBuf,
    pub clearance: f64,
    /// The first (coarsest) resolution of the ladder: the one a layout is reported at when
    /// no finer level beats it.
    pub resolution: f64,
    /// `--resolutions` (`--resolution X` = just `X`): the raster resolutions the packer climbs,
    /// coarse to fine, mm.
    pub resolutions: Vec<f64>,
    pub angle_step: f64,
    pub attempts: usize,
    pub candidates: usize,
    pub stall: usize,
    pub max_objects: usize,
    pub seed: u64,
    /// Overrides the profile's edge margin (Snapmaker Orca's spiral lifting warns unless the
    /// gap is strictly above 3.5 mm, which a 3.5 margin hits exactly).
    pub edge_margin: Option<f64>,
    pub nozzle_clean: Option<NozzleCleanSpec>,
    pub time: TimeOptions,
    /// `--rotations`: the rotation classes (degrees about the frame centre, in `[0, 360)`) the
    /// layout may use; the first is class 0. Supports are predicted per class.
    pub rotations: Vec<f64>,
    /// `--support-source`: probe slices (default) or the support model.
    pub support_source: SupportSource,
    /// `--support-orders`: which readings of the slicers' merge order the support model runs
    /// (`along`, `against`; default `along`, the reading that matches the corpus once the roof
    /// lines run at the build's own angle).
    pub support_orders: Vec<MergeOrder>,
    /// `--support-margins FIRST_LAYER:BAND` (mm): replaces the flavour's calibrated margins.
    pub support_margins: Option<(f64, f64)>,
    /// `--cache-dir`: where the per-class support probes are kept between runs.
    pub cache_dir: Option<PathBuf>,
    /// `--verify` (or a time window, or `--finish-state`): slice the assembled plate and run
    /// the GUI, support and plate checks; a failure is learned once and repacked.
    pub verify: bool,
    /// `--support-overlap-mm`: how far the supports of different objects may overlap
    /// (`PackOptions::support_overlap_mm`); `None` = unlimited, the plain two-zone rule.
    pub support_overlap_mm: Option<f64>,
    /// `--foreign-gaps`: the (support base, support band) gaps to a neighbour's body, tried
    /// one pack each, floor first; none below `foreign_gap_floor`.
    pub foreign_gaps: Vec<ForeignGaps>,
    /// `--foreign-gap-floor`: the least a rung may be. The default is the former fixed rule,
    /// what control full-plate slices passed with zero contacts; lowering it is explicit and
    /// warned about in the progress output and the report.
    pub foreign_gap_floor: ForeignGaps,
    /// `--keep-own-gap`: keep the project's support first-layer gap even when the user never
    /// changed it from the system preset (otherwise it is forced, see `own_gap`).
    pub keep_own_gap: bool,
    /// `--finish-state PATH`: skip modelling and packing, verify the plate of a saved state.
    pub finish_state: Option<PathBuf>,
    /// `--slicer bambu|orca`: the slicer the user prints with, one of the profile's `slicers`
    /// (`PrinterProfile::for_slicer`): it picks the slicer binary and makes that slicer's GUI
    /// the explicit target flavour. `None` = the profile's own `slicer_kind`, and the target
    /// GUI comes from the project's `Application`.
    pub slicer: Option<String>,
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

/// `0,180` as degrees in `[0, 360)`: finite, distinct, at least one.
fn parse_rotations(text: &str) -> AppResult<Vec<f64>> {
    let mut rotations: Vec<f64> = Vec::new();
    for part in text.split(',') {
        let degrees: f64 = part
            .trim()
            .parse()
            .map_err(|error| format!("invalid rotation {part:?} in --rotations: {error}"))?;
        if !degrees.is_finite() {
            return Err(format!("--rotations must be finite; got {part:?}").into());
        }
        let degrees = degrees.rem_euclid(360.0);
        if rotations.contains(&degrees) {
            return Err(format!("--rotations lists {degrees} twice").into());
        }
        rotations.push(degrees);
    }
    Ok(rotations)
}

/// `--resolutions a,b,c`: positive millimetres, at least one.
fn parse_resolutions(list: &str) -> AppResult<Vec<f64>> {
    let mut resolutions = Vec::new();
    for item in list.split(',') {
        let value: f64 = item.trim().parse()?;
        if !value.is_finite() || value <= 0.0 {
            return Err(format!("--resolutions: {item} is not a positive resolution").into());
        }
        resolutions.push(value);
    }
    Ok(resolutions)
}

pub fn parse_args_from(mut args: impl Iterator<Item = String>) -> AppResult<Args> {
    let mut source_3mf = None;
    let mut profile_path = None;
    let mut output = None;
    let mut work_dir = None;
    let mut clearance = DEFAULT_CLEARANCE_MM;
    let mut resolutions = DEFAULT_RESOLUTIONS_MM.to_vec();
    let mut angle_step = DEFAULT_ANGLE_STEP_DEG;
    let mut attempts = DEFAULT_ATTEMPTS;
    let mut candidates = DEFAULT_CANDIDATES;
    let mut stall = None;
    let mut max_objects = 0usize;
    let mut edge_margin: Option<f64> = None;
    let mut rotations = parse_rotations(DEFAULT_ROTATIONS)?;
    let mut cache_dir = None;
    let mut support_source = SupportSource::default();
    let mut support_orders = vec![MergeOrder::Along];
    let mut support_margins = None;
    let mut verify = false;
    let mut seed = DEFAULT_SEED;
    let mut nozzle_clean = None;
    let mut time = TimeOptions::default();
    let mut support_overlap_mm = None;
    let mut finish_state = None;
    let mut slicer = None;
    let mut keep_own_gap = false;
    let mut foreign_gap_floor = ForeignGaps::default();
    let mut foreign_gaps = parse_foreign_gaps(DEFAULT_FOREIGN_GAPS)?;
    while let Some(option) = args.next() {
        // The valueless switches; every other option consumes the next argument.
        if option == "--ams-correction" {
            time.ams_correction = true;
            continue;
        }
        if option == "--keep-own-gap" {
            keep_own_gap = true;
            continue;
        }
        if option == "--no-centre" {
            time.no_centre = true;
            continue;
        }
        if option == "--verify" {
            verify = true;
            continue;
        }
        // The default job no longer slices the plate, so the old switch has nothing to turn
        // off; the webapp and old scripts may still pass it.
        if option == "--no-final-slice" {
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
            "--resolution" => resolutions = vec![value.parse()?],
            "--resolutions" => resolutions = parse_resolutions(&value)?,
            "--angle-step" => angle_step = value.parse()?,
            "--attempts" => attempts = value.parse()?,
            "--candidates" => candidates = value.parse()?,
            "--stall" => stall = Some(value.parse()?),
            "--max-objects" => max_objects = value.parse()?,
            "--edge-margin" => edge_margin = Some(value.parse()?),
            // Compatibility: the portfolio always runs every member.
            "--layout" => match value.as_str() {
                "search" | "grid" => {}
                other => return Err(format!("unknown layout mode: {other}").into()),
            },
            "--rotations" => rotations = parse_rotations(&value)?,
            "--cache-dir" => cache_dir = Some(PathBuf::from(value)),
            "--support-orders" => support_orders = parse_orders(&value)?,
            "--support-margins" => support_margins = Some(parse_margins(&value)?),
            "--support-source" => {
                support_source = match value.as_str() {
                    "probe" => SupportSource::Probe,
                    "model" => SupportSource::Model,
                    other => return Err(format!("unknown support source: {other}").into()),
                }
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
            "--foreign-gap-floor" => foreign_gap_floor = parse_foreign_pair(&option, &value)?,
            "--foreign-gaps" => foreign_gaps = parse_foreign_gaps(&value)?,
            "--finish-state" => finish_state = Some(PathBuf::from(value)),
            "--slicer" => {
                if !SLICER_KINDS.contains(&value.as_str()) {
                    return Err(format!(
                        "--slicer {value}: expected one of {}",
                        SLICER_KINDS.join(", ")
                    )
                    .into());
                }
                slicer = Some(value);
            }
            "--support-overlap-mm" => {
                support_overlap_mm =
                    Some(parse_time_value(&option, &value, 0.0, f64::INFINITY, false)?);
            }
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    check_ladder(&foreign_gaps, foreign_gap_floor)?;
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
        resolution: resolutions[0],
        resolutions,
        angle_step,
        attempts,
        candidates,
        stall,
        max_objects,
        seed,
        edge_margin,
        nozzle_clean,
        time,
        rotations,
        cache_dir,
        support_source,
        support_orders,
        support_margins,
        verify,
        support_overlap_mm,
        foreign_gaps,
        foreign_gap_floor,
        keep_own_gap,
        finish_state,
        slicer,
    })
}

/// `along,against`.
fn parse_orders(text: &str) -> AppResult<Vec<MergeOrder>> {
    let orders: Vec<MergeOrder> = text
        .split(',')
        .map(|name| match name.trim() {
            "along" => Ok(MergeOrder::Along),
            "against" => Ok(MergeOrder::Against),
            other => Err(format!("unknown merge order: {other} (along, against)")),
        })
        .collect::<Result<_, _>>()?;
    if orders.is_empty() {
        return Err("--support-orders needs at least one reading".into());
    }
    Ok(orders)
}

/// `FIRST_LAYER:BAND`, both in mm and not negative.
fn parse_margins(text: &str) -> AppResult<(f64, f64)> {
    let (first, band) = text
        .split_once(':')
        .ok_or("--support-margins expects FIRST_LAYER:BAND in mm")?;
    let parse = |part: &str| -> AppResult<f64> {
        let value: f64 = part.trim().parse()?;
        if !value.is_finite() || value < 0.0 {
            return Err(format!("invalid support margin {part}").into());
        }
        Ok(value)
    };
    Ok((parse(first)?, parse(band)?))
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
    fn slicer_is_optional_and_one_of_the_runner_kinds() {
        assert_eq!(parse(&REQUIRED).unwrap().slicer, None);
        assert_eq!(parse_with(&["--slicer", "orca"]).unwrap().slicer.as_deref(), Some("orca"));
        assert_eq!(parse_with(&["--slicer", "bambu"]).unwrap().slicer.as_deref(), Some("bambu"));
        assert!(parse_with(&["--slicer", "prusa"]).is_err());
    }

    #[test]
    fn keep_own_gap_is_a_valueless_switch_off_by_default() {
        assert!(!parse(&REQUIRED).unwrap().keep_own_gap);
        assert!(parse_with(&["--keep-own-gap", "--seed", "3"]).unwrap().keep_own_gap);
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
    fn resolutions_default_to_the_ladder_and_a_single_resolution_replaces_it() {
        let defaults = parse(&REQUIRED).unwrap();
        assert_eq!(defaults.resolutions, vec![0.25, 0.2]);
        assert_eq!(defaults.resolution, 0.25);
        let single = parse_with(&["--resolution", "0.5"]).unwrap();
        assert_eq!((single.resolution, single.resolutions), (0.5, vec![0.5]));
        let list = parse_with(&["--resolutions", "0.3, 0.2"]).unwrap();
        assert_eq!((list.resolution, list.resolutions), (0.3, vec![0.3, 0.2]));
        assert!(parse_with(&["--resolutions", "0.3,0"]).is_err());
        assert!(parse_with(&["--resolutions", ""]).is_err());
    }

    #[test]
    fn gap_flags_default_to_the_floor_alone() {
        let defaults = parse(&REQUIRED).unwrap();
        assert_eq!(defaults.foreign_gaps.len(), 1);
        assert_eq!(defaults.foreign_gaps[0], ForeignGaps::default());
        assert_eq!(defaults.foreign_gap_floor, ForeignGaps::default());
        let args = parse_with(&["--foreign-gaps", "0.5,0.1:0.7"]).unwrap();
        assert_eq!(
            args.foreign_gaps,
            vec![
                ForeignGaps { base_mm: 0.0, band_mm: 0.5 },
                ForeignGaps { base_mm: 0.1, band_mm: 0.7 }
            ]
        );
        assert!(parse_with(&["--foreign-gaps", "x"]).is_err());
        assert!(parse_with(&["--own-first-layer-gap", "1.2"]).is_err());
    }

    #[test]
    fn a_rung_below_the_floor_needs_the_floor_flag_in_either_order() {
        assert!(parse_with(&["--foreign-gaps", "0:0.25,0:0.5"]).is_err());
        for order in [
            ["--foreign-gaps", "0:0.25,0:0.5", "--foreign-gap-floor", "0:0.25"],
            ["--foreign-gap-floor", "0:0.25", "--foreign-gaps", "0:0.25,0:0.5"],
        ] {
            let args = parse_with(&order).unwrap();
            assert_eq!(args.foreign_gap_floor, ForeignGaps { base_mm: 0.0, band_mm: 0.25 });
            assert_eq!(args.foreign_gaps.len(), 2);
        }
        assert!(parse_with(&["--foreign-gap-floor", "-1"]).is_err());
    }

    #[test]
    fn verify_rotations_and_cache_flags_parse() {
        let defaults = parse(&REQUIRED).unwrap();
        assert!(!defaults.verify);
        assert_eq!(defaults.finish_state, None);
        assert_eq!(defaults.rotations, vec![0.0, 180.0]);
        assert_eq!(defaults.cache_dir, None);
        assert!(parse_with(&["--verify"]).unwrap().verify);
        // The old switch is accepted and changes nothing.
        assert!(!parse_with(&["--no-final-slice"]).unwrap().verify);
        assert_eq!(
            parse_with(&["--finish-state", "out.state.json"]).unwrap().finish_state,
            Some(PathBuf::from("out.state.json"))
        );
        assert_eq!(parse_with(&["--rotations", "0, 90,-90"]).unwrap().rotations, vec![0.0, 90.0, 270.0]);
        assert_eq!(
            parse_with(&["--cache-dir", "c"]).unwrap().cache_dir,
            Some(PathBuf::from("c"))
        );
        for bad in ["", "0,0", "0,360", "x", "NaN"] {
            assert!(parse_with(&["--rotations", bad]).is_err(), "{bad:?}");
        }
        assert!(parse_with(&["--sample-objects", "3"]).is_err());
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

//! AMS filament-change calibration test generator (spec
//! `docs/superpowers/specs/2026-10-26-ams-calibration-and-supports-design.md`, section 5):
//!
//!   ams_calibration --template project.3mf --profile profiles/p1s.json \
//!     --variant normal|perfectionist --out-dir out/
//!
//! Writes `calib_A.3mf`, `calib_B.3mf`, `instructions.md` and `calibration.json` into
//! `--out-dir`, slicing both projects with the profile's production slicer (scratch files in
//! `<out-dir>/slicing/`). A slice whose change count differs from the requested one is an error:
//! it guards the `custom_gcode_per_layer.xml` format against slicer versions.

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use goat_packer::{
    ams_calibration::{
        CalibrationProjects, Variant, bambu_filament_change_times, cube_footprint,
        instructions_markdown, orca_toolchange_count, plan_projects, read_template,
        write_calibration_3mf,
    },
    colored_packer::{PrinterProfile, load_printer_profile, parse_prime_tower_bounds},
    scheduler::{parse_bambu_result, parse_orca_gcode},
};

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

struct Args {
    template: PathBuf,
    profile: PathBuf,
    variant: Variant,
    out_dir: PathBuf,
}

fn parse_args() -> AppResult<Args> {
    let mut args = std::env::args().skip(1);
    let mut template = None;
    let mut profile = None;
    let mut variant = None;
    let mut out_dir = None;
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--template" => template = Some(PathBuf::from(value)),
            "--profile" => profile = Some(PathBuf::from(value)),
            "--variant" => variant = Some(Variant::parse(&value)?),
            "--out-dir" => out_dir = Some(PathBuf::from(value)),
            other => return Err(format!("unknown option {other}").into()),
        }
    }
    Ok(Args {
        template: template.ok_or("--template is required")?,
        profile: profile.ok_or("--profile is required")?,
        variant: variant.ok_or("--variant is required (normal|perfectionist)")?,
        out_dir: out_dir.ok_or("--out-dir is required")?,
    })
}

/// One sliced calibration project.
struct Slice {
    estimate_s: f64,
    changes: u64,
    gcode: Vec<u8>,
}

fn run() -> AppResult<()> {
    let args = parse_args()?;
    let profile = load_printer_profile(&args.profile)?;
    let template = read_template(&args.template)?;
    let projects = plan_projects(&template, &profile, args.variant)?;

    fs::create_dir_all(&args.out_dir)?;
    let calib_a = args.out_dir.join("calib_A.3mf");
    let calib_b = args.out_dir.join("calib_B.3mf");
    write_calibration_3mf(
        &calib_a,
        &template.project_settings,
        &projects.model,
        &projects.model_settings,
        None,
    )?;
    write_calibration_3mf(
        &calib_b,
        &template.project_settings,
        &projects.model,
        &projects.model_settings,
        Some(&projects.custom_gcode),
    )?;
    fs::write(
        args.out_dir.join("instructions.md"),
        instructions_markdown(&profile.printer_name, args.variant, &projects.plan),
    )?;

    let work_dir = args.out_dir.join("slicing");
    fs::create_dir_all(&work_dir)?;
    let cli_settings = cli_project_settings(&profile, &template.project_settings)?;
    let slice_a = slice(&profile, &projects, &cli_settings, None, &work_dir, "calib_A")?;
    let slice_b = slice(
        &profile,
        &projects,
        &cli_settings,
        Some(&projects.custom_gcode),
        &work_dir,
        "calib_B",
    )?;
    check_tower_clear(&profile, &projects, &slice_b.gcode)?;

    let changes_expected = args.variant.changes() as u64;
    if slice_a.changes != 0 {
        return Err(format!(
            "calib_A sliced with {} filament changes, expected none (see {})",
            slice_a.changes,
            work_dir.display()
        )
        .into());
    }
    if slice_b.changes != changes_expected {
        return Err(format!(
            "calib_B sliced with {} filament changes, expected {changes_expected}: the slicer \
             did not apply Metadata/custom_gcode_per_layer.xml as written (see {})",
            slice_b.changes,
            work_dir.display()
        )
        .into());
    }
    let report = serde_json::json!({
        "variant": args.variant.name(),
        "changes_expected": changes_expected,
        "changes_sliced": slice_b.changes,
        "estimate_a_s": slice_a.estimate_s,
        "estimate_b_s": slice_b.estimate_s,
        "slicer_change_s": projects.settings.slicer_change_s,
    });
    fs::write(
        args.out_dir.join("calibration.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

/// Settings for the copy that is actually sliced: the downloadable projects keep the template
/// settings byte for byte, but profiles with `blank_start_gcode_for_cli` (Orca/U1) need the
/// start G-code blanked for the CLI, exactly like `colored_3mf_packer`'s `prepare_cli_copy`.
/// The start G-code is identical in A and B, so its time cancels out of the measurement.
fn cli_project_settings(profile: &PrinterProfile, original: &[u8]) -> AppResult<Vec<u8>> {
    if !profile.blank_start_gcode_for_cli {
        return Ok(original.to_vec());
    }
    let mut settings: serde_json::Value = serde_json::from_slice(original)?;
    settings["machine_start_gcode"] = serde_json::Value::String(String::new());
    Ok(serde_json::to_vec(&settings)?)
}

fn slice(
    profile: &PrinterProfile,
    projects: &CalibrationProjects,
    settings: &[u8],
    custom_gcode: Option<&str>,
    work_dir: &Path,
    label: &str,
) -> AppResult<Slice> {
    let input = work_dir.join(format!("{label}_cli.3mf"));
    write_calibration_3mf(
        &input,
        settings,
        &projects.model,
        &projects.model_settings,
        custom_gcode,
    )?;
    let output_dir = work_dir.join(format!("{label}_slice"));
    if output_dir.exists() {
        fs::remove_dir_all(&output_dir)?;
    }
    fs::create_dir_all(&output_dir)?;
    let mut command = goat_packer::slicer_cli::slice_command(
        Path::new(&profile.slicer_path),
        &[],
        &output_dir,
        &input,
    );
    let output = goat_packer::process_output::bounded_output(&mut command)?;
    let log_path = work_dir.join(format!("{label}_slice.log"));
    fs::write(&log_path, [&output.stdout[..], &output.stderr[..]].concat())?;
    if !output.status.success() {
        let rerun = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        return Err(format!(
            "{} slicing of {label} failed ({}); slicer report: {}; full log: {}; rerun manually: {rerun}",
            profile.printer_name,
            output.status,
            slicer_report_error(&output_dir).unwrap_or_else(|| "<no result.json>".to_owned()),
            log_path.display(),
        )
        .into());
    }
    let gcode_path = fs::read_dir(&output_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "gcode"))
        .ok_or_else(|| format!("{label}: slicer output did not contain G-code"))?;
    let gcode = fs::read(gcode_path)?;
    let (estimate_s, changes) = if profile.slicer_kind == "bambu" {
        let result = fs::read(output_dir.join("result.json"))?;
        (
            parse_bambu_result(&result)?.slicer_total_seconds as f64,
            bambu_filament_change_times(&result)?,
        )
    } else {
        (
            parse_orca_gcode(&gcode, 1)?.slicer_total_seconds as f64,
            orca_toolchange_count(&gcode),
        )
    };
    Ok(Slice {
        estimate_s,
        changes,
        gcode,
    })
}

fn slicer_report_error(output_dir: &Path) -> Option<String> {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(output_dir.join("result.json")).ok()?).ok()?;
    let error = value.get("error_string")?.as_str()?.to_owned();
    (!error.is_empty()).then_some(error)
}

/// The cube was placed against a square estimate of the tower; the real tower depth only
/// exists after slicing, so make sure it did not grow into the cube.
fn check_tower_clear(
    profile: &PrinterProfile,
    projects: &CalibrationProjects,
    gcode: &[u8],
) -> AppResult<()> {
    let Some(tower) = parse_prime_tower_bounds(gcode, profile.tower_line_width_mm, 0.0) else {
        return Ok(());
    };
    let cube = cube_footprint(projects.centre);
    let tower = tower.keepout;
    if cube.min_x < tower.max_x
        && tower.min_x < cube.max_x
        && cube.min_y < tower.max_y
        && tower.min_y < cube.max_y
    {
        return Err(format!(
            "calib_B prime tower {tower:?} overlaps the calibration cube {cube:?}; move the tower \
             in the template project"
        )
        .into());
    }
    Ok(())
}

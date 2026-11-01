//! `qualify`: slice a draft end-to-end in the runner image (spec «Qualification», CLI spike
//! appendix). Not unit-tested as a whole: tests never call a slicer (CLAUDE.md); the pure
//! helpers below are.

use std::{
    fs,
    path::{Path, PathBuf},
};

use goat_packer::{
    embedded_mesh::write_binary_stl,
    orca_presets::PresetIndex,
    orca_project::{self, check_result_json, failure_line, find_file, read_json},
    process_output::bounded_output,
    purge_detect::purge_lines,
    slicer_cli::slice_command,
};
use serde_json::{Value, json};

use crate::AppResult;
use crate::annotate::{apply_macro_margin, start_gcode, start_gcode_macros};
use crate::util::{is_hand_made, today};

/// Exit code for "qualification ran and failed, report row written" — the wrapper tells it
/// apart from a crash or a `timeout` kill (which leave no row).
const EXIT_NOT_QUALIFIED: i32 = 3;
const DEFAULT_ORCA: &str = "/opt/slicers/orca/AppRun";
/// The packer's default `--clearance`; verify_outputs must require the same minimum gap.
const PACKER_CLEARANCE_MM: f64 = 0.7;
const REPORT_HEADER: &str =
    "| vendor | printer_model | id | result | reason | seconds |\n|---|---|---|---|---|---|\n";
/// The probe is a cube of this edge, mm, and this tall.
const PROBE_CUBE_MM: f64 = 20.0;
const PROBE_CUBE_HEIGHT_MM: f64 = 10.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    /// Flatten presets and export `probe.3mf` (needs the Orca CLI: runner-orca image).
    Export,
    /// Probe slice, purge detection, pack + verify, write the profile (needs the draft's
    /// slicer: runner-bambu for Bambu printers).
    Pack,
    All,
}

#[derive(Debug)]
struct QualifyArgs {
    draft: PathBuf,
    orca: Option<PathBuf>,
    orca_slicer: PathBuf,
    slicer: Option<PathBuf>,
    packer: PathBuf,
    verify: PathBuf,
    work: PathBuf,
    out: PathBuf,
    report: Option<PathBuf>,
    stage: Stage,
}

fn parse_qualify_args(raw: Vec<String>) -> AppResult<QualifyArgs> {
    let mut args = raw.into_iter();
    let (mut draft, mut orca, mut slicer, mut work, mut report) = (None, None, None, None, None);
    let mut orca_slicer = PathBuf::from(DEFAULT_ORCA);
    let mut packer = PathBuf::from("colored_3mf_packer");
    let mut verify = PathBuf::from("verify_outputs");
    let mut out = PathBuf::from("profiles");
    let mut stage = Stage::All;
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--draft" => draft = Some(PathBuf::from(value)),
            "--orca" => orca = Some(PathBuf::from(value)),
            "--orca-slicer" => orca_slicer = PathBuf::from(value),
            "--slicer" => slicer = Some(PathBuf::from(value)),
            "--packer" => packer = PathBuf::from(value),
            "--verify" => verify = PathBuf::from(value),
            "--work" => work = Some(PathBuf::from(value)),
            "--out" => out = PathBuf::from(value),
            "--report" => report = Some(PathBuf::from(value)),
            "--stage" => {
                stage = match value.as_str() {
                    "export" => Stage::Export,
                    "pack" => Stage::Pack,
                    "all" => Stage::All,
                    other => return Err(format!("unknown stage {other} (export|pack|all)").into()),
                }
            }
            _ => return Err(format!("unknown option {option}").into()),
        }
    }
    let args = QualifyArgs {
        draft: draft.ok_or("missing --draft")?,
        orca,
        orca_slicer,
        slicer,
        packer,
        verify,
        work: work.ok_or("missing --work")?,
        out,
        report,
        stage,
    };
    if args.stage != Stage::Pack && args.orca.is_none() {
        return Err("missing --orca (needed to export the probe)".into());
    }
    Ok(args)
}

pub fn qualify(raw: Vec<String>) -> AppResult<()> {
    let args = parse_qualify_args(raw)?;
    let draft: Value = serde_json::from_slice(&fs::read(&args.draft)?)?;
    let field = |key: &str| {
        draft
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let (id, vendor, model) = (field("id"), field("vendor"), field("printer_model"));
    if id.is_empty() {
        return Err(format!("{}: draft without id", args.draft.display()).into());
    }
    fs::create_dir_all(&args.work)?;
    let report = args
        .report
        .clone()
        .unwrap_or_else(|| args.work.join("report.md"));
    let started = std::time::Instant::now();
    let outcome = qualify_stages(&args, &draft);
    let seconds = started.elapsed().as_secs_f64();
    match outcome {
        Ok(None) => {
            println!(
                "{id}: probe exported to {}",
                args.work.join("probe.3mf").display()
            );
            Ok(())
        }
        Ok(Some(notes)) => {
            append_report(
                &report,
                &report_row(&vendor, &model, &id, "qualified", &notes, seconds),
            )?;
            println!("{id}: qualified ({notes})");
            Ok(())
        }
        Err(reason) => {
            let reason = reason.to_string();
            append_report(
                &report,
                &report_row(&vendor, &model, &id, "failed", &reason, seconds),
            )?;
            eprintln!("{id}: not qualified: {reason}");
            std::process::exit(EXIT_NOT_QUALIFIED);
        }
    }
}

/// `Ok(None)` after an export-only stage, `Ok(Some(summary))` when the profile was written.
fn qualify_stages(args: &QualifyArgs, draft: &Value) -> AppResult<Option<String>> {
    let probe = args.work.join("probe.3mf");
    if args.stage != Stage::Pack {
        export_probe(args, draft, &probe)?;
        if args.stage == Stage::Export {
            return Ok(None);
        }
    }
    if !probe.is_file() {
        return Err(format!("{} missing: run --stage export first", probe.display()).into());
    }
    pack_and_verify(args, draft, &probe).map(Some)
}

fn draft_str<'a>(draft: &'a Value, key: &str) -> AppResult<&'a str> {
    draft
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("draft has no {key}").into())
}

/// Spike 2026-11-01: flattened machine/process/filament + `--arrange 1` + `--export-3mf`, then the
/// build item is moved to the centre of the draft's printable area, off its exclusions
/// (`orca_project::place_build_item`). A 20×20×10 mm cube generated here: single colour, so no
/// prime tower and the tower is not qualified (the packer measures it per job from the user's own
/// probe).
fn export_probe(args: &QualifyArgs, draft: &Value, probe: &Path) -> AppResult<()> {
    let orca = args.orca.as_ref().ok_or("missing --orca")?;
    let index = PresetIndex::load(orca)?;
    let presets = orca_project::catalog_presets(
        &index,
        draft_str(draft, "vendor")?,
        draft_str(draft, "orca_machine")?,
    )?;
    let files = orca_project::write_cli_presets(&presets, &args.work.join("presets"))?;
    fs::write(
        args.work.join("presets.json"),
        serde_json::to_vec_pretty(
            &json!({"machine": presets.machine_name(), "process": presets.process_name(), "filament": presets.filament_name(),
                    // The pack stage may run without --orca (runner-bambu): it reads the macros here.
                    "start_gcode_macros": start_gcode_macros(&start_gcode(&presets.machine))}),
        )?,
    )?;
    let cube = args.work.join("cube.stl");
    write_binary_stl(
        &cube,
        &cube_triangles(PROBE_CUBE_MM, PROBE_CUBE_MM, PROBE_CUBE_HEIGHT_MM),
    )?;
    let export_dir = args.work.join("export");
    reset_dir(&export_dir)?;
    let mut command = orca_project::export_command(&args.orca_slicer, &files, &cube, &export_dir);
    run_logged(&mut command, &args.work.join("export.log"), "probe export")?;
    check_result_json(&export_dir, "probe export")?;
    fs::copy(orca_project::exported_3mf(&export_dir)?, probe)?;
    let target = probe_origin(draft, PROBE_CUBE_MM);
    let centre = (
        target.0 + PROBE_CUBE_MM / 2.0,
        target.1 + PROBE_CUBE_MM / 2.0,
    );
    orca_project::place_build_item(probe, centre)?;
    Ok(())
}

/// A packer run that passed: which workaround it took (`None` = plain), the profile it ran with
/// and where its output is.
struct PackedProbe {
    workaround: Option<&'static str>,
    profile: Value,
    packed: PathBuf,
    packer_work: PathBuf,
}

fn pack_and_verify(args: &QualifyArgs, draft: &Value, probe: &Path) -> AppResult<String> {
    let id = draft_str(draft, "id")?;
    if let Some(existing) = read_json(&args.out.join(format!("{id}.json")))
        && is_hand_made(&existing)
    {
        return Err(format!(
            "{id}: hand-made profile in {} is never overwritten",
            args.out.display()
        )
        .into());
    }
    let slicer = match &args.slicer {
        Some(path) => path.clone(),
        None => PathBuf::from(draft_str(draft, "slicer_path")?),
    };
    let purge = detect_purge(args, draft, &slicer, probe)?;

    let mut profile = draft.clone();
    profile["purge_lines"] = json!(purge);
    let presets = read_json(&args.work.join("presets.json")).unwrap_or_default();
    let macros: Vec<String> = serde_json::from_value(
        presets
            .get("start_gcode_macros")
            .cloned()
            .unwrap_or_default(),
    )
    .unwrap_or_default();
    // Before packing: the probe plate must already keep the macro-purge margin.
    let macro_note = apply_macro_margin(&mut profile, &macros);
    profile["slicer_path"] = json!(slicer.display().to_string());
    let passed = pack_with_workarounds(args, &profile, probe)?;
    verify_packed(args, &profile, &passed, &purge)?;

    let objects = read_json(&passed.packer_work.join("placements.json"))
        .and_then(|p| p.get("placements").and_then(Value::as_array).map(Vec::len))
        .unwrap_or(0);
    let mut notes = vec![
        format!(
            "process: {}",
            presets
                .get("process")
                .and_then(Value::as_str)
                .unwrap_or("?")
        ),
        format!(
            "filament: {}",
            presets
                .get("filament")
                .and_then(Value::as_str)
                .unwrap_or("?")
        ),
        format!("purge lines detected: {}", purge.len()),
        format!("probe plate: {objects} cubes 20x20 mm, final slice and verify_outputs OK"),
        "tower not qualified (single-colour probe)".to_owned(),
    ];
    notes.push(match passed.workaround {
        None => "no CLI workaround needed".to_owned(),
        Some(flag) => format!("{flag}: plain run failed, workaround passed"),
    });
    notes.extend(macro_note);
    let mut qualified = passed.profile;
    qualified["source"]["qualified"] = json!(true);
    qualified["source"]["qualified_on"] = json!(today());
    qualified["source"]["notes"] = json!(notes);
    // The packer-side copy may point at a mounted override; profiles keep the runner path.
    qualified["slicer_path"] = draft["slicer_path"].clone();
    fs::create_dir_all(&args.out)?;
    let mut text = serde_json::to_string_pretty(&qualified)?;
    text.push('\n');
    fs::write(args.out.join(format!("{id}.json")), text)?;
    Ok(format!(
        "{objects} cubes, {} purge lines, {}",
        purge.len(),
        passed.workaround.unwrap_or("no workaround")
    ))
}

/// Probe slice of the untouched probe: the real start G-code, so its purge is in the G-code.
fn detect_purge(
    args: &QualifyArgs,
    draft: &Value,
    slicer: &Path,
    probe: &Path,
) -> AppResult<Vec<[f64; 5]>> {
    let printable: [f64; 4] =
        serde_json::from_value(draft.get("printable_area").cloned().unwrap_or_default())
            .map_err(|_| "draft printable_area is not [x0,y0,x1,y1]")?;
    let probe_out = args.work.join("probe_slice");
    reset_dir(&probe_out)?;
    let mut command = slice_command(slicer, &[], &probe_out, probe);
    run_logged(
        &mut command,
        &args.work.join("probe_slice.log"),
        "probe slice",
    )?;
    check_result_json(&probe_out, "probe slice")?;
    let gcode = read_gcode(&probe_out)?;
    Ok(purge_lines(&gcode, printable))
}

/// Known CLI workarounds, tried in this order only when the packer itself fails (plan Task 6).
const WORKAROUNDS: [(&str, Option<&str>); 3] = [
    ("plain", None),
    ("no_check", Some("no_check_for_final_tower_probe")),
    ("blank_start", Some("blank_start_gcode_for_cli")),
];

fn pack_with_workarounds(
    args: &QualifyArgs,
    profile: &Value,
    probe: &Path,
) -> AppResult<PackedProbe> {
    let mut failures = Vec::new();
    for (label, flag) in WORKAROUNDS {
        let mut attempt = profile.clone();
        if let Some(flag) = flag {
            attempt[flag] = json!(true);
        }
        let dir = args.work.join(format!("attempt_{label}"));
        reset_dir(&dir)?;
        let profile_path = dir.join("profile.json");
        fs::write(&profile_path, serde_json::to_vec_pretty(&attempt)?)?;
        let packed = dir.join("packed.3mf");
        let packer_work = dir.join("work");
        let mut command = std::process::Command::new(&args.packer);
        command
            .arg("--source")
            .arg(probe)
            .arg("--profile")
            .arg(&profile_path)
            .arg("--output")
            .arg(&packed)
            .arg("--work-dir")
            .arg(&packer_work);
        match run_logged(&mut command, &dir.join("packer.log"), "packer") {
            Ok(()) => {
                return Ok(PackedProbe {
                    workaround: flag,
                    profile: attempt,
                    packed,
                    packer_work,
                });
            }
            Err(error) => failures.push(format!("{label}: {error}")),
        }
    }
    Err(failures.join("; ").into())
}

/// Independent exact-geometry check (shares no packing code).
fn verify_packed(
    args: &QualifyArgs,
    profile: &Value,
    passed: &PackedProbe,
    purge: &[[f64; 5]],
) -> AppResult<()> {
    let mut command = std::process::Command::new(&args.verify);
    command
        .arg(passed.packer_work.join("embedded_source.stl"))
        .arg(passed.packer_work.join("placements.json"))
        .arg(&passed.packed)
        .arg("--minimum-gap")
        .arg(PACKER_CLEARANCE_MM.to_string())
        .arg("--edge-margin")
        .arg(
            profile
                .get("edge_margin_mm")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .to_string(),
        )
        .arg("--report")
        .arg(args.work.join("verify.json"));
    for line in purge {
        command.arg("--purge-line").arg(join_numbers(line));
    }
    for rect in passed.profile["exclusion_rectangles"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        if let Ok(rect) = serde_json::from_value::<[f64; 4]>(rect) {
            command.arg("--keepout").arg(join_numbers(&rect));
        }
    }
    run_logged(
        &mut command,
        &args.work.join("verify.log"),
        "verify_outputs",
    )
}

fn reset_dir(dir: &Path) -> std::io::Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)
}

fn join_numbers(values: &[f64]) -> String {
    values
        .iter()
        .map(f64::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// Runs the command with stdout+stderr saved to `log`; a non-zero exit becomes an error carrying
/// the last meaningful output line (the packer prints `error: …` last).
fn run_logged(command: &mut std::process::Command, log: &Path, what: &str) -> AppResult<()> {
    let output = bounded_output(command)
        .map_err(|e| format!("{what}: cannot start {:?}: {e}", command.get_program()))?;
    fs::write(log, [&output.stdout[..], &output.stderr[..]].concat())?;
    if output.status.success() {
        return Ok(());
    }
    let reason = failure_line(&output.stderr)
        .or_else(|| failure_line(&output.stdout))
        .unwrap_or_default();
    Err(format!("{what} failed ({}): {reason}", output.status).into())
}

/// Plain `plate_1.gcode` (Orca and BambuStudio CLI with `--slice 1 --outputdir`), else a G-code
/// inside a `.gcode.3mf`.
fn read_gcode(dir: &Path) -> AppResult<String> {
    if let Some(path) = find_file(dir, |n| n.ends_with(".gcode")) {
        return Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned());
    }
    let packaged =
        find_file(dir, |n| n.ends_with(".gcode.3mf")).ok_or("probe slice wrote no G-code")?;
    let mut zip = zip::ZipArchive::new(fs::File::open(packaged)?)?;
    let name = zip
        .file_names()
        .filter(|n| n.ends_with(".gcode"))
        .min()
        .map(str::to_owned)
        .ok_or("no G-code inside the .gcode.3mf")?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut zip.by_name(&name)?, &mut text)?;
    Ok(text)
}

/// Lower-left corner for the probe cube: centred in the draft's printable area, off its
/// exclusion rectangles (`orca_project::free_spot`).
fn probe_origin(draft: &Value, size: f64) -> (f64, f64) {
    let area: [f64; 4] =
        serde_json::from_value(draft.get("printable_area").cloned().unwrap_or_default())
            .unwrap_or([0.0, 0.0, size, size]);
    let exclusions: Vec<[f64; 4]> = serde_json::from_value(
        draft
            .get("exclusion_rectangles")
            .cloned()
            .unwrap_or_default(),
    )
    .unwrap_or_default();
    orca_project::free_spot(area, &exclusions, (size, size))
}

/// Closed box `[0,w]×[0,d]×[0,h]`, 12 triangles wound counter-clockwise seen from outside.
fn cube_triangles(w: f64, d: f64, h: f64) -> Vec<[(f64, f64, f64); 3]> {
    let v = |i: usize| {
        let (x, y, z) = (i & 1, (i >> 1) & 1, (i >> 2) & 1);
        (x as f64 * w, y as f64 * d, z as f64 * h)
    };
    // Faces as corner quads (outward CCW): -z, +z, -y, +y, -x, +x.
    let quads = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    quads
        .iter()
        .flat_map(|q| [[v(q[0]), v(q[1]), v(q[2])], [v(q[0]), v(q[2]), v(q[3])]])
        .collect()
}

fn report_row(
    vendor: &str,
    model: &str,
    id: &str,
    result: &str,
    reason: &str,
    seconds: f64,
) -> String {
    let cell = |s: &str| s.replace('|', "\\|").replace(['\n', '\r'], " ");
    format!(
        "| {} | {} | {} | {} | {} | {seconds:.0} |\n",
        cell(vendor),
        cell(model),
        cell(id),
        result,
        cell(reason)
    )
}

/// One `write` per row with O_APPEND: rows from parallel containers do not interleave.
fn append_report(path: &Path, row: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let fresh = !path.exists();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let text = if fresh {
        format!("{REPORT_HEADER}{row}")
    } else {
        row.to_owned()
    };
    file.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_cube_sits_centred_and_off_exclusions() {
        let plain = json!({"printable_area": [0.0, 0.0, 256.0, 256.0], "exclusion_rectangles": []});
        assert_eq!(probe_origin(&plain, 20.0), (118.0, 118.0));
        let blocked = json!({"printable_area": [0.0, 0.0, 256.0, 256.0], "exclusion_rectangles": [[100.0, 100.0, 156.0, 156.0]]});
        let (x, y) = probe_origin(&blocked, 20.0);
        assert!(
            x + 20.0 <= 100.0 || x >= 156.0 || y + 20.0 <= 100.0 || y >= 156.0,
            "({x}, {y})"
        );
        let h2d = json!({"printable_area": [25.0, 0.0, 325.0, 320.0]});
        assert_eq!(probe_origin(&h2d, 20.0), (165.0, 150.0));
    }

    #[test]
    fn probe_cube_is_closed_and_wound_outwards() {
        let triangles = cube_triangles(20.0, 20.0, 10.0);
        assert_eq!(triangles.len(), 12);
        // Divergence theorem: the signed volume is +w·d·h only for outward (CCW) winding.
        let volume: f64 = triangles
            .iter()
            .map(|[a, b, c]| {
                (a.0 * (b.1 * c.2 - b.2 * c.1) - a.1 * (b.0 * c.2 - b.2 * c.0)
                    + a.2 * (b.0 * c.1 - b.1 * c.0))
                    / 6.0
            })
            .sum();
        assert!((volume - 4000.0).abs() < 1e-9, "{volume}");
        let max = triangles
            .iter()
            .flatten()
            .fold((0.0_f64, 0.0_f64, 0.0_f64), |m, p| {
                (m.0.max(p.0), m.1.max(p.1), m.2.max(p.2))
            });
        assert_eq!(max, (20.0, 20.0, 10.0));
    }

    #[test]
    fn qualify_args_need_orca_except_for_the_pack_stage() {
        let args = |list: &[&str]| parse_qualify_args(list.iter().map(|s| s.to_string()).collect());
        let pack = args(&["--draft", "d.json", "--work", "w", "--stage", "pack"]).unwrap();
        assert_eq!(pack.stage, Stage::Pack);
        assert_eq!(pack.orca_slicer, PathBuf::from(DEFAULT_ORCA));
        assert!(pack.slicer.is_none());
        assert!(args(&["--draft", "d.json", "--work", "w"]).is_err());
        let all = args(&[
            "--draft", "d.json", "--work", "w", "--orca", "o", "--slicer", "/s",
        ])
        .unwrap();
        assert_eq!(
            (all.stage, all.slicer),
            (Stage::All, Some(PathBuf::from("/s")))
        );
        assert!(args(&["--draft", "d.json", "--work", "w", "--stage", "nope"]).is_err());
    }

    #[test]
    fn report_rows_escape_cells_and_get_one_header() {
        let row = report_row(
            "Acme",
            "Box | 2",
            "acme_box",
            "failed",
            "line1\nline2",
            12.4,
        );
        assert_eq!(
            row,
            "| Acme | Box \\| 2 | acme_box | failed | line1 line2 | 12 |\n"
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.md");
        append_report(&path, &row).unwrap();
        append_report(&path, &row).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("| vendor |").count(), 1);
        assert_eq!(text.lines().count(), 4);
    }

    #[test]
    fn gcode_is_read_plain_or_from_a_gcode_3mf() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("plate_1.gcode"), ";LAYER_CHANGE\n").unwrap();
        assert_eq!(read_gcode(dir.path()).unwrap(), ";LAYER_CHANGE\n");
        let dir = tempfile::tempdir().unwrap();
        let mut zip =
            zip::ZipWriter::new(fs::File::create(dir.path().join("plate_1.gcode.3mf")).unwrap());
        zip.start_file(
            "Metadata/plate_1.gcode",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, b"G1 X1\n").unwrap();
        zip.finish().unwrap();
        assert_eq!(read_gcode(dir.path()).unwrap(), "G1 X1\n");
        assert!(read_gcode(tempfile::tempdir().unwrap().path()).is_err());
    }
}

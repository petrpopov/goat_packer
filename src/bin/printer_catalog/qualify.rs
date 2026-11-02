//! `qualify`: slice a draft end-to-end in the runner image (spec «Qualification», CLI spike
//! appendix). Not unit-tested as a whole: tests never call a slicer (CLAUDE.md); the pure
//! helpers in `args`, `probe` and `files` are.

mod args;
mod files;
mod probe;

use std::{
    fs,
    path::{Path, PathBuf},
};

use goat_packer::{
    colored_packer::PrinterProfile,
    embedded_mesh::write_binary_stl,
    orca_presets::PresetIndex,
    orca_project::{self, check_result_json, read_json},
    purge_detect::{DEFAULT_FILAMENT_DIAMETER_MM, PurgeStrip, purge_strips},
    slicer_cli::slice_command,
};
use serde_json::{Value, json};

use crate::AppResult;
use crate::annotate::macro_purge_note;
use crate::extract::first_number;
use crate::rules::{PrinterFacts, derive};
use crate::util::{is_hand_made, today};
use args::{QualifyArgs, Stage, parse_qualify_args};
use files::{append_report, join_numbers, read_gcode, report_row, reset_dir, run_logged};
use probe::{PROBE_CUBE_HEIGHT_MM, PROBE_CUBE_MM, cube_triangles, probe_origin};

/// Exit code for "qualification ran and failed, report row written" — the wrapper tells it
/// apart from a crash or a `timeout` kill (which leave no row).
const EXIT_NOT_QUALIFIED: i32 = 3;
/// The packer's default `--clearance`; verify_outputs must require the same minimum gap.
const PACKER_CLEARANCE_MM: f64 = 0.7;

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
                    // The pack stage may run without --orca (runner-bambu): it reads the machine
                    // facts (start G-code macros, first-layer scan) and the filament here.
                    "printer_facts": PrinterFacts::of_machine(&presets.machine),
                    "filament_diameter_mm": first_number(presets.filament.get("filament_diameter"))}),
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
    let presets = read_json(&args.work.join("presets.json")).unwrap_or_default();
    let filament_diameter = presets
        .get("filament_diameter_mm")
        .and_then(Value::as_f64)
        .unwrap_or(DEFAULT_FILAMENT_DIAMETER_MM);
    let strips = detect_purge(args, draft, &slicer, probe, filament_diameter)?;
    let purge: Vec<[f64; 5]> = strips.iter().map(|strip| strip.line).collect();

    let mut profile = draft.clone();
    profile["purge_lines"] = json!(purge);
    let mut facts: PrinterFacts =
        serde_json::from_value(presets.get("printer_facts").cloned().unwrap_or_default())
            .unwrap_or_default();
    facts.purge_z_mm = strips.iter().map(|strip| strip.z_mm).reduce(f64::max);
    let macro_note = macro_purge_note(&purge, &facts.macros);
    // Before packing: the probe plate must already keep the derived rules and the matching
    // edge margin (a hidden macro purge's band among them).
    with_derived_rules(&mut profile, draft_str(draft, "vendor")?, &facts)?;
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

/// Writes the derived `bed_rules`, `gui_flavour` and `edge_margin_mm` into a profile value.
fn with_derived_rules(profile: &mut Value, vendor: &str, facts: &PrinterFacts) -> AppResult<()> {
    let typed: PrinterProfile = serde_json::from_value(profile.clone())?;
    let derived = derive(&typed, vendor, facts);
    profile["edge_margin_mm"] = json!(derived.edge_margin_mm);
    profile["bed_rules"] = serde_json::to_value(&derived.bed_rules)?;
    profile["gui_flavour"] = json!(derived.gui_flavour);
    Ok(())
}

/// Probe slice of the untouched probe: the real start G-code, so its purge is in the G-code.
fn detect_purge(
    args: &QualifyArgs,
    draft: &Value,
    slicer: &Path,
    probe: &Path,
    filament_diameter_mm: f64,
) -> AppResult<Vec<PurgeStrip>> {
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
    Ok(purge_strips(&gcode, printable, filament_diameter_mm))
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

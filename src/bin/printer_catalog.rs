//! Developer tool: turns OrcaSlicer's JSON machine presets (pinned checkout) into draft
//! `profiles/*.json` for rectangular-bed printers (spec
//! `docs/superpowers/specs/2026-11-01-printer-catalog-and-stl-design.md`).
//!
//!   printer_catalog extract --orca <checkout>/resources/profiles --tag v2.4.2
//!     [--vendor Name]... [--profiles profiles/] --out <dir>
//!   printer_catalog qualify --draft <id.json> --orca <checkout>/resources/profiles
//!     --work <dir> [--out profiles/] [--report <file.md>] [--stage export|pack|all]
//!     [--orca-slicer /opt/slicers/orca/AppRun] [--slicer <path, default: draft slicer_path>]
//!     [--packer colored_3mf_packer] [--verify verify_outputs]
//!
//! `qualify` runs in the runner image (`deploy/catalog-qualify.sh`): the probe 3MF is always
//! exported by the Orca CLI, slicing and packing use the draft's slicer, so a Bambu printer
//! runs `--stage export` in runner-orca and `--stage pack` in runner-bambu.
//!
//! Only numbers and our own traced polygon leave Orca's files (AGPL, never copied). Draft JSON
//! keys are alphabetical: serde_json is built without `preserve_order`, so `Map` is a BTreeMap.
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use goat_packer::{
    bed_outline::outline_from_stl,
    embedded_mesh::write_binary_stl,
    orca_presets::{
        DEFAULT_FILAMENT, FILAMENT_LIBRARY, PresetIndex, exclusion_rects, parse_points,
        rect_from_points,
    },
    process_output::bounded_output,
    purge_detect::purge_lines,
    slicer_cli::slice_command,
};
use serde_json::{Map, Value, json};

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("extract") => extract(args.collect()),
        Some("qualify") => qualify(args.collect()),
        _ => Err("usage: printer_catalog extract|qualify ...".into()),
    }
}

struct ExtractArgs {
    orca: PathBuf,
    tag: String,
    vendors: Vec<String>,
    profiles: PathBuf,
    out: PathBuf,
}

fn parse_extract_args(raw: Vec<String>) -> AppResult<ExtractArgs> {
    let mut args = raw.into_iter();
    let (mut orca, mut tag, mut out) = (None, None, None);
    let mut vendors = Vec::new();
    let mut profiles = PathBuf::from("profiles");
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--orca" => orca = Some(PathBuf::from(value)),
            "--tag" => tag = Some(value),
            "--vendor" => vendors.push(value),
            "--profiles" => profiles = PathBuf::from(value),
            "--out" => out = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown option {option}").into()),
        }
    }
    Ok(ExtractArgs {
        orca: orca.ok_or("missing --orca")?,
        tag: tag.ok_or("missing --tag")?,
        vendors,
        profiles,
        out: out.ok_or("missing --out")?,
    })
}

/// Ids of hand-made profiles: no `source` block, or `source.manual == true`.
fn protected_ids(profiles_dir: &Path) -> AppResult<Vec<String>> {
    let mut ids = Vec::new();
    if !profiles_dir.is_dir() {
        return Ok(ids);
    }
    for entry in fs::read_dir(profiles_dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let value: Value = serde_json::from_slice(&fs::read(&path)?)?;
        if is_hand_made(&value) {
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()));
            ids.extend(id);
        }
    }
    Ok(ids)
}

fn is_hand_made(profile: &Value) -> bool {
    match profile.get("source") {
        None => true,
        Some(source) => source.get("manual").and_then(Value::as_bool) == Some(true),
    }
}

fn extract(raw: Vec<String>) -> AppResult<()> {
    let args = parse_extract_args(raw)?;
    let index = PresetIndex::load(&args.orca)?;
    let protected = protected_ids(&args.profiles)?;
    let date = today();
    let (drafts, skipped) = build_drafts(&index, &args.orca, &args.tag, &date, &protected, &args.vendors);
    fs::create_dir_all(&args.out)?;
    for draft in &drafts {
        let id = draft["id"].as_str().ok_or("draft without id")?;
        let mut text = serde_json::to_string_pretty(draft)?;
        text.push('\n');
        fs::write(args.out.join(format!("{id}.json")), text)?;
    }
    let reasons: Vec<String> = skipped.iter().map(|(r, n)| format!("{n} {r}")).collect();
    println!(
        "{} drafts, {} skipped ({})",
        drafts.len(),
        skipped.values().sum::<usize>(),
        reasons.join(", ")
    );
    Ok(())
}

/// UTC date `YYYY-MM-DD` from the system clock (no chrono dependency).
fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| (d.as_secs() / 86_400) as i64);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_owned()
}

/// Orca stores numbers as strings, string lists or `"a;b"`; the first value as a number.
fn first_number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.split([';', ',']).next()?.trim().parse().ok(),
        Value::Array(a) => first_number(a.first()),
        _ => None,
    }
}

pub fn drafts(
    index: &PresetIndex,
    vendors_root: &Path,
    orca_tag: &str,
    date: &str,
    protected_ids: &[String],
) -> Vec<Value> {
    build_drafts(index, vendors_root, orca_tag, date, protected_ids, &[]).0
}

/// Drafts plus skip-reason counts.
fn build_drafts(
    index: &PresetIndex,
    vendors_root: &Path,
    orca_tag: &str,
    date: &str,
    protected_ids: &[String],
    only_vendors: &[String],
) -> (Vec<Value>, BTreeMap<String, usize>) {
    let mut out = Vec::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut skip = |reason: &str| *skipped.entry(reason.to_owned()).or_default() += 1;
    for machine in index.machines() {
        if !only_vendors.is_empty() && !only_vendors.contains(&machine.vendor) {
            continue;
        }
        let id = slug(&machine.name);
        if protected_ids.contains(&id) {
            skip("protected hand-made profile");
            continue;
        }
        let Ok(merged) = index.resolve(&machine.vendor, &machine.name) else {
            skip("unresolvable inherits");
            continue;
        };
        let Some(points) = merged.get("printable_area").and_then(parse_points) else {
            skip("no printable_area");
            continue;
        };
        let Some(area) = rect_from_points(&points) else {
            skip("non-rectangular bed");
            continue;
        };
        let Some(height) = first_number(merged.get("printable_height")) else {
            skip("no printable_height");
            continue;
        };
        let model = merged
            .get("printer_model")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&machine.name)
            .to_owned();

        let mut draft = Map::new();
        draft.insert("id".into(), json!(id));
        draft.insert("vendor".into(), json!(machine.vendor));
        draft.insert("printer_name".into(), json!(model));
        draft.insert("printer_model".into(), json!(model));
        draft.insert("orca_machine".into(), json!(machine.name));
        let (kind, path) = slicer_for(&machine.vendor, &model);
        draft.insert("slicer_kind".into(), json!(kind));
        draft.insert("slicer_path".into(), json!(path));
        draft.insert("blank_start_gcode_for_cli".into(), json!(false));
        draft.insert("no_check_for_final_tower_probe".into(), json!(false));
        draft.insert("printable_area".into(), json!(area));
        draft.insert("printable_height".into(), json!(height));
        if let Some(d) = first_number(merged.get("nozzle_diameter")) {
            draft.insert("nozzle_diameter".into(), json!(d));
        }
        draft.insert("edge_margin_mm".into(), json!(0.0));
        draft.insert(
            "exclusion_rectangles".into(),
            json!(exclusion_rects(merged.get("bed_exclude_area"))),
        );
        draft.insert("purge_lines".into(), json!([]));
        draft.insert("tower_marker_style".into(), json!("wipe_tower"));
        draft.insert("tower_line_width_mm".into(), json!(0.5));
        if let Some(outline) = plate_outline(index, vendors_root, &machine.vendor, &model, area) {
            draft.insert("plate_outline".into(), json!(outline));
        }
        draft.insert(
            "source".into(),
            json!({"orca_tag": orca_tag, "imported": date, "qualified": false}),
        );
        out.push(Value::Object(draft));
    }
    (out, skipped)
}

/// Slicer kind and runner path for packing/final slices. Bambu printers must slice with
/// BambuStudio: `colored_3mf_packer` refuses an orca profile for a `Bambu Lab …` printer_model
/// (spike 2026-11-01). The probe 3MF is still exported by the Orca CLI (it accepts BBL presets);
/// the runner images expose both paths, each pointing at the image's own slicer.
fn slicer_for(vendor: &str, printer_model: &str) -> (&'static str, &'static str) {
    if vendor == "BBL" || printer_model.starts_with("Bambu Lab") {
        ("bambu", "/opt/slicers/bambu/AppRun")
    } else {
        ("orca", "/opt/slicers/orca/AppRun")
    }
}

/// Outline traced from the `machine_model` preset's `bed_model` STL when that file exists in the
/// vendor folder; `None` otherwise (the webapp then draws the printable rectangle).
fn plate_outline(
    index: &PresetIndex,
    vendors_root: &Path,
    vendor: &str,
    model: &str,
    area: [f64; 4],
) -> Option<Vec<[f64; 2]>> {
    let preset = index.get(vendor, model)?;
    if preset.get("type").and_then(Value::as_str) != Some("machine_model") {
        return None;
    }
    let file = preset.get("bed_model").and_then(Value::as_str).filter(|s| !s.is_empty())?;
    let bytes = fs::read(vendors_root.join(vendor).join(file)).ok()?;
    outline_from_stl(&bytes, area)
}

// ---------------------------------------------------------------------------------------------
// qualify: slice a draft end-to-end in the runner image (spec «Qualification», CLI spike
// appendix). Not unit-tested as a whole: tests never call a slicer (CLAUDE.md); the pure
// helpers below are.

/// Exit code for "qualification ran and failed, report row written" — the wrapper tells it
/// apart from a crash or a `timeout` kill (which leave no row).
const EXIT_NOT_QUALIFIED: i32 = 3;
const DEFAULT_ORCA: &str = "/opt/slicers/orca/AppRun";
/// The packer's default `--clearance`; verify_outputs must require the same minimum gap.
const PACKER_CLEARANCE_MM: f64 = 0.7;
const REPORT_HEADER: &str =
    "| vendor | printer_model | id | result | reason | seconds |\n|---|---|---|---|---|---|\n";

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

fn qualify(raw: Vec<String>) -> AppResult<()> {
    let args = parse_qualify_args(raw)?;
    let draft: Value = serde_json::from_slice(&fs::read(&args.draft)?)?;
    let field = |key: &str| draft.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    let (id, vendor, model) = (field("id"), field("vendor"), field("printer_model"));
    if id.is_empty() {
        return Err(format!("{}: draft without id", args.draft.display()).into());
    }
    fs::create_dir_all(&args.work)?;
    let report = args.report.clone().unwrap_or_else(|| args.work.join("report.md"));
    let started = std::time::Instant::now();
    let outcome = qualify_stages(&args, &draft);
    let seconds = started.elapsed().as_secs_f64();
    match outcome {
        Ok(None) => {
            println!("{id}: probe exported to {}", args.work.join("probe.3mf").display());
            Ok(())
        }
        Ok(Some(notes)) => {
            append_report(&report, &report_row(&vendor, &model, &id, "qualified", &notes, seconds))?;
            println!("{id}: qualified ({notes})");
            Ok(())
        }
        Err(reason) => {
            let reason = reason.to_string();
            append_report(&report, &report_row(&vendor, &model, &id, "failed", &reason, seconds))?;
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

/// Spike 2026-11-01: flattened machine/process/filament + `--arrange 1` (centres the cube) +
/// `--export-3mf`. A 20×20×10 mm cube generated here: single colour, so no prime tower and
/// the tower is not qualified (the packer measures it per job from the user's own probe).
fn export_probe(args: &QualifyArgs, draft: &Value, probe: &Path) -> AppResult<()> {
    let orca = args.orca.as_ref().ok_or("missing --orca")?;
    let index = PresetIndex::load(orca)?;
    let vendor = draft_str(draft, "vendor")?;
    let machine = draft_str(draft, "orca_machine")?;
    let process = index
        .select_process(vendor, machine)
        .ok_or_else(|| format!("no process preset for {machine} in {vendor}"))?;
    let presets = args.work.join("presets");
    fs::create_dir_all(&presets)?;
    let write = |file: &str, preset: Map<String, Value>| -> AppResult<PathBuf> {
        let path = presets.join(file);
        fs::write(&path, serde_json::to_vec_pretty(&Value::Object(preset))?)?;
        Ok(path)
    };
    let machine_json = write("machine.json", index.cli_preset(vendor, machine, machine)?)?;
    let process_json = write("process.json", index.cli_preset(vendor, &process, machine)?)?;
    let filament_json =
        write("filament.json", index.cli_preset(FILAMENT_LIBRARY, DEFAULT_FILAMENT, machine)?)?;
    fs::write(
        args.work.join("presets.json"),
        serde_json::to_vec_pretty(&json!({"machine": machine, "process": process, "filament": DEFAULT_FILAMENT}))?,
    )?;
    let cube = args.work.join("cube.stl");
    write_binary_stl(&cube, &cube_triangles(20.0, 20.0, 10.0))?;
    let export_dir = args.work.join("export");
    reset_dir(&export_dir)?;
    let mut command = std::process::Command::new(&args.orca_slicer);
    command
        .arg("--load-settings")
        .arg(format!("{};{}", machine_json.display(), process_json.display()))
        .arg("--load-filaments")
        .arg(&filament_json)
        .args(["--arrange", "1", "--export-3mf", "probe.3mf", "--outputdir"])
        .arg(&export_dir)
        .arg(&cube);
    run_logged(&mut command, &args.work.join("export.log"), "probe export")?;
    check_result_json(&export_dir, "probe export")?;
    let exported = find_file(&export_dir, |name| name.ends_with(".3mf"))
        .ok_or("probe export wrote no 3MF")?;
    fs::copy(exported, probe)?;
    Ok(())
}

fn pack_and_verify(args: &QualifyArgs, draft: &Value, probe: &Path) -> AppResult<String> {
    let id = draft_str(draft, "id")?;
    if let Some(existing) = read_json(&args.out.join(format!("{id}.json"))) {
        if is_hand_made(&existing) {
            return Err(format!("{id}: hand-made profile in {} is never overwritten", args.out.display()).into());
        }
    }
    let slicer = match &args.slicer {
        Some(path) => path.clone(),
        None => PathBuf::from(draft_str(draft, "slicer_path")?),
    };
    let printable: [f64; 4] = serde_json::from_value(draft.get("printable_area").cloned().unwrap_or_default())
        .map_err(|_| "draft printable_area is not [x0,y0,x1,y1]")?;

    // Probe slice of the untouched probe: the real start G-code, so its purge is in the G-code.
    let probe_out = args.work.join("probe_slice");
    reset_dir(&probe_out)?;
    let mut command = slice_command(&slicer, &[], &probe_out, probe);
    run_logged(&mut command, &args.work.join("probe_slice.log"), "probe slice")?;
    check_result_json(&probe_out, "probe slice")?;
    let gcode = read_gcode(&probe_out)?;
    let purge = purge_lines(&gcode, printable);

    let mut profile = draft.clone();
    profile["purge_lines"] = json!(purge);
    profile["slicer_path"] = json!(slicer.display().to_string());
    // Known CLI workarounds, tried in this order only when the packer itself fails (plan Task 6).
    let attempts: [(&str, Option<&str>); 3] = [
        ("plain", None),
        ("no_check", Some("no_check_for_final_tower_probe")),
        ("blank_start", Some("blank_start_gcode_for_cli")),
    ];
    let mut failures = Vec::new();
    let mut passed = None;
    for (label, flag) in attempts {
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
                passed = Some((flag, attempt, packed, packer_work));
                break;
            }
            Err(error) => failures.push(format!("{label}: {error}")),
        }
    }
    let Some((flag, mut qualified, packed, packer_work)) = passed else {
        return Err(failures.join("; ").into());
    };

    // Independent exact-geometry check (shares no packing code).
    let mut command = std::process::Command::new(&args.verify);
    command
        .arg(packer_work.join("embedded_source.stl"))
        .arg(packer_work.join("placements.json"))
        .arg(&packed)
        .arg("--minimum-gap")
        .arg(PACKER_CLEARANCE_MM.to_string())
        .arg("--edge-margin")
        .arg(draft.get("edge_margin_mm").and_then(Value::as_f64).unwrap_or(0.0).to_string())
        .arg("--report")
        .arg(args.work.join("verify.json"));
    for line in &purge {
        command.arg("--purge-line").arg(join_numbers(line));
    }
    for rect in qualified["exclusion_rectangles"].as_array().cloned().unwrap_or_default() {
        if let Ok(rect) = serde_json::from_value::<[f64; 4]>(rect) {
            command.arg("--keepout").arg(join_numbers(&rect));
        }
    }
    run_logged(&mut command, &args.work.join("verify.log"), "verify_outputs")?;

    let objects = read_json(&packer_work.join("placements.json"))
        .and_then(|p| p.get("placements").and_then(Value::as_array).map(Vec::len))
        .unwrap_or(0);
    let presets = read_json(&args.work.join("presets.json")).unwrap_or_default();
    let mut notes = vec![
        format!("process: {}", presets.get("process").and_then(Value::as_str).unwrap_or("?")),
        format!("filament: {}", presets.get("filament").and_then(Value::as_str).unwrap_or("?")),
        format!("purge lines detected: {}", purge.len()),
        format!("probe plate: {objects} cubes 20x20 mm, final slice and verify_outputs OK"),
        "tower not qualified (single-colour probe)".to_owned(),
    ];
    notes.push(match flag {
        None => "no CLI workaround needed".to_owned(),
        Some(flag) => format!("{flag}: plain run failed, workaround passed"),
    });
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
        flag.unwrap_or("no workaround")
    ))
}

fn reset_dir(dir: &Path) -> std::io::Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn join_numbers(values: &[f64]) -> String {
    values.iter().map(f64::to_string).collect::<Vec<_>>().join(",")
}

fn find_file(dir: &Path, wanted: impl Fn(&str) -> bool) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| wanted(&n.to_string_lossy())))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Runs the command with stdout+stderr saved to `log`; a non-zero exit becomes an error carrying
/// the last meaningful output line (the packer prints `error: …` last).
fn run_logged(command: &mut std::process::Command, log: &Path, what: &str) -> AppResult<()> {
    let output = bounded_output(command).map_err(|e| format!("{what}: cannot start {:?}: {e}", command.get_program()))?;
    fs::write(log, [&output.stdout[..], &output.stderr[..]].concat())?;
    if output.status.success() {
        return Ok(());
    }
    let reason = failure_line(&output.stderr).or_else(|| failure_line(&output.stdout)).unwrap_or_default();
    Err(format!("{what} failed ({}): {reason}", output.status).into())
}

/// The most useful single line of a failed tool's output: the last `error`-looking line, else
/// the last non-empty one.
fn failure_line(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let pick = lines
        .iter()
        .rev()
        .find(|l| l.to_ascii_lowercase().contains("error"))
        .or(lines.last())?;
    Some(pick.chars().take(300).collect())
}

/// Orca/Bambu write `result.json` with `return_code` (0 = OK) and `error_string`; a slicer can
/// exit 0 with a failure recorded there.
fn check_result_json(dir: &Path, what: &str) -> AppResult<()> {
    let Some(result) = read_json(&dir.join("result.json")) else { return Ok(()) };
    let code = result.get("return_code").and_then(Value::as_i64).unwrap_or(0);
    if code != 0 {
        let message = result.get("error_string").and_then(Value::as_str).unwrap_or("");
        return Err(format!("{what}: slicer return_code {code}: {message}").into());
    }
    Ok(())
}

/// Plain `plate_1.gcode` (Orca and BambuStudio CLI with `--slice 1 --outputdir`), else a G-code
/// inside a `.gcode.3mf`.
fn read_gcode(dir: &Path) -> AppResult<String> {
    if let Some(path) = find_file(dir, |n| n.ends_with(".gcode")) {
        return Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned());
    }
    let packaged = find_file(dir, |n| n.ends_with(".gcode.3mf")).ok_or("probe slice wrote no G-code")?;
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

/// Closed box `[0,w]×[0,d]×[0,h]`, 12 triangles wound counter-clockwise seen from outside.
fn cube_triangles(w: f64, d: f64, h: f64) -> Vec<[(f64, f64, f64); 3]> {
    let v = |i: usize| {
        let (x, y, z) = (i & 1, (i >> 1) & 1, (i >> 2) & 1);
        (x as f64 * w, y as f64 * d, z as f64 * h)
    };
    // Faces as corner quads (outward CCW): -z, +z, -y, +y, -x, +x.
    let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    quads
        .iter()
        .flat_map(|q| [[v(q[0]), v(q[1]), v(q[2])], [v(q[0]), v(q[2]), v(q[3])]])
        .collect()
}

fn report_row(vendor: &str, model: &str, id: &str, result: &str, reason: &str, seconds: f64) -> String {
    let cell = |s: &str| s.replace('|', "\\|").replace(['\n', '\r'], " ");
    format!("| {} | {} | {} | {} | {} | {seconds:.0} |\n", cell(vendor), cell(model), cell(id), result, cell(reason))
}

/// One `write` per row with O_APPEND: rows from parallel containers do not interleave.
fn append_report(path: &Path, row: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let fresh = !path.exists();
    let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
    let text = if fresh { format!("{REPORT_HEADER}{row}") } else { row.to_owned() };
    file.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog/presets")
    }

    #[test]
    fn extract_builds_drafts_for_rectangular_machines() {
        let root = root();
        let index = PresetIndex::load(&root).unwrap();
        let drafts = drafts(&index, &root, "v2.4.2", "2026-11-01", &[]);
        let ids: Vec<_> = drafts.iter().map(|d| d["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["acme_box_0_4_nozzle", "acme_ring_0_4_nozzle"]);
        let first = &drafts[0];
        assert_eq!(first["vendor"], "Acme");
        assert_eq!(first["printer_model"], "Acme Box");
        assert_eq!(first["printable_area"], json!([0.0, 0.0, 220.0, 220.0]));
        assert_eq!(first["exclusion_rectangles"], json!([[0.0, 0.0, 18.0, 28.0]]));
        assert_eq!(first["source"]["qualified"], false);
        assert_eq!(first["printable_height"], 250.0);
        assert_eq!(first["nozzle_diameter"], 0.4);
        assert!(first.get("plate_outline").is_none());
    }

    #[test]
    fn hand_made_profiles_are_never_overwritten() {
        let root = root();
        let index = PresetIndex::load(&root).unwrap();
        let drafts = drafts(
            &index,
            &root,
            "v2.4.2",
            "2026-11-01",
            &["acme_box_0_4_nozzle".to_owned()],
        );
        assert_eq!(drafts.len(), 1);
    }

    #[test]
    fn non_rectangular_beds_are_counted_as_skipped() {
        let mut index = PresetIndex::default();
        index.insert(
            "V",
            json!({"type":"machine","name":"Tri","instantiation":"true",
                   "printable_area":["0x0","100x0","50x80"],"printable_height":"100"}),
        );
        let (drafts, skipped) = build_drafts(&index, Path::new("/nonexistent"), "t", "d", &[], &[]);
        assert!(drafts.is_empty());
        assert_eq!(skipped["non-rectangular bed"], 1);
    }

    #[test]
    fn plate_outline_is_absent_without_bed_model_file() {
        let mut index = PresetIndex::default();
        index.insert(
            "V",
            json!({"type":"machine","name":"Box 0.4","instantiation":"true","printer_model":"Box",
                   "printable_area":["0x0","100x0","100x100","0x100"],"printable_height":"100"}),
        );
        index.insert("V", json!({"type":"machine_model","name":"Box","bed_model":"box_bed.stl"}));
        // The model names an STL, but the vendor folder has none.
        let dir = tempfile::tempdir().unwrap();
        let (drafts, _) = build_drafts(&index, dir.path(), "t", "d", &[], &[]);
        assert_eq!(drafts.len(), 1);
        assert!(drafts[0].get("plate_outline").is_none());
    }

    #[test]
    fn bambu_printers_slice_with_bambu_studio() {
        let mut index = PresetIndex::default();
        let machine = |name: &str, model: &str| {
            json!({"type":"machine","name":name,"instantiation":"true","printer_model":model,
                   "printable_area":["0x0","256x0","256x256","0x256"],"printable_height":"250"})
        };
        index.insert("BBL", machine("Bambu Lab P1S 0.4 nozzle", "Bambu Lab P1S"));
        index.insert("Acme", machine("Acme Box 0.4 nozzle", "Acme Box"));
        let (drafts, _) = build_drafts(&index, Path::new("/nonexistent"), "t", "d", &[], &[]);
        let kinds: Vec<_> = drafts.iter().map(|d| (d["vendor"].as_str().unwrap(), d["slicer_kind"].as_str().unwrap(), d["slicer_path"].as_str().unwrap())).collect();
        assert_eq!(
            kinds,
            vec![("Acme", "orca", "/opt/slicers/orca/AppRun"), ("BBL", "bambu", "/opt/slicers/bambu/AppRun")]
        );
        assert_eq!(slicer_for("SomeFork", "Bambu Lab X1"), ("bambu", "/opt/slicers/bambu/AppRun"));
    }

    #[test]
    fn probe_cube_is_closed_and_wound_outwards() {
        let triangles = cube_triangles(20.0, 20.0, 10.0);
        assert_eq!(triangles.len(), 12);
        // Divergence theorem: the signed volume is +w·d·h only for outward (CCW) winding.
        let volume: f64 = triangles
            .iter()
            .map(|[a, b, c]| {
                (a.0 * (b.1 * c.2 - b.2 * c.1) - a.1 * (b.0 * c.2 - b.2 * c.0) + a.2 * (b.0 * c.1 - b.1 * c.0)) / 6.0
            })
            .sum();
        assert!((volume - 4000.0).abs() < 1e-9, "{volume}");
        let max = triangles.iter().flatten().fold((0.0_f64, 0.0_f64, 0.0_f64), |m, p| (m.0.max(p.0), m.1.max(p.1), m.2.max(p.2)));
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
        let all = args(&["--draft", "d.json", "--work", "w", "--orca", "o", "--slicer", "/s"]).unwrap();
        assert_eq!((all.stage, all.slicer), (Stage::All, Some(PathBuf::from("/s"))));
        assert!(args(&["--draft", "d.json", "--work", "w", "--stage", "nope"]).is_err());
    }

    #[test]
    fn report_rows_escape_cells_and_get_one_header() {
        let row = report_row("Acme", "Box | 2", "acme_box", "failed", "line1\nline2", 12.4);
        assert_eq!(row, "| Acme | Box \\| 2 | acme_box | failed | line1 line2 | 12 |\n");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.md");
        append_report(&path, &row).unwrap();
        append_report(&path, &row).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("| vendor |").count(), 1);
        assert_eq!(text.lines().count(), 4);
    }

    #[test]
    fn failure_line_prefers_the_last_error() {
        let out = b"stage 1\nerror: printer_model 'X' implies slicer 'bambu'\nbye\n";
        assert_eq!(failure_line(out).unwrap(), "error: printer_model 'X' implies slicer 'bambu'");
        assert_eq!(failure_line(b"a\n\nlast\n").unwrap(), "last");
        assert_eq!(failure_line(b""), None);
    }

    #[test]
    fn slicer_result_json_failures_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_result_json(dir.path(), "x").is_ok());
        fs::write(dir.path().join("result.json"), r#"{"return_code": -17, "error_string": "not compatible"}"#).unwrap();
        let error = check_result_json(dir.path(), "probe export").unwrap_err().to_string();
        assert!(error.contains("-17") && error.contains("not compatible"), "{error}");
        fs::write(dir.path().join("result.json"), r#"{"return_code": 0, "error_string": "Success."}"#).unwrap();
        assert!(check_result_json(dir.path(), "x").is_ok());
    }

    #[test]
    fn gcode_is_read_plain_or_from_a_gcode_3mf() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("plate_1.gcode"), ";LAYER_CHANGE\n").unwrap();
        assert_eq!(read_gcode(dir.path()).unwrap(), ";LAYER_CHANGE\n");
        let dir = tempfile::tempdir().unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(dir.path().join("plate_1.gcode.3mf")).unwrap());
        zip.start_file("Metadata/plate_1.gcode", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zip, b"G1 X1\n").unwrap();
        zip.finish().unwrap();
        assert_eq!(read_gcode(dir.path()).unwrap(), "G1 X1\n");
        assert!(read_gcode(tempfile::tempdir().unwrap().path()).is_err());
    }

    #[test]
    fn slug_collapses_punctuation() {
        assert_eq!(slug("Acme Box 0.4 nozzle"), "acme_box_0_4_nozzle");
        assert_eq!(slug("Prusa MK4S (HF)"), "prusa_mk4s_hf");
    }
}

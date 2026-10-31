//! Developer tool: turns OrcaSlicer's JSON machine presets (pinned checkout) into draft
//! `profiles/*.json` for rectangular-bed printers (spec
//! `docs/superpowers/specs/2026-11-01-printer-catalog-and-stl-design.md`).
//!
//!   printer_catalog extract --orca <checkout>/resources/profiles --tag v2.4.2
//!     [--vendor Name]... [--profiles profiles/] --out <dir>
//!   printer_catalog qualify ...   (not implemented yet)
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
    orca_presets::{PresetIndex, exclusion_rects, parse_points, rect_from_points},
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
        Some("qualify") => {
            println!("qualify: not implemented yet");
            Ok(())
        }
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
        let hand_made = match value.get("source") {
            None => true,
            Some(source) => source.get("manual").and_then(Value::as_bool) == Some(true),
        };
        if hand_made {
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
        draft.insert("slicer_kind".into(), json!("orca"));
        draft.insert("slicer_path".into(), json!("/opt/slicers/orca/AppRun"));
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
    fn slug_collapses_punctuation() {
        assert_eq!(slug("Acme Box 0.4 nozzle"), "acme_box_0_4_nozzle");
        assert_eq!(slug("Prusa MK4S (HF)"), "prusa_mk4s_hf");
    }
}

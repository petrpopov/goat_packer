//! `extract`: draft profiles from OrcaSlicer's machine presets.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use goat_packer::{
    bed_outline::outline_from_stl,
    orca_presets::{
        PresetIndex, exclusion_rects, parse_points, rect_from_points, shared_extruder_area,
    },
};
use serde_json::{Map, Value, json};

use crate::AppResult;
use crate::util::{is_hand_made, slug, today};

/// The nozzle diameter the catalog prefers when a printer model has several presets.
const PREFERRED_NOZZLE_MM: f64 = 0.4;
/// Default prime-tower line width of a draft, mm.
const DRAFT_TOWER_LINE_WIDTH_MM: f64 = 0.5;

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
            // Also by printer_name: it equals the catalog's printer_model of the same printer.
            ids.extend(
                value
                    .get("printer_name")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            );
        }
    }
    Ok(ids)
}

pub fn extract(raw: Vec<String>) -> AppResult<()> {
    let args = parse_extract_args(raw)?;
    let index = PresetIndex::load(&args.orca)?;
    let protected = protected_ids(&args.profiles)?;
    let date = today();
    let (drafts, skipped) = build_drafts(
        &index,
        &args.orca,
        &args.tag,
        &date,
        &protected,
        &args.vendors,
    );
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

/// Orca stores numbers as strings, string lists or `"a;b"`; the first value as a number.
pub fn first_number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.split([';', ',']).next()?.trim().parse().ok(),
        Value::Array(a) => first_number(a.first()),
        _ => None,
    }
}

#[cfg(test)]
pub fn drafts(
    index: &PresetIndex,
    vendors_root: &Path,
    orca_tag: &str,
    date: &str,
    protected_ids: &[String],
) -> Vec<Value> {
    build_drafts(index, vendors_root, orca_tag, date, protected_ids, &[]).0
}

/// A machine preset of one `(vendor, printer_model)` group: nozzle, preset name, resolved JSON.
type Candidate = (f64, String, Map<String, Value>);

/// Skip-reason counts of a `build_drafts` run.
type Skipped = BTreeMap<String, usize>;

fn count_skip(skipped: &mut Skipped, reason: &str) {
    *skipped.entry(reason.to_owned()).or_default() += 1;
}

/// Machine presets grouped by `(vendor, printer_model)`.
fn group_machines(
    index: &PresetIndex,
    only_vendors: &[String],
    skipped: &mut Skipped,
) -> BTreeMap<(String, String), Vec<Candidate>> {
    let mut groups: BTreeMap<(String, String), Vec<Candidate>> = BTreeMap::new();
    for machine in index.machines() {
        if !only_vendors.is_empty() && !only_vendors.contains(&machine.vendor) {
            continue;
        }
        let Ok(merged) = index.resolve(&machine.vendor, &machine.name) else {
            count_skip(skipped, "unresolvable inherits");
            continue;
        };
        let model = merged
            .get("printer_model")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&machine.name)
            .to_owned();
        let nozzle = first_number(merged.get("nozzle_diameter")).unwrap_or(f64::INFINITY);
        groups
            .entry((machine.vendor.clone(), model))
            .or_default()
            .push((nozzle, machine.name.clone(), merged));
    }
    groups
}

/// The preset a printer model is judged by: the 0.4 nozzle one, else the smallest nozzle, ties by
/// name.
fn preferred_preset(mut candidates: Vec<Candidate>) -> (String, Map<String, Value>) {
    let is_default = |n: f64| (n - PREFERRED_NOZZLE_MM).abs() < 1e-9;
    candidates.sort_by(|a, b| {
        is_default(b.0)
            .cmp(&is_default(a.0))
            .then(a.0.total_cmp(&b.0))
            .then_with(|| a.1.cmp(&b.1))
    });
    let (_, machine_name, merged) = candidates.swap_remove(0);
    (machine_name, merged)
}

/// Drafts plus skip-reason counts.
pub fn build_drafts(
    index: &PresetIndex,
    vendors_root: &Path,
    orca_tag: &str,
    date: &str,
    protected_ids: &[String],
    only_vendors: &[String],
) -> (Vec<Value>, Skipped) {
    let mut out = Vec::new();
    let mut skipped = Skipped::new();
    let origin = DraftOrigin {
        index,
        vendors_root,
        orca_tag,
        date,
    };
    // One profile per (vendor, printer_model). Skip reasons below are judged on that chosen
    // preset only.
    for ((vendor, model), candidates) in group_machines(index, only_vendors, &mut skipped) {
        let (machine_name, merged) = preferred_preset(candidates);
        let id = slug(&model);
        // Protected by id or by printer_name, so a printer never gets two profiles.
        if protected_ids.contains(&id) || protected_ids.contains(&model) {
            count_skip(&mut skipped, "protected hand-made profile");
            continue;
        }
        let Some(points) = merged.get("printable_area").and_then(parse_points) else {
            count_skip(&mut skipped, "no printable_area");
            continue;
        };
        let Some(area) = rect_from_points(&points) else {
            count_skip(&mut skipped, "non-rectangular bed");
            continue;
        };
        let Some(height) = first_number(merged.get("printable_height")) else {
            count_skip(&mut skipped, "no printable_height");
            continue;
        };
        let machine = Machine {
            vendor: &vendor,
            model: &model,
            name: &machine_name,
            resolved: &merged,
        };
        out.push(draft_of(&origin, &machine, &id, area, height));
    }
    set_vendor_display(&mut out);
    (out, skipped)
}

/// Where drafts come from: the preset index and the import's tag and date.
struct DraftOrigin<'a> {
    index: &'a PresetIndex,
    vendors_root: &'a Path,
    orca_tag: &'a str,
    date: &'a str,
}

/// The machine preset chosen for one printer model.
struct Machine<'a> {
    vendor: &'a str,
    model: &'a str,
    /// The preset's own name (`Acme Box 0.4 nozzle`).
    name: &'a str,
    resolved: &'a Map<String, Value>,
}

/// One draft profile; `qualified: false` until `qualify` has run it through the slicer.
fn draft_of(
    origin: &DraftOrigin,
    machine: &Machine,
    id: &str,
    area: [f64; 4],
    height: f64,
) -> Value {
    let Machine {
        vendor,
        model,
        name: machine_name,
        resolved: merged,
    } = *machine;
    let mut draft = Map::new();
    draft.insert("id".into(), json!(id));
    draft.insert("vendor".into(), json!(vendor));
    draft.insert("printer_name".into(), json!(model));
    draft.insert("printer_model".into(), json!(model));
    draft.insert("orca_machine".into(), json!(machine_name));
    let (kind, path) = slicer_for(vendor, model);
    draft.insert("slicer_kind".into(), json!(kind));
    draft.insert("slicer_path".into(), json!(path));
    draft.insert("blank_start_gcode_for_cli".into(), json!(false));
    draft.insert("no_check_for_final_tower_probe".into(), json!(false));
    draft.insert(
        "printable_area".into(),
        json!(shared_extruder_area(
            area,
            merged.get("extruder_printable_area")
        )),
    );
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
    draft.insert(
        "tower_line_width_mm".into(),
        json!(DRAFT_TOWER_LINE_WIDTH_MM),
    );
    if let Some(outline) = plate_outline(origin.index, origin.vendors_root, vendor, model, area) {
        draft.insert("plate_outline".into(), json!(outline));
    }
    draft.insert(
        "source".into(),
        json!({"orca_tag": origin.orca_tag, "imported": origin.date, "qualified": false}),
    );
    Value::Object(draft)
}

/// Display vendor per Orca vendor folder: the leading words shared by every `printer_model` of
/// the folder, kept only while each model still has a word of its own (`Bambu Lab A1`, `Bambu Lab
/// X1` → `Bambu Lab`). A folder with one model or no shared words shows its folder name.
pub fn vendor_display_names<'a>(
    pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> BTreeMap<String, String> {
    let mut models: BTreeMap<&str, Vec<Vec<&str>>> = BTreeMap::new();
    for (vendor, model) in pairs {
        models
            .entry(vendor)
            .or_default()
            .push(model.split_whitespace().collect());
    }
    models
        .into_iter()
        .map(|(vendor, words)| {
            let shortest = words.iter().map(Vec::len).min().unwrap_or(0);
            let mut shared = 0;
            while words.len() > 1
                && shared + 1 < shortest
                && words.iter().all(|w| w[shared] == words[0][shared])
            {
                shared += 1;
            }
            let display = if shared == 0 {
                vendor.to_owned()
            } else {
                words[0][..shared].join(" ")
            };
            (vendor.to_owned(), display)
        })
        .collect()
}

/// Writes `vendor_display` into every profile that has a `vendor` and a `printer_model`.
pub fn set_vendor_display(profiles: &mut [Value]) {
    let field = |p: &Value, key: &str| p.get(key).and_then(Value::as_str).map(str::to_owned);
    let pairs: Vec<(String, String)> = profiles
        .iter()
        .filter_map(|p| Some((field(p, "vendor")?, field(p, "printer_model")?)))
        .collect();
    let names = vendor_display_names(pairs.iter().map(|(v, m)| (v.as_str(), m.as_str())));
    for profile in profiles.iter_mut() {
        if field(profile, "printer_model").is_none() {
            continue;
        }
        if let Some(display) = field(profile, "vendor").and_then(|v| names.get(&v).cloned()) {
            profile["vendor_display"] = json!(display);
        }
    }
}

/// Slicer kind and runner path for packing/final slices. Bambu printers must slice with
/// BambuStudio: `colored_3mf_packer` refuses an orca profile for a `Bambu Lab …` printer_model
/// (spike 2026-11-01). The probe 3MF is still exported by the Orca CLI (it accepts BBL presets);
/// the runner images expose both paths, each pointing at the image's own slicer.
pub fn slicer_for(vendor: &str, printer_model: &str) -> (&'static str, &'static str) {
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
    let file = preset
        .get("bed_model")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
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
        assert_eq!(ids, vec!["acme_box", "acme_ring"]);
        let first = &drafts[0];
        assert_eq!(first["vendor"], "Acme");
        assert_eq!(first["printer_model"], "Acme Box");
        assert_eq!(first["printable_area"], json!([0.0, 0.0, 220.0, 220.0]));
        assert_eq!(
            first["exclusion_rectangles"],
            json!([[0.0, 0.0, 18.0, 28.0]])
        );
        assert_eq!(first["source"]["qualified"], false);
        assert_eq!(first["printable_height"], 250.0);
        assert_eq!(first["nozzle_diameter"], 0.4);
        assert!(first.get("plate_outline").is_none());
    }

    #[test]
    fn one_draft_per_printer_model_prefers_the_0_4_nozzle() {
        let root = root();
        let index = PresetIndex::load(&root).unwrap();
        let drafts = drafts(&index, &root, "v2.4.2", "2026-11-01", &[]);
        let boxes: Vec<_> = drafts
            .iter()
            .filter(|d| d["printer_model"] == "Acme Box")
            .collect();
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0]["orca_machine"], "Acme Box 0.4 nozzle");
        assert_eq!(boxes[0]["id"], "acme_box");
    }

    #[test]
    fn smallest_nozzle_wins_without_a_0_4() {
        let mut index = PresetIndex::default();
        for (name, nozzle) in [("Q 0.8", "0.8"), ("Q 0.25", "0.25"), ("Q 0.6", "0.6")] {
            index.insert(
                "V",
                json!({"type":"machine","name":name,"instantiation":"true","printer_model":"Q",
                       "nozzle_diameter":[nozzle],"printable_area":["0x0","100x0","100x100","0x100"],
                       "printable_height":"100"}),
            );
        }
        let (drafts, _) = build_drafts(&index, Path::new("/nonexistent"), "t", "d", &[], &[]);
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0]["orca_machine"], "Q 0.25");
    }

    #[test]
    fn hand_made_printer_name_protects_the_model() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("x.json"),
            r#"{"id":"hand","printer_name":"Acme Box"}"#,
        )
        .unwrap();
        let protected = protected_ids(dir.path()).unwrap();
        let root = root();
        let index = PresetIndex::load(&root).unwrap();
        let (drafts, skipped) = build_drafts(&index, &root, "t", "d", &protected, &[]);
        assert!(drafts.iter().all(|d| d["printer_model"] != "Acme Box"));
        assert_eq!(skipped["protected hand-made profile"], 1);
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
            &["acme_box".to_owned()],
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
        index.insert(
            "V",
            json!({"type":"machine_model","name":"Box","bed_model":"box_bed.stl"}),
        );
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
        let kinds: Vec<_> = drafts
            .iter()
            .map(|d| {
                (
                    d["vendor"].as_str().unwrap(),
                    d["slicer_kind"].as_str().unwrap(),
                    d["slicer_path"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("Acme", "orca", "/opt/slicers/orca/AppRun"),
                ("BBL", "bambu", "/opt/slicers/bambu/AppRun")
            ]
        );
        assert_eq!(
            slicer_for("SomeFork", "Bambu Lab X1"),
            ("bambu", "/opt/slicers/bambu/AppRun")
        );
    }

    #[test]
    fn vendor_display_is_the_prefix_every_model_of_the_folder_shares() {
        let names = vendor_display_names([
            ("BBL", "Bambu Lab A1"),
            ("BBL", "Bambu Lab X1 Carbon"),
            ("Voron", "Voron 2.4 250"),
            ("Voron", "Voron Trident 250"),
            ("Solo", "Solo One"),
            ("Mix", "Alpha One"),
            ("Mix", "Beta Two"),
            ("Tiny", "Lab"),
            ("Tiny", "Lab X"),
        ]);
        assert_eq!(names["BBL"], "Bambu Lab");
        assert_eq!(names["Voron"], "Voron");
        assert_eq!(names["Solo"], "Solo");
        assert_eq!(names["Mix"], "Mix");
        // Every model keeps a word of its own: `Lab` cannot be the vendor of `Lab`.
        assert_eq!(names["Tiny"], "Tiny");
    }
}

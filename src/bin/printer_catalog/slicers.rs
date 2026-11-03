//! `slicers`: which slicers a user may print each catalog printer with (the profile's
//! `slicers` map, `PrinterProfile::for_slicer`). The runner images carry one slicer each
//! (`goat-packer-runner-{bambu,orca}`) and their preset trees: an STL job exports, probes and
//! slices with the chosen one. The profile's own `slicer_kind` is always offered. Another kind
//! is offered only when its tree has the machine (vendor folder + `orca_machine`, process and
//! filament resolving as `source_project` resolves them) with the same bed and start G-code as
//! the Orca 2.4.2 preset the profile was qualified on: purge lines, exclusions and the margin
//! were measured from that preset, and a tree with a different start G-code would purge where
//! the profile does not know. Bambu printers come from Orca's BBL tree, so Orca is offered for
//! them whenever that tree has the printer (not the A2L).

use std::{fs, path::PathBuf};

use goat_packer::colored_packer::SLICER_KINDS;
use goat_packer::orca_presets::{PresetIndex, exclusion_rects, parse_points, rect_from_points};
use goat_packer::orca_project::catalog_presets;
use serde_json::{Map, Value, json};

use crate::AppResult;
use crate::annotate::start_gcode;
use crate::profile_text::with_top_level;
use crate::rules::target_gui;
use crate::util::{is_hand_made, json_files};

/// What a machine preset contributes to a profile: equal facts mean the profile holds for it.
#[derive(Debug, PartialEq)]
struct BedFacts {
    printer_model: String,
    area: Option<[f64; 4]>,
    exclusions: Vec<[f64; 4]>,
    height: Option<String>,
    extruder_offset: Option<Vec<(f64, f64)>>,
    start_gcode: Vec<String>,
}

impl BedFacts {
    fn of(machine: &Map<String, Value>) -> Self {
        let text = |key: &str| match machine.get(key) {
            Some(Value::String(s)) => Some(s.trim().to_owned()),
            Some(Value::Number(n)) => Some(n.to_string()),
            Some(Value::Array(items)) => items.first().and_then(Value::as_str).map(|s| s.trim().to_owned()),
            _ => None,
        };
        Self {
            printer_model: text("printer_model").unwrap_or_default(),
            area: machine.get("printable_area").and_then(parse_points).as_deref().and_then(rect_from_points),
            exclusions: exclusion_rects(machine.get("bed_exclude_area")),
            // "250" and "250.0" are one height.
            height: text("printable_height").and_then(|h| h.parse::<f64>().ok()).map(|h| format!("{h}")),
            extruder_offset: machine.get("extruder_offset").and_then(parse_points),
            start_gcode: start_gcode(machine)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect(),
        }
    }
}

/// The preset trees of the runner images by slicer kind.
pub struct Trees {
    pub orca: PresetIndex,
    pub bambu: PresetIndex,
}

impl Trees {
    fn of(&self, kind: &str) -> &PresetIndex {
        if kind == "bambu" { &self.bambu } else { &self.orca }
    }
}

/// The `slicers` map of one profile.
pub fn offered(profile: &Value, trees: &Trees) -> AppResult<Map<String, Value>> {
    let field = |key: &str| profile.get(key).and_then(Value::as_str).unwrap_or_default();
    let (vendor, machine, own) = (field("vendor"), field("orca_machine"), field("slicer_kind"));
    let own_gui = profile
        .get("gui_flavour")
        .and_then(Value::as_str)
        .ok_or("profile has no gui_flavour (run annotate first)")?;
    let mut map = Map::new();
    map.insert(own.to_owned(), json!(own_gui));
    let Ok(reference) = trees.orca.resolve(vendor, machine) else {
        return Ok(map);
    };
    let reference = BedFacts::of(&reference);
    for kind in SLICER_KINDS.into_iter().filter(|kind| *kind != own) {
        let tree = trees.of(kind);
        let Ok(presets) = catalog_presets(tree, vendor, machine) else { continue };
        if BedFacts::of(&presets.machine) == reference {
            map.insert(kind.to_owned(), json!(target_gui(kind, vendor)));
        }
    }
    Ok(map)
}

/// `slicers --orca <tree> --bambu <tree> [--profiles profiles/]`: writes the `slicers` map of
/// every catalog profile (hand-made ones are never touched) as a minimal text edit.
pub fn slicers(raw: Vec<String>) -> AppResult<()> {
    let mut args = raw.into_iter();
    let mut dir = PathBuf::from("profiles");
    let (mut orca, mut bambu) = (None, None);
    while let Some(option) = args.next() {
        let value = args.next().ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--profiles" => dir = PathBuf::from(value),
            "--orca" => orca = Some(PathBuf::from(value)),
            "--bambu" => bambu = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown option {option}").into()),
        }
    }
    let trees = Trees {
        orca: PresetIndex::load(&orca.ok_or("missing --orca")?)?,
        bambu: PresetIndex::load(&bambu.ok_or("missing --bambu")?)?,
    };
    let (mut catalog, mut changed) = (0, Vec::new());
    for path in json_files(&dir)? {
        let text = fs::read_to_string(&path)?;
        let value: Value = serde_json::from_str(&text)?;
        if is_hand_made(&value) {
            continue;
        }
        catalog += 1;
        let map = offered(&value, &trees).map_err(|e| format!("{}: {e}", path.display()))?;
        let kinds: Vec<&str> = map.keys().map(String::as_str).collect();
        let edited = with_top_level(&text, "slicers", &Value::Object(map.clone()))
            .ok_or_else(|| format!("{}: not a pretty-printed profile object", path.display()))?;
        if edited != text {
            fs::write(&path, edited)?;
            changed.push(format!("{}: {}", path.display(), kinds.join(", ")));
        }
    }
    println!("{catalog} catalog profiles, {} rewritten", changed.len());
    for line in changed {
        println!("  {line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use goat_packer::colored_packer::bed_rules::GuiFlavour;

    /// Every `slicers` entry names a GUI flavour that parses.
    fn flavours(map: &Map<String, Value>) -> Vec<String> {
        map.values()
            .map(|v| GuiFlavour::try_from(v.as_str().unwrap().to_owned()).unwrap().to_string())
            .collect()
    }

    const START: &str = "G28\nG1 X10 Y1 E5 ; purge\n";

    fn tree(vendor: &str, machine: Value) -> PresetIndex {
        let mut index = PresetIndex::default();
        index.insert(vendor, machine);
        index.insert(
            vendor,
            json!({"type": "process", "name": "0.20mm Standard @Acme", "instantiation": "true",
                   "layer_height": "0.2", "compatible_printers": ["Acme Box 0.4 nozzle"]}),
        );
        index.insert(vendor, json!({"type": "filament", "name": "Generic PLA", "instantiation": "true"}));
        index
    }

    fn machine(start: &str, height: &str) -> Value {
        json!({"type": "machine", "name": "Acme Box 0.4 nozzle", "instantiation": "true",
               "printer_model": "Acme Box", "printable_area": ["0x0", "220x0", "220x220", "0x220"],
               "printable_height": height, "machine_start_gcode": start})
    }

    fn profile(vendor: &str, kind: &str, gui: &str) -> Value {
        json!({"vendor": vendor, "orca_machine": "Acme Box 0.4 nozzle", "slicer_kind": kind, "gui_flavour": gui})
    }

    #[test]
    fn another_slicer_is_offered_only_with_the_same_bed_and_start_gcode() {
        let same = Trees { orca: tree("Acme", machine(START, "250")), bambu: tree("Acme", machine(START, "250.0")) };
        let map = offered(&profile("Acme", "orca", "orca-2.4"), &same).unwrap();
        assert_eq!(map, json!({"bambu": "bambu-02.08", "orca": "orca-2.4"}).as_object().unwrap().clone());
        assert_eq!(flavours(&map), vec!["bambu-02.08", "orca-2.4"]);

        let other_start = Trees { orca: tree("Acme", machine(START, "250")), bambu: tree("Acme", machine("G28\nPRINT_START\n", "250")) };
        let map = offered(&profile("Acme", "orca", "orca-2.4"), &other_start).unwrap();
        assert_eq!(map.keys().collect::<Vec<_>>(), vec!["orca"]);

        let missing = Trees { orca: tree("Acme", machine(START, "250")), bambu: PresetIndex::default() };
        assert_eq!(offered(&profile("Acme", "orca", "orca-2.4"), &missing).unwrap().len(), 1);
    }

    #[test]
    fn a_bambu_printer_offers_orca_and_a_snapmaker_one_keeps_its_gui() {
        let trees = Trees { orca: tree("BBL", machine(START, "250")), bambu: tree("BBL", machine("M970\n", "250")) };
        let map = offered(&profile("BBL", "bambu", "bambu-02.08"), &trees).unwrap();
        // The Bambu tree's own start G-code is irrelevant to its own kind; Orca's BBL preset is
        // the qualified reference, so Orca is offered.
        assert_eq!(map, json!({"bambu": "bambu-02.08", "orca": "orca-2.4"}).as_object().unwrap().clone());
        // A printer missing from Orca's tree (A2L) keeps its own slicer only.
        let only_bambu = Trees { orca: PresetIndex::default(), bambu: tree("BBL", machine(START, "250")) };
        assert_eq!(offered(&profile("BBL", "bambu", "bambu-02.08"), &only_bambu).unwrap().len(), 1);

        let snap = Trees { orca: tree("Snapmaker", machine(START, "250")), bambu: PresetIndex::default() };
        let map = offered(&profile("Snapmaker", "orca", "snapmaker-orca-2.4"), &snap).unwrap();
        assert_eq!(map, json!({"orca": "snapmaker-orca-2.4"}).as_object().unwrap().clone());
    }

    #[test]
    fn slicers_rewrites_catalog_profiles_and_leaves_hand_made_ones() {
        let dir = tempfile::tempdir().unwrap();
        let presets = dir.path().join("presets");
        for (kind, start) in [("orca", START), ("bambu", START)] {
            let machines = presets.join(kind).join("Acme").join("machine");
            fs::create_dir_all(&machines).unwrap();
            let index_dir = presets.join(kind).join("Acme");
            fs::write(machines.join("m.json"), machine(start, "250").to_string()).unwrap();
            fs::create_dir_all(index_dir.join("process")).unwrap();
            fs::write(
                index_dir.join("process").join("p.json"),
                json!({"type": "process", "name": "0.20mm Standard @Acme", "instantiation": "true",
                       "layer_height": "0.2", "compatible_printers": ["Acme Box 0.4 nozzle"]})
                .to_string(),
            )
            .unwrap();
            fs::create_dir_all(index_dir.join("filament")).unwrap();
            fs::write(
                index_dir.join("filament").join("f.json"),
                json!({"type": "filament", "name": "Generic PLA", "instantiation": "true"}).to_string(),
            )
            .unwrap();
        }
        let profiles = dir.path().join("profiles");
        fs::create_dir_all(&profiles).unwrap();
        let mut catalog = profile("Acme", "orca", "orca-2.4");
        catalog["source"] = json!({"qualified": true});
        let text = format!("{}\n", serde_json::to_string_pretty(&catalog).unwrap());
        fs::write(profiles.join("acme.json"), &text).unwrap();
        let hand = "{\"vendor\": \"Acme\", \"slicer_kind\": \"orca\"}";
        fs::write(profiles.join("hand.json"), hand).unwrap();
        let run = || {
            slicers(vec![
                "--profiles".into(), profiles.display().to_string(),
                "--orca".into(), presets.join("orca").display().to_string(),
                "--bambu".into(), presets.join("bambu").display().to_string(),
            ])
            .unwrap()
        };
        run();
        let once = fs::read_to_string(profiles.join("acme.json")).unwrap();
        assert!(once.contains("  \"slicers\": {\n    \"bambu\": \"bambu-02.08\",\n    \"orca\": \"orca-2.4\"\n  },\n  \"source\""));
        run();
        assert_eq!(fs::read_to_string(profiles.join("acme.json")).unwrap(), once);
        assert_eq!(fs::read_to_string(profiles.join("hand.json")).unwrap(), hand);
    }
}

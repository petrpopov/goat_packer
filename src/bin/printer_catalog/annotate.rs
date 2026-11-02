//! `annotate`: bed rules, GUI flavour and the macro-purge rule for existing catalog profiles.

use std::{fs, path::PathBuf};

use goat_packer::colored_packer::PrinterProfile;
use goat_packer::colored_packer::bed_rules::old_field_mismatches;
use goat_packer::orca_presets::PresetIndex;
use serde_json::{Map, Value, json};

use crate::AppResult;
use crate::profile_text::with_top_level;
use crate::rules::{PrinterFacts, derive};
use crate::util::{is_hand_made, json_files};

/// Edge band for a printer whose start G-code purges inside a firmware macro: where the macro
/// draws its line we cannot know, 5 mm is the custom-bed default for the same reason.
pub const MACRO_PURGE_MARGIN_MM: f64 = 5.0;
const MACRO_NOTE_PREFIX: &str = "start G-code calls firmware macros";
/// Firmware commands that are not user macros (and never purge): Klipper built-ins seen in Orca
/// 2.4.2 start G-codes, Bambu's `SYNC`. Without the list a K1 `BED_MESH_CALIBRATE` or an X2D
/// `SYNC R0 T600` would count as a hidden purge.
const FIRMWARE_COMMANDS: &[&str] = &[
    "ACTIVATE_EXTRUDER",
    "BED_MESH_CALIBRATE",
    "BED_MESH_CLEAR",
    "BED_MESH_PROFILE",
    "CLEAR_PAUSE",
    "EXCLUDE_OBJECT_DEFINE",
    "FIRMWARE_RESTART",
    "QUAD_GANTRY_LEVEL",
    "RESPOND",
    "RESTORE_GCODE_STATE",
    "SAVE_GCODE_STATE",
    "SET_EXTRUDER_ROTATION_DISTANCE",
    "SET_FAN_SPEED",
    "SET_GCODE_OFFSET",
    "SET_GCODE_VARIABLE",
    "SET_HEATER_TEMPERATURE",
    "SET_IDLE_TIMEOUT",
    "SET_INPUT_SHAPER",
    "SET_KINEMATIC_POSITION",
    "SET_LED",
    "SET_PIN",
    "SET_PRESSURE_ADVANCE",
    "SET_PRINT_STATS_INFO",
    "SET_RETRACTION",
    "SET_STEPPER_ENABLE",
    "SET_TMC_CURRENT",
    "SET_VELOCITY_LIMIT",
    "SYNC",
    "SYNC_EXTRUDER_MOTION",
    "TEMPERATURE_WAIT",
    "Z_TILT_ADJUST",
];

/// Names of firmware macros called by a start G-code, upper-cased, in first-call order: lines
/// whose first word is not a G/M/T code (nor a Prusa `P0` tool code), not a `KEY=value`
/// assignment and not one of [`FIRMWARE_COMMANDS`] (`PRINT_START`,
/// `START_PRINT`, `SM_PRINT_PREHEAT`, …). Comments (`;`, `#`) are dropped; a line starting with
/// an Orca template (`{if …}`, `{endif}`) or placeholder (`[…]`) is not a macro call.
pub fn start_gcode_macros(gcode: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for raw in gcode.lines() {
        let line = raw.split([';', '#']).next().unwrap_or_default().trim();
        if line.is_empty() || line.starts_with('{') || line.starts_with('[') {
            continue;
        }
        let Some(word) = line.split_whitespace().next() else {
            continue;
        };
        let mut chars = word.chars();
        let gcode_word = matches!(
            chars.next().map(|c| c.to_ascii_uppercase()),
            Some('G' | 'M' | 'T' | 'P')
        ) && chars.next().is_some_and(|c| c.is_ascii_digit());
        let identifier = word.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if gcode_word || !identifier {
            continue;
        }
        let name = word.to_ascii_uppercase();
        if !FIRMWARE_COMMANDS.contains(&name.as_str()) && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// `machine_start_gcode` of a resolved machine preset (a string, or Orca's one-element list).
pub fn start_gcode(machine: &Map<String, Value>) -> String {
    match machine.get("machine_start_gcode") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn macro_note(names: &[String]) -> String {
    format!(
        "{MACRO_NOTE_PREFIX} ({}); purge not visible — {MACRO_PURGE_MARGIN_MM:.0} mm edge margin",
        names.join(", ")
    )
}

/// A purge the probe could not see: macro calls in the start G-code and no detected lines.
pub fn hidden_macro_purge(purge_lines: &[[f64; 5]], names: &[String]) -> bool {
    !names.is_empty() && purge_lines.is_empty()
}

/// The `source.notes` entry of a hidden macro purge, `None` without one.
pub fn macro_purge_note(purge_lines: &[[f64; 5]], names: &[String]) -> Option<String> {
    hidden_macro_purge(purge_lines, names).then(|| macro_note(names))
}

/// The macro note put into (or replaced in) `source.notes` as a minimal text edit (see
/// `profile_text`). `None` when the text does not have that shape.
fn with_macro_note(text: &str, names: &[String]) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let note = format!("      {}", serde_json::to_string(&macro_note(names)).ok()?);
    match lines.iter().position(|l| l == "    \"notes\": [") {
        Some(open) => {
            let close = open
                + 1
                + lines[open + 1..]
                    .iter()
                    .position(|l| l.trim_start().starts_with(']'))?;
            if let Some(old) = (open + 1..close).find(|&i| {
                lines[i]
                    .trim_start()
                    .starts_with(&format!("\"{MACRO_NOTE_PREFIX}"))
            }) {
                let comma = if lines[old].ends_with(',') { "," } else { "" };
                lines[old] = format!("{note}{comma}");
            } else {
                if close > open + 1 {
                    lines[close - 1].push(',');
                }
                lines.insert(close, note);
            }
        }
        None => {
            let source = lines.iter().position(|l| l == "  \"source\": {")?;
            lines.splice(
                source + 1..source + 1,
                ["    \"notes\": [".to_owned(), note, "    ],".to_owned()],
            );
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    serde_json::from_str::<Value>(&out).ok()?;
    Some(out)
}

/// One catalog profile with its derived rules written in: `bed_rules`, `gui_flavour`, the
/// matching `edge_margin_mm` and the macro note. The result must load as a profile whose old
/// fields agree with its rules.
fn annotated(text: &str, value: &Value, index: &PresetIndex) -> AppResult<String> {
    let field = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default();
    let machine = index.resolve(field("vendor"), field("orca_machine"))?;
    let facts = PrinterFacts::of_machine(&machine);
    let profile: PrinterProfile = serde_json::from_value(value.clone())?;
    let derived = derive(&profile, field("vendor"), &facts);
    let shape = || "no pretty-printed profile object / source block to edit".to_owned();
    let mut text = text.to_owned();
    if hidden_macro_purge(&profile.purge_lines, &facts.macros) {
        text = with_macro_note(&text, &facts.macros).ok_or_else(shape)?;
    }
    for (key, set) in [
        ("edge_margin_mm", json!(derived.edge_margin_mm)),
        ("bed_rules", serde_json::to_value(&derived.bed_rules)?),
        ("gui_flavour", json!(derived.gui_flavour)),
    ] {
        text = with_top_level(&text, key, &set).ok_or_else(shape)?;
    }
    let written: PrinterProfile = serde_json::from_str(&text)?;
    if let Some(rules) = &written.bed_rules {
        rules.validate()?;
    }
    let mismatches = old_field_mismatches(&written);
    if !mismatches.is_empty() {
        return Err(mismatches.join("; ").into());
    }
    Ok(text)
}

/// `annotate --orca <presets> [--profiles profiles/]`: derives the bed rules, GUI flavour and
/// edge margin of every existing catalog profile (hand-made ones are never touched), each
/// `orca_machine` resolved in its own `vendor` folder, and applies the macro-purge rule. No
/// slicing: the rules come from the profile's fields and its machine preset.
pub fn annotate(raw: Vec<String>) -> AppResult<()> {
    let mut args = raw.into_iter();
    let mut dir = PathBuf::from("profiles");
    let mut orca = None;
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--profiles" => dir = PathBuf::from(value),
            "--orca" => orca = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown option {option}").into()),
        }
    }
    let index = PresetIndex::load(&orca.ok_or("missing --orca")?)?;
    let (mut catalog, mut changed) = (0, Vec::new());
    for path in json_files(&dir)? {
        let text = fs::read_to_string(&path)?;
        let value: Value = serde_json::from_str(&text)?;
        if is_hand_made(&value) {
            continue;
        }
        catalog += 1;
        let edited =
            annotated(&text, &value, &index).map_err(|e| format!("{}: {e}", path.display()))?;
        if edited != text {
            fs::write(&path, edited)?;
            changed.push(path.display().to_string());
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

    #[test]
    fn start_gcode_macros_are_calls_that_are_not_gcode_or_templates() {
        let anycubic = "M190 S[bed_temperature_initial_layer_single]\nM109 S[nozzle_temperature_initial_layer]\n\
            PRINT_START EXTRUDER=[nozzle_temperature_initial_layer] BED=[bed_temperature_initial_layer_single]\n\
            ; START_PRINT in a comment\nG1 X10 ; PRINT_END\n";
        assert_eq!(start_gcode_macros(anycubic), vec!["PRINT_START"]);
        let snapmaker = "{if first_layer_temperature[0] > 200}M104 S200{endif}\n[pre_heat]\nSM_PRINT_PREHEAT\n\
            start_print BED=60\nSM_PRINT_PREHEAT\nT0\ng28\n  M862.3 P \"MK4S\"\n# klipper comment MACRO\n";
        assert_eq!(
            start_gcode_macros(snapmaker),
            vec!["SM_PRINT_PREHEAT", "START_PRINT"]
        );
        // Plain Marlin start: G/M/T codes, assignments and junk words are not macro calls.
        let marlin = "G28 ; home\nM83\nT1\nBED_TEMP=60\n@pause\nG1 X0 Y0 E15 F600 ; purge line\nP0 S1 L2 D0\n";
        assert!(start_gcode_macros(marlin).is_empty());
        // Klipper built-ins and Bambu's SYNC are firmware commands, not purge macros.
        let builtins = "BED_MESH_CALIBRATE PROBE_COUNT=5,5\nSET_VELOCITY_LIMIT ACCEL=500\nSYNC R0 T600\nCUSTOM_PURGE\n";
        assert_eq!(start_gcode_macros(builtins), vec!["CUSTOM_PURGE"]);
        assert!(start_gcode_macros(marlin).is_empty());
        assert!(start_gcode_macros("").is_empty());
    }

    #[test]
    fn the_macro_note_is_only_for_a_purge_the_probe_could_not_see() {
        let names = vec!["PRINT_START".to_owned()];
        let note = macro_purge_note(&[], &names).unwrap();
        assert!(note.contains("PRINT_START") && note.contains("5 mm edge margin"));
        assert!(macro_purge_note(&[[1.0, 2.0, 3.0, 2.0, 1.0]], &names).is_none());
        assert!(macro_purge_note(&[], &[]).is_none());
    }

    #[test]
    fn macro_note_is_a_minimal_text_edit() {
        let names = vec!["PRINT_START".to_owned(), "START_PRINT".to_owned()];
        let text = "{\n  \"edge_margin_mm\": 0.0,\n  \"outline\": [\n    0.49999396104662325\n  ],\n  \"source\": {\n    \"imported\": \"d\",\n    \"notes\": [\n      \"process: p\"\n    ],\n    \"qualified\": true\n  }\n}\n";
        let edited = with_macro_note(text, &names).unwrap();
        assert!(edited.contains("  \"edge_margin_mm\": 0.0,\n"));
        assert!(edited.contains("0.49999396104662325"));
        assert!(edited.contains("      \"process: p\",\n      \"start G-code calls firmware macros (PRINT_START, START_PRINT); purge not visible — 5 mm edge margin\"\n    ],"));
        // Idempotent: the note is replaced, not repeated.
        assert_eq!(with_macro_note(&edited, &names).unwrap(), edited);
        // No notes yet: the array is created inside `source`.
        let bare =
            "{\n  \"edge_margin_mm\": 6.5,\n  \"source\": {\n    \"qualified\": true\n  }\n}\n";
        let edited: Value = serde_json::from_str(&with_macro_note(bare, &names).unwrap()).unwrap();
        assert_eq!(edited["source"]["notes"].as_array().unwrap().len(), 1);
    }

    /// A catalog file as `extract`/`qualify` write it: serde pretty print, alphabetical keys.
    fn catalog_text(vendor: &str, purge: Value) -> String {
        let profile = json!({
            "id": "acme_box", "printer_name": "Acme Box", "printer_model": "Acme Box",
            "vendor": vendor, "orca_machine": "Acme Box 0.4 nozzle", "slicer_kind": "orca",
            "slicer_path": "/opt/slicers/orca/AppRun", "blank_start_gcode_for_cli": false,
            "no_check_for_final_tower_probe": false, "printable_area": [0.0, 0.0, 220.0, 220.0],
            "edge_margin_mm": 0.0, "exclusion_rectangles": [[0.0, 0.0, 18.0, 28.0]],
            "purge_lines": purge, "tower_marker_style": "wipe_tower", "tower_line_width_mm": 0.5,
            "source": {"orca_tag": "v2.4.2", "qualified": true}
        });
        let mut text = serde_json::to_string_pretty(&profile).unwrap();
        text.push('\n');
        text
    }

    fn index(vendor: &str, start_gcode: &str) -> PresetIndex {
        let mut index = PresetIndex::default();
        index.insert(
            vendor,
            json!({"type": "machine", "name": "Acme Box 0.4 nozzle", "instantiation": "true",
                   "printer_model": "Acme Box", "machine_start_gcode": start_gcode}),
        );
        index
    }

    #[test]
    fn annotate_writes_rules_flavour_and_margin_and_is_idempotent() {
        let text = catalog_text("Acme", json!([]));
        let index = index("Acme", "G28\nPRINT_START BED=60\n");
        let once = annotated(&text, &serde_json::from_str(&text).unwrap(), &index).unwrap();
        let profile: PrinterProfile = serde_json::from_str(&once).unwrap();
        assert_eq!(profile.edge_margin_mm, MACRO_PURGE_MARGIN_MM);
        assert_eq!(profile.gui_flavour.unwrap().to_string(), "orca-2.4");
        let rules = profile.bed_rules.unwrap();
        let ids: Vec<&str> = rules.keepouts.iter().map(|k| k.id.as_str()).collect();
        assert_eq!(ids, vec!["bed_exclude_0", "macro_purge"]);
        assert!(once.contains("purge not visible"));
        // Only the touched keys change; the second run changes nothing.
        assert!(once.starts_with("{\n  \"bed_rules\": {"));
        assert!(once.contains("  \"gui_flavour\": \"orca-2.4\",\n  \"id\": \"acme_box\","));
        let twice = annotated(&once, &serde_json::from_str(&once).unwrap(), &index).unwrap();
        assert_eq!(twice, once);
    }

    #[test]
    fn annotate_keeps_a_visible_purge_soft_and_the_margin_at_the_gui_band() {
        let text = catalog_text("Snapmaker", json!([[85.0, 1.0, 185.0, 1.0, 1.8]]));
        let index = index("Snapmaker", "G28\nPRINT_START\n");
        let once = annotated(&text, &serde_json::from_str(&text).unwrap(), &index).unwrap();
        let profile: PrinterProfile = serde_json::from_str(&once).unwrap();
        assert_eq!(profile.edge_margin_mm, 3.5);
        let rules = profile.bed_rules.unwrap();
        assert_eq!(rules.body_edge_band_mm, 3.5);
        assert!(rules.keepouts.iter().all(|k| k.edge_band_mm.is_none()));
        assert!(!once.contains("purge not visible"));
    }
}

//! `annotate`: the macro-purge rule for existing catalog profiles.

use std::{fs, path::PathBuf};

use goat_packer::orca_presets::PresetIndex;
use serde_json::{Map, Value, json};

use crate::AppResult;
use crate::util::{is_hand_made, json_files};

/// Edge margin for a printer whose start G-code purges inside a firmware macro: where the macro
/// draws its line we cannot know, 5 mm is the custom-bed default for the same reason.
const MACRO_PURGE_MARGIN_MM: f64 = 5.0;
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

/// Applies the macro-purge rule to a profile value (qualify): no detected purge lines and macro
/// calls in the start G-code → edge margin at least [`MACRO_PURGE_MARGIN_MM`]. Returns the note.
pub fn apply_macro_margin(profile: &mut Value, names: &[String]) -> Option<String> {
    let purge_empty = profile
        .get("purge_lines")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty);
    if names.is_empty() || !purge_empty {
        return None;
    }
    let current = profile
        .get("edge_margin_mm")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    profile["edge_margin_mm"] = json!(current.max(MACRO_PURGE_MARGIN_MM));
    Some(macro_note(names))
}

/// The same rule as a minimal text edit of a pretty-printed profile (like [`with_vendor_display`]:
/// a serde round-trip would rewrite the floats of qualified purge lines and outlines): the
/// top-level `edge_margin_mm` line is raised to the margin and the note is put into (or replaced
/// in) `source.notes`. `None` when the text does not have that shape.
fn with_macro_margin(text: &str, names: &[String]) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let margin = lines
        .iter()
        .position(|l| l.starts_with("  \"edge_margin_mm\": "))?;
    let value: f64 = lines[margin]["  \"edge_margin_mm\": ".len()..]
        .trim_end_matches(',')
        .trim()
        .parse()
        .ok()?;
    if value < MACRO_PURGE_MARGIN_MM {
        let comma = if lines[margin].ends_with(',') {
            ","
        } else {
            ""
        };
        lines[margin] = format!("  \"edge_margin_mm\": {MACRO_PURGE_MARGIN_MM:.1}{comma}");
    }
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

/// `annotate --orca <presets> [--profiles profiles/]`: the macro-purge rule for existing catalog
/// profiles (hand-made ones are never touched); each `orca_machine` is resolved in its own
/// `vendor` folder. Prints how many profiles call macros and how many files changed.
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
    let paths = json_files(&dir)?;
    let (mut catalog, mut with_macros, mut purge_visible, mut changed) = (0, 0, 0, Vec::new());
    for path in paths {
        let text = fs::read_to_string(&path)?;
        let value: Value = serde_json::from_str(&text)?;
        if is_hand_made(&value) {
            continue;
        }
        catalog += 1;
        let field = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default();
        let machine = index
            .resolve(field("vendor"), field("orca_machine"))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let names = start_gcode_macros(&start_gcode(&machine));
        if names.is_empty() {
            continue;
        }
        with_macros += 1;
        if value
            .get("purge_lines")
            .and_then(Value::as_array)
            .is_some_and(|p| !p.is_empty())
        {
            purge_visible += 1;
            continue;
        }
        let edited = with_macro_margin(&text, &names).ok_or_else(|| {
            format!(
                "{}: no top-level edge_margin_mm / source block to edit",
                path.display()
            )
        })?;
        if edited != text {
            fs::write(&path, edited)?;
            changed.push(format!("{} ({})", field("id"), names.join(", ")));
        }
    }
    println!(
        "{catalog} catalog profiles, {with_macros} call start G-code macros ({purge_visible} with detected purge lines), {} rewritten",
        changed.len()
    );
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
    fn macro_purge_raises_the_margin_only_without_detected_purge() {
        let names = vec!["PRINT_START".to_owned()];
        let mut bare = json!({"edge_margin_mm": 0.0, "purge_lines": []});
        assert!(
            apply_macro_margin(&mut bare, &names)
                .unwrap()
                .contains("PRINT_START")
        );
        assert_eq!(bare["edge_margin_mm"], 5.0);
        let mut wide = json!({"edge_margin_mm": 8.0, "purge_lines": []});
        apply_macro_margin(&mut wide, &names).unwrap();
        assert_eq!(wide["edge_margin_mm"], 8.0);
        let mut purged = json!({"edge_margin_mm": 0.0, "purge_lines": [[1.0, 2.0, 3.0, 2.0, 1.0]]});
        assert!(apply_macro_margin(&mut purged, &names).is_none());
        assert_eq!(purged["edge_margin_mm"], 0.0);
        assert!(apply_macro_margin(&mut bare, &[]).is_none());
    }

    #[test]
    fn macro_margin_is_a_minimal_text_edit() {
        let names = vec!["PRINT_START".to_owned(), "START_PRINT".to_owned()];
        let text = "{\n  \"edge_margin_mm\": 0.0,\n  \"outline\": [\n    0.49999396104662325\n  ],\n  \"source\": {\n    \"imported\": \"d\",\n    \"notes\": [\n      \"process: p\"\n    ],\n    \"qualified\": true\n  }\n}\n";
        let edited = with_macro_margin(text, &names).unwrap();
        assert!(edited.contains("  \"edge_margin_mm\": 5.0,\n"));
        assert!(edited.contains("0.49999396104662325"));
        assert!(edited.contains("      \"process: p\",\n      \"start G-code calls firmware macros (PRINT_START, START_PRINT); purge not visible — 5 mm edge margin\"\n    ],"));
        // Idempotent: the note is replaced, not repeated.
        assert_eq!(with_macro_margin(&edited, &names).unwrap(), edited);
        // No notes yet: the array is created inside `source`.
        let bare =
            "{\n  \"edge_margin_mm\": 6.5,\n  \"source\": {\n    \"qualified\": true\n  }\n}\n";
        let edited: Value =
            serde_json::from_str(&with_macro_margin(bare, &names).unwrap()).unwrap();
        assert_eq!(edited["edge_margin_mm"], 6.5);
        assert_eq!(edited["source"]["notes"].as_array().unwrap().len(), 1);
    }
}

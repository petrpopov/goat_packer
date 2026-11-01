//! `relabel`: recompute `vendor_display` of existing catalog profiles.

use std::{fs, path::PathBuf};

use serde_json::Value;

use crate::AppResult;
use crate::extract::set_vendor_display;
use crate::util::{is_hand_made, json_files};

/// Sets `vendor_display` in a pretty-printed profile by editing its text: a serde round-trip
/// would rewrite floats (serde_json without `float_roundtrip` parses 0.49999396104662325 as
/// …233) all over the qualified purge lines. Keys are alphabetical, so the field goes right
/// after the top-level `"vendor"` line (replaced in place when present).
fn with_vendor_display(text: &str, display: &str) -> Option<String> {
    let value = serde_json::to_string(display).ok()?;
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    if let Some(line) = lines
        .iter_mut()
        .find(|l| l.starts_with("  \"vendor_display\": "))
    {
        let comma = if line.ends_with(',') { "," } else { "" };
        *line = format!("  \"vendor_display\": {value}{comma}");
    } else {
        let at = lines.iter().position(|l| l.starts_with("  \"vendor\": "))?;
        let comma = if lines[at].ends_with(',') {
            ","
        } else {
            lines[at].push(',');
            ""
        };
        lines.insert(at + 1, format!("  \"vendor_display\": {value}{comma}"));
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    serde_json::from_str::<Value>(&out).ok()?;
    Some(out)
}

/// `relabel [--profiles profiles/]`: recomputes `vendor_display` of the catalog profiles (those
/// with a `source` block; hand-made ones are never touched) and rewrites the changed files.
pub fn relabel(raw: Vec<String>) -> AppResult<()> {
    let mut args = raw.into_iter();
    let mut dir = PathBuf::from("profiles");
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--profiles" => dir = PathBuf::from(value),
            _ => return Err(format!("unknown option {option}").into()),
        }
    }
    let paths = json_files(&dir)?;
    let mut catalog = Vec::new();
    for path in paths {
        let value: Value = serde_json::from_slice(&fs::read(&path)?)?;
        if !is_hand_made(&value) {
            catalog.push((path, value));
        }
    }
    let mut profiles: Vec<Value> = catalog.iter().map(|(_, v)| v.clone()).collect();
    set_vendor_display(&mut profiles);
    let mut changed = 0;
    for ((path, before), after) in catalog.iter().zip(&profiles) {
        let Some(display) = after.get("vendor_display").and_then(Value::as_str) else {
            continue;
        };
        if before.get("vendor_display").and_then(Value::as_str) == Some(display) {
            continue;
        }
        let text = fs::read_to_string(path)?;
        let edited = with_vendor_display(&text, display).ok_or_else(|| {
            format!(
                "{}: no top-level \"vendor\" line to put vendor_display after",
                path.display()
            )
        })?;
        fs::write(path, edited)?;
        changed += 1;
    }
    println!("{} catalog profiles, {changed} rewritten", catalog.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::slug;
    use goat_packer::orca_project::read_json;
    use serde_json::json;

    #[test]
    fn relabel_rewrites_catalog_profiles_and_leaves_hand_made_ones() {
        let dir = tempfile::tempdir().unwrap();
        let hand = r#"{"id":"p1s","printer_name":"Bambu Lab P1S","printer_model":"Bambu Lab P1S","vendor":"Bambu Lab"}"#;
        fs::write(dir.path().join("p1s.json"), hand).unwrap();
        for model in ["Bambu Lab A1", "Bambu Lab H2D"] {
            let profile = json!({"id": slug(model), "vendor": "BBL", "printer_model": model, "source": {"qualified": true}});
            fs::write(
                dir.path().join(format!("{}.json", slug(model))),
                serde_json::to_string_pretty(&profile).unwrap(),
            )
            .unwrap();
        }
        relabel(vec!["--profiles".into(), dir.path().display().to_string()]).unwrap();
        let a1 = read_json(&dir.path().join("bambu_lab_a1.json")).unwrap();
        // Text edit, not a serde round-trip: the float keeps its digits.
        let pretty = "{\n  \"purge\": [\n    0.49999396104662325\n  ],\n  \"vendor\": \"BBL\"\n}\n";
        assert_eq!(
            with_vendor_display(pretty, "Bambu Lab").unwrap(),
            "{\n  \"purge\": [\n    0.49999396104662325\n  ],\n  \"vendor\": \"BBL\",\n  \"vendor_display\": \"Bambu Lab\"\n}\n"
        );
        let again =
            with_vendor_display(&with_vendor_display(pretty, "X").unwrap(), "Bambu Lab").unwrap();
        assert_eq!(again, with_vendor_display(pretty, "Bambu Lab").unwrap());
        assert_eq!(
            (a1["vendor"].as_str(), a1["vendor_display"].as_str()),
            (Some("BBL"), Some("Bambu Lab"))
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("p1s.json")).unwrap(),
            hand
        );
    }
}

//! Minimal text edits of a catalog profile. Catalog files are serde_json pretty prints with
//! alphabetical keys; a serde round-trip would rewrite floats all over the qualified purge
//! lines and outlines (serde_json without `float_roundtrip` parses 0.49999396104662325 as
//! …233), so a command changes only the lines of the keys it sets.

use serde_json::Value;

/// A top-level key line of a pretty print: two spaces, then the quoted key.
const KEY_INDENT: &str = "  \"";

/// `text` with the top-level `key` set to `value`: its lines replaced in place, or inserted at
/// the key's alphabetical position. Every other line stays byte for byte. `None` when `text`
/// is not a pretty-printed JSON object or the result does not parse.
pub fn with_top_level(text: &str, key: &str, value: &Value) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let close = lines.iter().rposition(|line| line == "}")?;
    if lines.first().map(String::as_str) != Some("{") {
        return None;
    }
    let keys = top_level_keys(&lines[..close]);
    let rendered = render(key, value)?;
    match keys.iter().position(|(_, name)| name.as_str() >= key) {
        Some(at) if keys[at].1 == key => {
            let end = keys.get(at + 1).map_or(close, |(line, _)| *line);
            let comma = lines[end - 1].ends_with(',');
            lines.splice(keys[at].0..end, with_comma(rendered, comma));
        }
        Some(at) => {
            lines.splice(keys[at].0..keys[at].0, with_comma(rendered, true));
        }
        None => {
            if let Some(last) = lines[..close].last_mut()
                && !keys.is_empty()
            {
                last.push(',');
            }
            lines.splice(close..close, with_comma(rendered, false));
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    serde_json::from_str::<Value>(&out).ok()?;
    Some(out)
}

/// `(line index, key)` of every top-level key, in file order.
fn top_level_keys(lines: &[String]) -> Vec<(usize, String)> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let rest = line.strip_prefix(KEY_INDENT)?;
            let (name, _) = rest.split_once('"')?;
            Some((index, name.to_owned()))
        })
        .collect()
}

/// `"key": value` pretty-printed at the top-level indent, without a trailing comma.
fn render(key: &str, value: &Value) -> Option<Vec<String>> {
    let pretty = serde_json::to_string_pretty(value).ok()?;
    let mut lines: Vec<String> = pretty.lines().map(|line| format!("  {line}")).collect();
    let first = lines.first_mut()?;
    *first = format!(
        "  {}: {}",
        serde_json::to_string(key).ok()?,
        first.trim_start()
    );
    Some(lines)
}

fn with_comma(mut lines: Vec<String>, comma: bool) -> Vec<String> {
    if comma && let Some(last) = lines.last_mut() {
        last.push(',');
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROFILE: &str = "{\n  \"edge_margin_mm\": 0.0,\n  \"id\": \"acme\",\n  \"purge_lines\": [\n    [\n      0.49999396104662325,\n      1.0\n    ]\n  ],\n  \"vendor\": \"Acme\"\n}\n";

    #[test]
    fn a_new_key_goes_to_its_alphabetical_place_and_nothing_else_moves() {
        let edited = with_top_level(PROFILE, "gui_flavour", &json!("orca-2.4")).unwrap();
        assert_eq!(
            edited,
            PROFILE.replace(
                "  \"id\": \"acme\",\n",
                "  \"gui_flavour\": \"orca-2.4\",\n  \"id\": \"acme\",\n"
            )
        );
        let first = with_top_level(PROFILE, "bed_rules", &json!({"keepouts": []})).unwrap();
        assert!(first.starts_with(
            "{\n  \"bed_rules\": {\n    \"keepouts\": []\n  },\n  \"edge_margin_mm\""
        ));
        let last = with_top_level(PROFILE, "zeta", &json!(1)).unwrap();
        assert!(last.ends_with("  \"vendor\": \"Acme\",\n  \"zeta\": 1\n}\n"));
        assert!(edited.contains("0.49999396104662325"));
    }

    #[test]
    fn an_existing_key_is_replaced_in_place_and_the_edit_is_idempotent() {
        let wide = with_top_level(PROFILE, "purge_lines", &json!([[1.0, 2.0]])).unwrap();
        assert!(wide.contains(
            "  \"purge_lines\": [\n    [\n      1.0,\n      2.0\n    ]\n  ],\n  \"vendor\""
        ));
        let margin = with_top_level(PROFILE, "edge_margin_mm", &json!(3.5)).unwrap();
        assert_eq!(
            margin,
            PROFILE.replace("\"edge_margin_mm\": 0.0", "\"edge_margin_mm\": 3.5")
        );
        let vendor = with_top_level(PROFILE, "vendor", &json!("Other")).unwrap();
        assert!(vendor.ends_with("  \"vendor\": \"Other\"\n}\n"));
        assert_eq!(
            with_top_level(&margin, "edge_margin_mm", &json!(3.5)).unwrap(),
            margin
        );
        assert_eq!(
            with_top_level(PROFILE, "edge_margin_mm", &json!(0.0)).unwrap(),
            PROFILE
        );
    }

    #[test]
    fn text_that_is_not_a_pretty_object_is_refused() {
        assert!(with_top_level("[1, 2]", "a", &json!(1)).is_none());
        assert!(with_top_level("{\"a\": 1}", "b", &json!(1)).is_none());
    }
}

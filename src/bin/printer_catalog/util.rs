//! Small helpers shared by the printer_catalog commands.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::AppResult;

/// Hand-made profile: no `source` block, or `source.manual == true`. The catalog never touches it.
pub fn is_hand_made(profile: &Value) -> bool {
    match profile.get("source") {
        None => true,
        Some(source) => source.get("manual").and_then(Value::as_bool) == Some(true),
    }
}

/// The `*.json` files of `dir`, sorted by path.
pub fn json_files(dir: &Path) -> AppResult<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    Ok(paths)
}

/// UTC date `YYYY-MM-DD` from the system clock (no chrono dependency).
pub fn today() -> String {
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

/// A printer model as a file-name-safe id: lowercase alphanumerics joined by single `_`.
pub fn slug(name: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_collapses_punctuation() {
        assert_eq!(slug("Acme Box 0.4 nozzle"), "acme_box_0_4_nozzle");
        assert_eq!(slug("Prusa MK4S (HF)"), "prusa_mk4s_hf");
    }
}

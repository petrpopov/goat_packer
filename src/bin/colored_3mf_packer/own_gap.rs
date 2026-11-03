//! Who decides the OWN support gap (support against its own model): the user's project or us.
//!
//! A first-layer gap the user changed from the slicer's system preset stays. One they never
//! touched (many do not know the parameter) is forced to our value: the probes slice with it
//! and the output 3MF carries it. `support_object_xy_distance` is never forced: nothing shows
//! it matters for goats (the printed-OK plates ran 0.75 on three printers and say nothing
//! about other values), so it stays the project's.

use std::path::Path;

use goat_packer::project_settings_edit::{edit_project_settings_in_3mf, format_setting_mm};

use crate::AppResult;
use crate::gaps::{FIRST_LAYER_KEY, GapSource, OwnGap};

/// Our first-layer gap on the reference nozzle. Evidence: at the slicers' 0.2 tree-support
/// bases overlapped the hooves by ~0.1 mm (the owner's koza-mini notes) and he raised it;
/// 0.75 on a 0.4 nozzle left a 0.88 mm edge gap on the printed A2L 70-goat GUI plate, 1.2 on a
/// 0.6 nozzle ~0.76 mm. It scales with the nozzle: the base is as coarse as its lines.
const FORCED_FIRST_LAYER_MM: f64 = 0.75;
const REFERENCE_NOZZLE_MM: f64 = 0.4;

impl OwnGap {
    /// Decided once at startup from the project's settings. `keep` (`--keep-own-gap`) leaves an
    /// untouched value alone.
    pub fn decide(settings: &serde_json::Value, keep: bool) -> AppResult<OwnGap> {
        let project = OwnGap::of_settings(settings)?;
        if user_set(settings, FIRST_LAYER_KEY) {
            return Ok(project);
        }
        if keep {
            return Ok(OwnGap { source: GapSource::Kept, ..project });
        }
        // Rounded to the three decimals a project stores, so the file and the job agree.
        let scaled = FORCED_FIRST_LAYER_MM * max_nozzle_mm(settings)? / REFERENCE_NOZZLE_MM;
        let first_layer_mm = (scaled * 1000.0).round() / 1000.0;
        Ok(OwnGap { first_layer_mm, source: GapSource::Forced, ..project })
    }

    /// The one progress line when a value is changed from the project's.
    pub fn forced_note(&self) -> Option<String> {
        self.edits_project().then(|| {
            format!(
                "own support gap: {FIRST_LAYER_KEY} forced to {} mm (the project carries {} and \
                 never changed it from the system preset; --keep-own-gap keeps it)",
                format_setting_mm(self.first_layer_mm),
                format_setting_mm(self.project_first_layer_mm)
            )
        })
    }
}

/// `different_settings_to_system` holds one `;`-separated key list per preset slot (print,
/// filaments, printer); a print setting is the user's iff the print slot (the first) names it.
/// Checked on the owner's plates: the P1S and A2L list both gap keys, the U1's 0.75 is the
/// system preset's own and is not listed.
fn user_set(settings: &serde_json::Value, key: &str) -> bool {
    let print_slot = match settings.get("different_settings_to_system") {
        Some(serde_json::Value::Array(slots)) => slots.first().and_then(|slot| slot.as_str()),
        Some(serde_json::Value::String(slot)) => Some(slot.as_str()),
        _ => None,
    };
    print_slot.is_some_and(|slot| slot.split(';').any(|name| name.trim() == key))
}

/// The widest nozzle of the project: on a multi-nozzle printer any of them prints supports.
fn max_nozzle_mm(settings: &serde_json::Value) -> AppResult<f64> {
    let value = settings.get("nozzle_diameter").ok_or(
        "project has no nozzle_diameter to scale the own support gap by; pass --keep-own-gap",
    )?;
    let entries = value.as_array().cloned().unwrap_or_else(|| vec![value.clone()]);
    entries
        .iter()
        .filter_map(|entry| match entry {
            serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
            other => other.as_f64(),
        })
        .filter(|nozzle| nozzle.is_finite() && *nozzle > 0.0)
        .reduce(f64::max)
        .ok_or_else(|| format!("project nozzle_diameter {value} is not a list of diameters").into())
}

/// Puts the forced gap into a copy of the project (a probe copy); a no-op unless forced.
pub fn apply_own_gap(project: &Path, gap: OwnGap) -> AppResult<()> {
    if !gap.edits_project() {
        return Ok(());
    }
    edit_project_settings_in_3mf(project, |settings| {
        settings[FIRST_LAYER_KEY] = format_setting_mm(gap.first_layer_mm).into();
        Ok(())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn untouched() -> serde_json::Value {
        json!({
            "support_object_first_layer_gap": "0.2",
            "support_object_xy_distance": "0.35",
            "nozzle_diameter": ["0.4"],
            "different_settings_to_system": ["seam_position", "", "", ""],
        })
    }

    #[test]
    fn a_listed_key_is_the_users_and_is_kept() {
        let mut settings = untouched();
        settings["support_object_first_layer_gap"] = "1.2".into();
        settings["different_settings_to_system"] =
            json!(["layer_height;support_object_first_layer_gap;x", ""]);
        let gap = OwnGap::decide(&settings, false).unwrap();
        assert_eq!((gap.source, gap.first_layer_mm), (GapSource::User, 1.2));
        assert!(!gap.edits_project() && gap.forced_note().is_none());
    }

    #[test]
    fn only_the_print_slot_counts_and_names_match_whole() {
        let mut settings = untouched();
        settings["different_settings_to_system"] =
            json!(["support_object_first_layer_gap_x", "support_object_first_layer_gap"]);
        assert_eq!(OwnGap::decide(&settings, false).unwrap().source, GapSource::Forced);
    }

    #[test]
    fn an_untouched_key_is_forced_scaled_by_the_widest_nozzle() {
        let gap = OwnGap::decide(&untouched(), false).unwrap();
        assert_eq!((gap.source, gap.first_layer_mm, gap.project_first_layer_mm), (GapSource::Forced, 0.75, 0.2));
        assert_eq!(gap.xy_mm, 0.35);
        assert!(gap.forced_note().unwrap().contains("forced to 0.75"));
        let mut wide = untouched();
        wide["nozzle_diameter"] = json!(["0.4", "0.6"]);
        let gap = OwnGap::decide(&wide, false).unwrap();
        assert!((gap.first_layer_mm - 1.125).abs() < 1e-9, "{gap:?}");
    }

    #[test]
    fn a_preset_value_equal_to_ours_needs_no_edit_and_keep_wins_over_forcing() {
        let mut settings = untouched();
        settings["support_object_first_layer_gap"] = "0.75".into();
        let gap = OwnGap::decide(&settings, false).unwrap();
        assert_eq!(gap.source, GapSource::Forced);
        assert!(!gap.edits_project() && gap.forced_note().is_none());
        let kept = OwnGap::decide(&untouched(), true).unwrap();
        assert_eq!((kept.source, kept.first_layer_mm), (GapSource::Kept, 0.2));
        assert!(!kept.edits_project());
    }

    #[test]
    fn forcing_without_a_nozzle_is_an_error_naming_the_escape_hatch() {
        let mut settings = untouched();
        settings.as_object_mut().unwrap().remove("nozzle_diameter");
        let error = OwnGap::decide(&settings, false).unwrap_err().to_string();
        assert!(error.contains("--keep-own-gap"), "{error}");
        assert!(OwnGap::decide(&settings, true).is_ok());
    }
}

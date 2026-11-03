//! The three distinct gaps of a plate, kept apart on purpose.
//!
//! - OWN gap (a support against its own model): enforced by the slicer from the project's
//!   settings (`support_object_first_layer_gap`, `support_object_xy_distance`). The probes
//!   slice with these values, so the learned supports already reflect them, and the output
//!   3MF carries the very same values (checked after assembly).
//! - FOREIGN gap (a support, or a support base, against a NEIGHBOUR's model body): packer-only,
//!   absent from the 3MF; `goat_packer::ForeignGaps`.
//! - ITERATION: one pack per (own first-layer gap x foreign pair), the best count wins.

use std::path::Path;

use goat_packer::{ForeignGaps, orca_project, project_settings_edit::format_setting_mm};
use serde::{Deserialize, Serialize};

use crate::AppResult;

const FIRST_LAYER_KEY: &str = "support_object_first_layer_gap";
const XY_KEY: &str = "support_object_xy_distance";
/// What the slicers use when a project does not carry the settings (Orca's own defaults, the
/// same ones `support_model::settings` assumes).
const DEFAULT_FIRST_LAYER_MM: f64 = 0.2;
const DEFAULT_XY_MM: f64 = 0.35;
/// Two gap values closer than this are the same setting (a project writes three decimals).
const SAME_GAP_MM: f64 = 5e-4;

/// Default `--foreign-gaps`, floor first, as `base:band` pairs. The floor (0:0.5, the former
/// fixed rule) is what control full-plate slices passed with zero contacts; the ladder only
/// goes UP from it, so spare room on the bed buys a wider margin and a stricter rung wins only
/// when it packs at least as many copies. Every rung is packed (the search is not monotone in
/// the rule); the best count wins and ties keep the larger gaps. Going below the floor needs
/// `--foreign-gap-floor` and is reported as a warning.
pub const DEFAULT_FOREIGN_GAPS: &str = "0:0.5,0.25:0.75,0.5:1.0,0.75:1.5";

/// The support-to-own-model gap settings a project slices with.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct OwnGap {
    pub first_layer_mm: f64,
    pub xy_mm: f64,
}

impl OwnGap {
    /// Read once at startup; every probe and the output 3MF must agree with it.
    pub fn of_settings(settings: &serde_json::Value) -> AppResult<OwnGap> {
        Ok(OwnGap {
            first_layer_mm: setting_mm(settings, FIRST_LAYER_KEY, DEFAULT_FIRST_LAYER_MM)?,
            xy_mm: setting_mm(settings, XY_KEY, DEFAULT_XY_MM)?,
        })
    }
}

fn setting_mm(settings: &serde_json::Value, key: &str, default: f64) -> AppResult<f64> {
    let Some(value) = settings.get(key) else {
        return Ok(default);
    };
    // Per-extruder settings are arrays; these two are scalars, but tolerate a one-entry array.
    let value = value.as_array().and_then(|list| list.first()).unwrap_or(value);
    let number = match value {
        serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
        other => other.as_f64(),
    };
    number
        .filter(|v| v.is_finite() && *v >= 0.0)
        .ok_or_else(|| format!("project setting {key} = {value} is not a distance").into())
}

pub fn same_gap(a: f64, b: f64) -> bool {
    (a - b).abs() < SAME_GAP_MM
}

/// `base:band`, or a bare number as a band with base 0.
pub fn parse_foreign_pair(option: &str, part: &str) -> AppResult<ForeignGaps> {
    let number = |text: &str| -> AppResult<f64> {
        text.trim()
            .parse()
            .map_err(|error| format!("invalid gap {part:?} in {option}: {error}").into())
    };
    let pair = match part.split_once(':') {
        Some((base, band)) => ForeignGaps { base_mm: number(base)?, band_mm: number(band)? },
        None => ForeignGaps { base_mm: 0.0, band_mm: number(part)? },
    };
    pair.check().map_err(|error| format!("{option} {part:?}: {error}"))?;
    Ok(pair)
}

/// `0.5:1.0,0:0.5,0.25` as foreign pairs, distinct, at least one.
pub fn parse_foreign_gaps(text: &str) -> AppResult<Vec<ForeignGaps>> {
    let mut pairs: Vec<ForeignGaps> = Vec::new();
    for part in text.split(',') {
        let pair = parse_foreign_pair("--foreign-gaps", part)?;
        if pairs.contains(&pair) {
            return Err(format!("--foreign-gaps lists {part:?} twice").into());
        }
        pairs.push(pair);
    }
    Ok(pairs)
}

/// The ladder never goes below the floor (default: the former fixed rule, the least the
/// control slices were checked at). Lowering it is the owner's explicit call.
pub fn check_ladder(pairs: &[ForeignGaps], floor: ForeignGaps) -> AppResult<()> {
    match pairs.iter().find(|p| p.base_mm < floor.base_mm || p.band_mm < floor.band_mm) {
        Some(low) => Err(format!(
            "--foreign-gaps {}:{} is below the floor {}:{}; pass --foreign-gap-floor to lower it",
            low.base_mm, low.band_mm, floor.base_mm, floor.band_mm
        )
        .into()),
        None => Ok(()),
    }
}

/// The warning for a floor below the default, `None` at or above it.
pub fn floor_warning(floor: ForeignGaps) -> Option<String> {
    let default = ForeignGaps::default();
    (floor.base_mm < default.base_mm || floor.band_mm < default.band_mm).then(|| {
        format!(
            "foreign gap floor {}:{} is below the default {}:{} that control slices passed with \
             zero contacts; layouts may not be safe",
            floor.base_mm, floor.band_mm, default.base_mm, default.band_mm
        )
    })
}

/// The output 3MF slices with the gaps the probes used. The project settings are copied
/// byte for byte except for the moved-tower rewrite and the nozzle-clean injection; a plate
/// whose slice supports sit closer than the probes learned would invalidate the layout.
pub fn check_output(output: &Path, expected: OwnGap) -> AppResult<()> {
    let settings = serde_json::Value::Object(orca_project::project_settings(output)?);
    let found = OwnGap::of_settings(&settings)?;
    for (key, found, expected) in [
        (FIRST_LAYER_KEY, found.first_layer_mm, expected.first_layer_mm),
        (XY_KEY, found.xy_mm, expected.xy_mm),
    ] {
        if !same_gap(found, expected) {
            return Err(format!(
                "{} carries {key} = {} but the probes sliced with {}",
                output.display(),
                format_setting_mm(found),
                format_setting_mm(expected)
            )
            .into());
        }
    }
    Ok(())
}

/// One packed foreign-gap variant, for the report.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GapVariant {
    pub foreign_base_mm: f64,
    pub foreign_band_mm: f64,
    /// Copies the pack fitted; 0 with `error` when the variant produced no valid layout.
    pub count: usize,
    pub wall_s: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl GapVariant {
    pub fn foreign(&self) -> ForeignGaps {
        ForeignGaps { base_mm: self.foreign_base_mm, band_mm: self.foreign_band_mm }
    }
}

/// The winner: the most copies; ties go to the larger foreign band gap, then the larger base
/// gap (the safer rule), then the earlier variant. `None` when no variant produced a layout.
pub fn choose(variants: &[GapVariant]) -> Option<usize> {
    (0..variants.len())
        .filter(|&i| variants[i].error.is_none())
        .max_by(|&a, &b| {
            let (va, vb) = (&variants[a], &variants[b]);
            va.count
                .cmp(&vb.count)
                .then(va.foreign_band_mm.total_cmp(&vb.foreign_band_mm))
                .then(va.foreign_base_mm.total_cmp(&vb.foreign_base_mm))
                // Equal keys: the earlier variant wins.
                .then(b.cmp(&a))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(base: f64, band: f64, count: usize) -> GapVariant {
        GapVariant { foreign_base_mm: base, foreign_band_mm: band, count, wall_s: 0.0, error: None }
    }

    #[test]
    fn the_most_copies_win_and_ties_keep_the_larger_gaps() {
        let variants = [
            variant(0.0, 0.5, 48),
            variant(0.25, 0.75, 50),
            variant(0.5, 1.0, 50),
            variant(0.75, 1.5, 49),
        ];
        // 50 copies twice: the larger band gap wins.
        assert_eq!(choose(&variants), Some(2));
        let same_band = [variant(0.0, 0.5, 50), variant(0.25, 0.5, 50)];
        assert_eq!(choose(&same_band), Some(1), "larger base gap");
        assert_eq!(choose(&[variant(0.0, 0.5, 50), variant(0.0, 0.5, 50)]), Some(0));
        assert_eq!(choose(&[]), None);
        // The floor wins alone when stricter rungs lose copies.
        assert_eq!(choose(&[variant(0.0, 0.5, 50), variant(0.5, 1.0, 49)]), Some(0));
    }

    #[test]
    fn a_failed_variant_never_wins() {
        let mut failed = variant(0.5, 1.0, 0);
        failed.error = Some("no layout".into());
        assert_eq!(choose(&[failed.clone()]), None);
        assert_eq!(choose(&[failed, variant(0.0, 0.5, 3)]), Some(1));
    }

    #[test]
    fn foreign_pairs_parse_with_a_bare_number_as_the_band() {
        let pairs = parse_foreign_gaps("0.5:1.0, 0.25 ,0:0.5").unwrap();
        assert_eq!(
            pairs,
            vec![
                ForeignGaps { base_mm: 0.5, band_mm: 1.0 },
                ForeignGaps { base_mm: 0.0, band_mm: 0.25 },
                ForeignGaps { base_mm: 0.0, band_mm: 0.5 },
            ]
        );
        for bad in ["", "x", "-1", "0.5:-1", "0:0.5,0:0.5", "NaN:1", "1:2:3"] {
            assert!(parse_foreign_gaps(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse_foreign_gaps(DEFAULT_FOREIGN_GAPS).unwrap().len(), 4);
    }

    #[test]
    fn the_default_ladder_starts_at_the_floor_and_only_goes_up() {
        let ladder = parse_foreign_gaps(DEFAULT_FOREIGN_GAPS).unwrap();
        assert_eq!(ladder[0], ForeignGaps::default());
        assert!(ladder.windows(2).all(|w| w[0].base_mm < w[1].base_mm && w[0].band_mm < w[1].band_mm));
        check_ladder(&ladder, ForeignGaps::default()).unwrap();
    }

    #[test]
    fn a_rung_below_the_floor_is_refused_and_a_lowered_floor_warns() {
        let floor = ForeignGaps::default();
        for low in ["0:0.25", "0.25:0.25", "0:0.1,0.5:1"] {
            let error = check_ladder(&parse_foreign_gaps(low).unwrap(), floor)
                .unwrap_err()
                .to_string();
            assert!(error.contains("--foreign-gap-floor"), "{error}");
        }
        let lowered = ForeignGaps { base_mm: 0.0, band_mm: 0.25 };
        check_ladder(&parse_foreign_gaps("0:0.25,0:0.5").unwrap(), lowered).unwrap();
        assert!(floor_warning(lowered).unwrap().contains("below the default"));
        assert_eq!(floor_warning(floor), None);
        // Raising the floor is fine and silent.
        assert_eq!(floor_warning(ForeignGaps { base_mm: 0.25, band_mm: 1.0 }), None);
    }

    #[test]
    fn the_gaps_are_read_from_strings_with_slicer_defaults() {
        let settings = serde_json::json!({
            "support_object_first_layer_gap": "0.75",
            "support_object_xy_distance": ["0.5"],
        });
        let gap = OwnGap::of_settings(&settings).unwrap();
        assert_eq!((gap.first_layer_mm, gap.xy_mm), (0.75, 0.5));
        let empty = OwnGap::of_settings(&serde_json::json!({})).unwrap();
        assert_eq!((empty.first_layer_mm, empty.xy_mm), (0.2, 0.35));
        assert!(OwnGap::of_settings(&serde_json::json!({XY_KEY: "x"})).is_err());
    }
}

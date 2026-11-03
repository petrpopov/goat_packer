//! What the first-layer and tower models read from a project: the slicer build that will
//! slice it, typed access to `project_settings.config`, and the explicit refusal for settings
//! the models do not describe (design §5.7).

use std::fmt;

use serde_json::{Map, Value};

use crate::paint::StateCoding;

/// The slicer build whose behaviour a prediction follows (design §3.1, spec 04 §0). Builds of
/// one family differ where the geometry or the acceptance changes, so the switch is per build,
/// never per printer.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SlicerBuild {
    /// BambuStudio 02.08 (tag B): BBL tower class, rib walls, chamfer always on.
    BambuStudio,
    /// OrcaSlicer up to 2.4 (tag O): Type 2 tower without ramming reserve or chamfer.
    Orca24,
    /// OrcaSlicer 2.5 and later (tag R): Type 2 tower, the CLI clamps the anchor on every slice.
    Orca25,
    /// Snapmaker Orca 2.4 (tag S): its own Type 2 tower with a brim chamfer, GUI re-clamp.
    SnapmakerOrca,
}

impl SlicerBuild {
    /// Reads a 3MF `Application` value (`BambuStudio-02.08.02.61`, `OrcaSlicer-2.5.0-rc`,
    /// `Snapmaker_Orca-2.4.0`) or a profile `gui_flavour` (`bambu-02.08`, `orca-2.4`,
    /// `snapmaker-orca-2.4`). Unknown versions map to the nearest known build of the family
    /// (design §5.3 step 4); unknown families give `None`.
    pub fn from_name(text: &str) -> Option<SlicerBuild> {
        let lower = text.trim().to_ascii_lowercase();
        let squeezed: String = lower
            .chars()
            .map(|c| if c == '_' || c == ' ' { '-' } else { c })
            .collect();
        if squeezed.starts_with("snapmaker-orca") || squeezed.starts_with("snapmakerorca") {
            return Some(SlicerBuild::SnapmakerOrca);
        }
        if squeezed.starts_with("bambustudio") || squeezed.starts_with("bambu-") {
            return Some(SlicerBuild::BambuStudio);
        }
        let version = squeezed
            .strip_prefix("orcaslicer-")
            .or_else(|| squeezed.strip_prefix("orca-"))?;
        Some(match leading_version(version) {
            Some((major, minor)) if (major, minor) >= (2, 5) => SlicerBuild::Orca25,
            _ => SlicerBuild::Orca24,
        })
    }

    /// How colour paint codes continue past state 2 in this build's 3MF (spec 01 §2.2).
    pub fn colour_coding(self) -> StateCoding {
        match self {
            SlicerBuild::BambuStudio => StateCoding::Bambu,
            _ => StateCoding::Orca,
        }
    }

    /// The tag the behaviour specs use for this build.
    pub fn tag(self) -> &'static str {
        match self {
            SlicerBuild::BambuStudio => "B",
            SlicerBuild::Orca24 => "O",
            SlicerBuild::Orca25 => "R",
            SlicerBuild::SnapmakerOrca => "S",
        }
    }
}

/// `(major, minor)` from a version text such as `2.5.0-rc` or `02.08.02.61`.
fn leading_version(text: &str) -> Option<(u32, u32)> {
    let mut numbers = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u32>().ok());
    Some((numbers.next()??, numbers.next().flatten().unwrap_or(0)))
}

/// A project setting the models do not describe: the prediction refuses instead of guessing,
/// and the pipeline falls back to the slicer or fails with this message (design §5.7).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Unsupported {
    pub setting: String,
    pub value: String,
    pub reason: String,
}

impl Unsupported {
    pub fn new(setting: &str, value: impl fmt::Display, reason: &str) -> Unsupported {
        Unsupported {
            setting: setting.to_owned(),
            value: value.to_string(),
            reason: reason.to_owned(),
        }
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported setting {} = {:?}: {}",
            self.setting, self.value, self.reason
        )
    }
}

impl std::error::Error for Unsupported {}

/// `project_settings.config` with typed reads. Values are strings or arrays of strings; a
/// scalar read takes the first array element (the first extruder or plate).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectSettings {
    values: Map<String, Value>,
}

impl ProjectSettings {
    pub fn new(values: Map<String, Value>) -> ProjectSettings {
        ProjectSettings { values }
    }

    /// From the JSON value of the whole file; anything but an object reads as empty.
    pub fn from_json(value: &Value) -> ProjectSettings {
        ProjectSettings::new(value.as_object().cloned().unwrap_or_default())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// The text of `key` (the first element of an array).
    pub fn text(&self, key: &str) -> Option<&str> {
        match self.values.get(key)? {
            Value::Array(items) => items.first()?.as_str(),
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    /// Every element of `key` as text; a scalar gives one element.
    pub fn texts(&self, key: &str) -> Vec<&str> {
        match self.values.get(key) {
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
            Some(Value::String(text)) => vec![text.as_str()],
            _ => Vec::new(),
        }
    }

    /// The number of `key`; a trailing `%` is dropped (the caller knows what it is a
    /// percentage of).
    pub fn number(&self, key: &str) -> Option<f64> {
        match self.values.get(key)? {
            Value::Array(items) => items.first().and_then(parse_number),
            other => parse_number(other),
        }
    }

    /// Every element of `key` as a number; elements that do not parse are skipped.
    pub fn numbers(&self, key: &str) -> Vec<f64> {
        match self.values.get(key) {
            Some(Value::Array(items)) => items.iter().filter_map(parse_number).collect(),
            Some(other) => parse_number(other).into_iter().collect(),
            None => Vec::new(),
        }
    }

    /// A boolean setting written as `"1"`/`"0"` or `"true"`/`"false"`.
    pub fn flag(&self, key: &str) -> Option<bool> {
        match self.text(key)?.trim() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        }
    }

    /// A length that may be written as a percentage of `base` (`"85%"` of the nozzle).
    pub fn length_or_percent(&self, key: &str, base: f64) -> Option<f64> {
        let text = self.text(key)?.trim();
        match text.strip_suffix('%') {
            Some(percent) => percent.trim().parse::<f64>().ok().map(|p| p / 100.0 * base),
            None => text.parse().ok(),
        }
    }
}

fn parse_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().trim_end_matches('%').trim().parse().ok(),
        _ => None,
    }
}

/// Bead spacing of a line of width `width` and height `height`: the flow model every build
/// shares, `w − h·(1 − π/4)` (spec 03 §0 notation `s`, spec 04 §0 `s_l`).
pub fn bead_spacing(width: f64, height: f64) -> f64 {
    width - height * (1.0 - std::f64::consts::FRAC_PI_4)
}

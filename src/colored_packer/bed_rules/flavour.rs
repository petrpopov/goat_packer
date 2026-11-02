//! The profile's `gui_flavour`: which slicer GUI the owner opens the plate in
//! (`"bambu-02.08"`, `"snapmaker-orca-2.4"`, `"orca-2.4"`). A job resolves its target GUI from
//! the command line or the project's `Application` first (design §5.3); the profile's value is
//! the fallback, and the only one an STL job has.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::gcode_collisions::gui_checks::SNAPMAKER_SPIRAL_LIFT_BAND_MM;

/// The slicer family; the GUI rules and the support behaviour switch on it.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GuiFamily {
    Bambu,
    Orca,
    SnapmakerOrca,
}

impl GuiFamily {
    /// Longest prefix first: `snapmaker-orca-` must not be read as something else.
    const ALL: [GuiFamily; 3] = [GuiFamily::SnapmakerOrca, GuiFamily::Orca, GuiFamily::Bambu];

    fn name(self) -> &'static str {
        match self {
            GuiFamily::Bambu => "bambu",
            GuiFamily::Orca => "orca",
            GuiFamily::SnapmakerOrca => "snapmaker-orca",
        }
    }
}

/// A slicer GUI and its version, written `<family>-<version>`.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct GuiFlavour {
    pub family: GuiFamily,
    /// Dotted digits as the GUI writes them (`02.08`, `2.4`).
    pub version: String,
}

impl GuiFlavour {
    pub fn new(family: GuiFamily, version: &str) -> Self {
        Self {
            family,
            version: version.to_owned(),
        }
    }

    /// The GUI's own restricted band for model parts along the printable edges. Snapmaker
    /// Orca warns about any part bbox closer than 3.5 mm to an edge of a rectangular bed (its
    /// spiral lift could leave the bed); profiles only describe rectangular beds. BambuStudio
    /// and vanilla Orca have no such band.
    pub fn body_edge_band_mm(&self) -> f64 {
        match self.family {
            GuiFamily::SnapmakerOrca => SNAPMAKER_SPIRAL_LIFT_BAND_MM,
            GuiFamily::Bambu | GuiFamily::Orca => 0.0,
        }
    }
}

impl fmt::Display for GuiFlavour {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{}", self.family.name(), self.version)
    }
}

impl From<GuiFlavour> for String {
    fn from(flavour: GuiFlavour) -> String {
        flavour.to_string()
    }
}

impl TryFrom<String> for GuiFlavour {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let unknown = || {
            format!(
                "unknown gui_flavour {text:?}: expected bambu-<version>, orca-<version> or \
                 snapmaker-orca-<version>"
            )
        };
        let (family, version) = GuiFamily::ALL
            .into_iter()
            .find_map(|family| {
                let prefix = format!("{}-", family.name());
                text.strip_prefix(&prefix).map(|version| (family, version))
            })
            .ok_or_else(unknown)?;
        let dotted_digits = version.starts_with(|c: char| c.is_ascii_digit())
            && version.chars().all(|c| c.is_ascii_digit() || c == '.');
        if !dotted_digits {
            return Err(unknown());
        }
        Ok(Self::new(family, version))
    }
}

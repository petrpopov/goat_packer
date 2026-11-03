//! Which slicer build the supports are predicted for (design 2026-11-02 §5.3). A flavour is
//! slicer behaviour, so it may branch code; printers never do. The behaviour switches of
//! design §3.1, spec 01 §7 and spec 02 §2.10 are the functions in `switches`.
//!
//! Resolution order (§5.3): an explicit name, then the source 3MF `Application` metadata,
//! then the profile's `gui_flavour`; an unknown version of a known family maps to the
//! nearest known build and is reported as approximated.

mod switches;

use serde::{Deserialize, Serialize};

use crate::object_print::Margins;
use crate::polygon_ops::Grid;

pub use switches::{
    AnchorClamp, CriticalOnly, ExclusionEncoding, FirstLayerExpansion, GuiExclusionCheck,
    MergeBucket, RootCap, SmallOverhang, TowerChamfer,
};

/// The four slicer builds the behaviour specs describe.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Flavour {
    /// Bambu Studio 02.08.02.61 (owner P1S / A2L GUI and CLI).
    Bambu0208,
    /// OrcaSlicer 2.4.2 (server U1 CLI).
    Orca242,
    /// OrcaSlicer 2.5.0-rc (local CLI).
    Orca250,
    /// Snapmaker Orca 2.4.0 (owner U1 GUI).
    Snapmaker240,
}

/// The slicer family a flavour belongs to.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Bambu,
    Orca,
    SnapmakerOrca,
}

/// Where a resolved flavour came from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlavourSource {
    Explicit,
    Application,
    Profile,
}

/// A flavour chosen for a job, with how it was found.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolvedFlavour {
    pub flavour: Flavour,
    pub source: FlavourSource,
    /// The project names another version of the family; the nearest known build stands in
    /// (report `flavour_approximated: true`, widest margins).
    pub approximated: bool,
}

impl Flavour {
    pub const ALL: [Flavour; 4] = [
        Flavour::Bambu0208,
        Flavour::Orca242,
        Flavour::Orca250,
        Flavour::Snapmaker240,
    ];

    pub fn family(self) -> Family {
        match self {
            Flavour::Bambu0208 => Family::Bambu,
            Flavour::Orca242 | Flavour::Orca250 => Family::Orca,
            Flavour::Snapmaker240 => Family::SnapmakerOrca,
        }
    }

    /// The profile `gui_flavour` / `--slicer-flavour` name (design §7.7).
    pub fn name(self) -> &'static str {
        match self {
            Flavour::Bambu0208 => "bambu-02.08",
            Flavour::Orca242 => "orca-2.4",
            Flavour::Orca250 => "orca-2.5",
            Flavour::Snapmaker240 => "snapmaker-orca-2.4",
        }
    }

    /// The integer grid this build computes on (spec 02 §1.1).
    pub fn grid(self) -> Grid {
        match self.family() {
            Family::Bambu => Grid::TenNanometre,
            Family::Orca | Family::SnapmakerOrca => Grid::Nanometre,
        }
    }

    /// Parses a flavour name: the `name()` forms, the family alone, or a full version.
    pub fn from_name(text: &str) -> Option<Flavour> {
        let text = text.trim().to_ascii_lowercase();
        Flavour::ALL
            .into_iter()
            .find(|flavour| flavour.name() == text)
            .or(match text.as_str() {
                "bambu" | "bambu-02.08.02.61" => Some(Flavour::Bambu0208),
                "orca" | "orca-2.4.2" => Some(Flavour::Orca242),
                "orca-2.5.0" | "orca-2.5.0-rc" => Some(Flavour::Orca250),
                "snapmaker-orca" | "snapmaker-orca-2.4.0" => Some(Flavour::Snapmaker240),
                _ => None,
            })
    }

    /// Reads the 3MF `Application` metadata (`BambuStudio-02.08.02.61`,
    /// `Snapmaker_Orca-2.4.0`, `OrcaSlicer-2.4.2`): `(flavour, exact)`. Orca forks that still
    /// write "BambuStudio" are told apart by the version shape: Bambu versions have four
    /// zero-padded parts (`02.08.02.61`), Orca versions three (`2.4.2`).
    pub fn from_application(application: &str) -> Option<(Flavour, bool)> {
        let (name, version) = split_application(application.trim());
        let parts: Vec<u32> = version
            .split('.')
            .map_while(|part| part.parse().ok())
            .collect();
        let lower = name.to_ascii_lowercase();
        let bambu_shape = parts.len() == 4 && version.starts_with('0');
        if lower.contains("snapmaker") {
            return Some((Flavour::Snapmaker240, parts == [2, 4, 0]));
        }
        if lower.contains("bambu") && bambu_shape {
            return Some((Flavour::Bambu0208, parts == [2, 8, 2, 61]));
        }
        if lower.contains("orca") || lower.contains("bambu") {
            return Some(nearest_orca(&parts));
        }
        None
    }

    /// Resolves the job's flavour (design §5.3): explicit name, then `Application`, then the
    /// profile's `gui_flavour`.
    pub fn resolve(
        explicit: Option<&str>,
        application: Option<&str>,
        gui_flavour: Option<&str>,
    ) -> Result<ResolvedFlavour, String> {
        if let Some(name) = explicit {
            let flavour = Flavour::from_name(name)
                .ok_or_else(|| format!("unknown slicer flavour {name:?}"))?;
            return Ok(ResolvedFlavour {
                flavour,
                source: FlavourSource::Explicit,
                approximated: false,
            });
        }
        if let Some((flavour, exact)) = application.and_then(Flavour::from_application) {
            return Ok(ResolvedFlavour {
                flavour,
                source: FlavourSource::Application,
                approximated: !exact,
            });
        }
        let name = gui_flavour.ok_or(
            "no slicer flavour: the project names no known slicer and the profile has no gui_flavour",
        )?;
        let flavour = Flavour::from_name(name)
            .ok_or_else(|| format!("unknown profile gui_flavour {name:?}"))?;
        Ok(ResolvedFlavour {
            flavour,
            source: FlavourSource::Profile,
            approximated: false,
        })
    }

    /// The first-layer margin δ_L1 of the support envelope, mm (design §5.5): the largest
    /// distance any recorded realization of the build's corpus lies outside the model's
    /// ensemble union at rotations 0° and 180°, plus 0.1 mm (`examples/support_corpus.rs`,
    /// campaign of 2026-11-03).
    pub fn first_layer_margin_mm(self) -> f64 {
        match self {
            // P1S and A2L: 34 observations (CLI runs, GUI runs, the printed-OK plates): worst
            // 2.30 mm, 180°, the feet under the right hoof.
            Flavour::Bambu0208 => 2.4,
            // The owner's Snapmaker GUI plates (4 plates, 8 observations): worst 1.61 mm. O has
            // no support slices of its own and takes S's values.
            Flavour::Snapmaker240 | Flavour::Orca242 => 1.7,
            // Vanilla Orca CLI on the U1 (10 observations): worst 3.50 mm at 180°. R is
            // deterministic, so this is model error, not slicer noise.
            Flavour::Orca250 => 3.6,
        }
    }

    /// The band margin δ_band per absolute 2 mm band, mm (the last entry repeats above): the
    /// per-band worst exceedance of the same campaign plus 0.1 mm, made non-increasing with
    /// height (a trunk that is off low is off by less near its tip).
    pub fn band_margins_mm(self) -> &'static [f64] {
        match self {
            Flavour::Bambu0208 => &[
                5.15, 5.15, 5.15, 4.96, 4.96, 4.81, 3.94, 2.39, 2.05, 1.73, 1.28, 1.04, 0.76,
            ],
            Flavour::Snapmaker240 | Flavour::Orca242 => &[
                2.75, 2.75, 2.75, 2.51, 2.25, 1.91, 1.49, 1.33, 1.33, 1.33, 0.73, 0.61, 0.48,
            ],
            Flavour::Orca250 => &[
                6.56, 6.56, 6.56, 5.81, 4.73, 3.61, 2.33, 1.56, 1.56, 1.19, 0.87, 0.62, 0.5,
            ],
        }
    }

    /// δ_band of absolute band `band`.
    pub fn band_margin_mm(self, band: usize) -> f64 {
        let table = self.band_margins_mm();
        table[band.min(table.len() - 1)]
    }

    /// The margins as one pair: the first layer's and the largest band's.
    pub fn margins(self) -> Margins {
        Margins {
            first_layer_mm: self.first_layer_margin_mm(),
            band_mm: self.band_margins_mm().iter().copied().fold(0.0, f64::max),
        }
    }
}

/// `(name, version)` of `Name-1.2.3`, `Name-1.2.3-rc` or `Name 1.2.3`.
fn split_application(application: &str) -> (&str, &str) {
    let digits_start = |text: &str| text.chars().next().is_some_and(|c| c.is_ascii_digit());
    let mut rest = application;
    while let Some((head, tail)) = rest.rsplit_once(['-', ' ']) {
        if digits_start(tail) {
            return (head, tail);
        }
        rest = head;
    }
    (application, "")
}

/// The Orca build nearest to a version: 2.5 and later map to 2.5.0-rc, older to 2.4.2.
fn nearest_orca(parts: &[u32]) -> (Flavour, bool) {
    let major_minor = (
        parts.first().copied().unwrap_or(0),
        parts.get(1).copied().unwrap_or(0),
    );
    if major_minor >= (2, 5) {
        (Flavour::Orca250, parts == [2, 5, 0])
    } else {
        (Flavour::Orca242, parts == [2, 4, 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_metadata_of_the_owner_projects() {
        assert_eq!(
            Flavour::from_application("BambuStudio-02.08.02.61"),
            Some((Flavour::Bambu0208, true))
        );
        assert_eq!(
            Flavour::from_application("Snapmaker_Orca-2.4.0"),
            Some((Flavour::Snapmaker240, true))
        );
        assert_eq!(
            Flavour::from_application("OrcaSlicer-2.4.2"),
            Some((Flavour::Orca242, true))
        );
        assert_eq!(
            Flavour::from_application("OrcaSlicer-2.5.0-rc"),
            Some((Flavour::Orca250, true))
        );
        assert_eq!(
            Flavour::from_application("BambuStudio-02.07.00.55"),
            Some((Flavour::Bambu0208, false))
        );
        // An Orca fork writing "BambuStudio" with an Orca-shaped version.
        assert_eq!(
            Flavour::from_application("BambuStudio-2.3.1"),
            Some((Flavour::Orca242, false))
        );
        assert_eq!(Flavour::from_application("PrusaSlicer-2.8.0"), None);
    }

    #[test]
    fn resolution_order_is_explicit_application_profile() {
        let resolved = Flavour::resolve(None, Some("Snapmaker_Orca-2.4.0"), Some("orca-2.4"));
        assert_eq!(resolved.unwrap().flavour, Flavour::Snapmaker240);
        let resolved = Flavour::resolve(Some("orca-2.5"), Some("Snapmaker_Orca-2.4.0"), None);
        assert_eq!(resolved.unwrap().source, FlavourSource::Explicit);
        let resolved = Flavour::resolve(None, Some("Cura 5"), Some("bambu-02.08")).unwrap();
        assert_eq!(
            (resolved.flavour, resolved.source),
            (Flavour::Bambu0208, FlavourSource::Profile)
        );
        assert!(Flavour::resolve(None, None, None).is_err());
    }

    #[test]
    fn names_round_trip() {
        for flavour in Flavour::ALL {
            assert_eq!(Flavour::from_name(flavour.name()), Some(flavour));
        }
    }
}

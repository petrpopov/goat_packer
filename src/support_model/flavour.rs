//! Which slicer build the supports are predicted for (design 2026-11-02 §5.3). A flavour is
//! slicer behaviour, so it may branch code; printers never do. The behaviour switches of
//! design §3.1, spec 01 §7 and spec 02 §2.10 are the functions in `switches`.
//!
//! Resolution order (§5.3): an explicit name, then the source 3MF `Application` metadata,
//! then the profile's `gui_flavour`; an unknown version of a known family maps to the
//! nearest known build (Bambu: the newest) and is reported as approximated, with the widest
//! margins of its family. A Bambu project of an older build than the profile's is sliced in
//! the profile's build.

mod switches;

use serde::{Deserialize, Serialize};

use crate::object_print::Margins;
use crate::polygon_ops::Grid;

pub use switches::{
    AnchorClamp, CriticalOnly, ExclusionEncoding, FirstLayerExpansion, GuiExclusionCheck,
    MergeBucket, RootCap, SmallOverhang, TowerChamfer,
};

/// The slicer builds the behaviour specs describe (four), plus Bambu Studio 02.09, which
/// behaves like 02.08 in the model and has its own calibrated margins. Builds of one family
/// are declared oldest first: `Ord` is their release order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Flavour {
    /// Bambu Studio 02.08.02.61 (owner P1S / A2L GUI and CLI until 2026-11-04).
    Bambu0208,
    /// Bambu Studio 02.09.01.70 (owner P1S / A2L GUI from 2026-11-05, server CLI).
    Bambu0209,
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
    pub const ALL: [Flavour; 5] = [
        Flavour::Bambu0208,
        Flavour::Bambu0209,
        Flavour::Orca242,
        Flavour::Orca250,
        Flavour::Snapmaker240,
    ];

    pub fn family(self) -> Family {
        match self {
            Flavour::Bambu0208 | Flavour::Bambu0209 => Family::Bambu,
            Flavour::Orca242 | Flavour::Orca250 => Family::Orca,
            Flavour::Snapmaker240 => Family::SnapmakerOrca,
        }
    }

    /// The profile `gui_flavour` / `--slicer-flavour` name (design §7.7).
    pub fn name(self) -> &'static str {
        match self {
            Flavour::Bambu0208 => "bambu-02.08",
            Flavour::Bambu0209 => "bambu-02.09",
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
                "bambu-02.08.02.61" => Some(Flavour::Bambu0208),
                "bambu" | "bambu-02.09.01.70" => Some(Flavour::NEWEST_BAMBU),
                "orca" | "orca-2.4.2" => Some(Flavour::Orca242),
                "orca-2.5.0" | "orca-2.5.0-rc" => Some(Flavour::Orca250),
                "snapmaker-orca" | "snapmaker-orca-2.4.0" => Some(Flavour::Snapmaker240),
                _ => None,
            })
    }

    /// The `Application` this build's GUI writes into its own projects, when a project made by
    /// another build has to be re-stamped so this GUI opens it as its own (`source_project
    /// --gui-flavour`). `None` for vanilla Orca: an Orca export already carries Orca's own
    /// stamp (a BambuStudio compatibility string plus the `OrcaSlicer` version tag).
    pub fn own_application(self) -> Option<&'static str> {
        match self {
            Flavour::Bambu0208 => Some("BambuStudio-02.08.02.61"),
            Flavour::Bambu0209 => Some("BambuStudio-02.09.01.70"),
            Flavour::Snapmaker240 => Some("Snapmaker_Orca-2.4.0"),
            Flavour::Orca242 | Flavour::Orca250 => None,
        }
    }

    /// The newest calibrated Bambu Studio build: the stand-in for an uncalibrated version.
    pub const NEWEST_BAMBU: Flavour = Flavour::Bambu0209;

    /// The four-part version a Bambu build writes (`02.09.01.70` -> `[2, 9, 1, 70]`).
    fn bambu_version(self) -> Option<[u32; 4]> {
        match self {
            Flavour::Bambu0208 => Some([2, 8, 2, 61]),
            Flavour::Bambu0209 => Some([2, 9, 1, 70]),
            Flavour::Orca242 | Flavour::Orca250 | Flavour::Snapmaker240 => None,
        }
    }

    /// Reads the 3MF `Application` metadata (`BambuStudio-02.08.02.61`,
    /// `Snapmaker_Orca-2.4.0`, `OrcaSlicer-2.4.2`): `(flavour, exact)`. Orca forks that still
    /// write "BambuStudio" are told apart by the version shape: Bambu versions have four
    /// zero-padded parts (`02.08.02.61`), Orca versions three (`2.4.2`). A Bambu version
    /// without its own calibration (older or newer) stands in as the newest calibrated build,
    /// not exact (its margins are then the widest of the family, `ResolvedFlavour`).
    pub fn from_application(application: &str) -> Option<(Flavour, bool)> {
        let (name, version) = split_application(application.trim());
        let parts = version_parts(version);
        let lower = name.to_ascii_lowercase();
        let bambu_shape = parts.len() == 4 && version.starts_with('0');
        if lower.contains("snapmaker") {
            return Some((Flavour::Snapmaker240, parts == [2, 4, 0]));
        }
        if lower.contains("bambu") && bambu_shape {
            let exact = Flavour::ALL
                .into_iter()
                .find(|flavour| flavour.bambu_version().is_some_and(|known| parts == known));
            return Some(exact.map_or((Flavour::NEWEST_BAMBU, false), |flavour| (flavour, true)));
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
            // An Orca export also writes `Application = BambuStudio-<compat>` and differs only
            // by its `OrcaSlicer` tag, which a project keeps after a later BambuStudio save (the
            // owner's P1S project: BambuStudio-02.08.02.61 plus OrcaSlicer 2.5.0-dev, printed
            // from BambuStudio). On a printer whose profile targets BambuStudio the profile
            // decides; an Orca user there passes `--slicer orca`.
            let profile_bambu = gui_flavour
                .and_then(Flavour::from_name)
                .filter(|profile| profile.family() == Family::Bambu);
            if let Some(profile) = profile_bambu
                && flavour.family() == Family::Orca
            {
                return Ok(ResolvedFlavour {
                    flavour: profile,
                    source: FlavourSource::Profile,
                    approximated: false,
                });
            }
            // A Bambu project saved by an older build is sliced by the GUI the user has now,
            // which opens it without a word: the owner's goat projects still say
            // BambuStudio-02.08.02.61 and are sliced in 02.09.01.70, whose supports lie up to
            // 2.4 mm (first layer) and 4.3 mm (bands) outside the 02.08 ensemble. So the
            // profile's Bambu build wins over an older (or equal) project version; a newer
            // project version than the profile knows stays the project's (approximated when
            // uncalibrated).
            if let (Some(profile), Some(project)) = (
                profile_bambu.and_then(Flavour::bambu_version),
                application.and_then(bambu_application_version),
            ) && project <= profile
            {
                return Ok(ResolvedFlavour {
                    flavour: profile_bambu.unwrap_or(flavour),
                    source: FlavourSource::Profile,
                    approximated: false,
                });
            }
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
    /// campaigns of 2026-11-03 and 2026-11-04). B was recalibrated with the overlap readings
    /// (`merge::Overlap`), S (and O with it) too; R keeps the margins of the smaller
    /// ensemble, which the larger one only covers better.
    pub fn first_layer_margin_mm(self) -> f64 {
        match self {
            // P1S and A2L: 133 observations at 0 and 180 degrees: macOS slices at 1-20 threads
            // (the CPU count faked per run), Linux runner slices at 1-20 threads of the same P1S
            // goat, earlier CLI runs and the owner's GUI runs. Worst 0.41 mm (A2L, 0 degrees,
            // one thread). The ensemble without the overlap readings needed 3.6 mm and still
            // missed 4.26 mm on these slices.
            Flavour::Bambu0208 => 0.51,
            // 02.09.01.70, 2026-11-05: 161 single-goat slices (P1S and A2L at 0 and 180 degrees;
            // macOS at 1-20 threads plus 4 repeats at 14 and the probe runs, 119; Linux x86 in
            // docker at 1-20 threads, 42). Worst 2.38 mm (A2L, 180 degrees, 13-16 threads); the
            // misses are systematic, at every thread count: next to 02.08 at the same count the
            // supports above ~12 mm are identical, below it 02.09 lands fewer, differently merged
            // feet (P1S first layer 523-586 mm² vs 668). No reading of the 02.08 model covers it
            // cheaply: Along+Against 2.40 mm (3x the time), extra pair-test readings at 1.6 and
            // 3.2 mm 1.13 mm but a 28 % larger union and 10x the time (P1S/A2L 38/65 vs 36/64).
            Flavour::Bambu0209 => 2.48,
            // U1, 2026-11-04: Snapmaker Orca 2.4.0 slices at 1-20 threads, 0 and 180 degrees
            // (40 observations; its CLI crashes in two GUI-only calls, so the corpus copy had
            // them stubbed): worst 0.67 mm; the owner's printed 48-goat GUI plate lies 0.06 mm
            // outside. O has no support slices of its own and takes S's values.
            Flavour::Snapmaker240 | Flavour::Orca242 => 0.77,
            // Vanilla Orca CLI on the U1 (40 observations): worst 4.85 mm over 40 observations at all rotations. R is
            // deterministic, so this is model error, not slicer noise.
            Flavour::Orca250 => 5.0,
        }
    }

    /// The band margin δ_band per absolute 2 mm band, mm (the last entry repeats above): the
    /// per-band worst exceedance of the same campaign plus 0.1 mm; the 2026-11-03 tables were
    /// made non-increasing with height (a trunk that is off low is off by less near its tip),
    /// B's and S's of 2026-11-04 are not (see their comments).
    pub fn band_margins_mm(self) -> &'static [f64] {
        match self {
            // Same 133 observations, per band (not smoothed: band 0 near the feet misses
            // less than bands 1-2, where A2L at 7-9 threads moves branches by up to 2.2 mm).
            Flavour::Bambu0208 => &[
                1.48, 2.31, 2.12, 1.36, 1.17, 1.13, 1.04, 0.91, 1.13, 0.86, 0.76, 1.0, 0.74,
            ],
            // Same 161 observations, per band. Bands 2-4 (4.26/4.44/4.13) come from ONE Linux
            // slice of the P1S at 16 threads (a trunk 4 mm off; two repeats did not show it);
            // without it 2.58/2.18/1.78 (P1S/A2L 37/65 instead of 36/64). Band 0: A2L at 0
            // degrees misses 3.06 mm at most thread counts on both platforms.
            Flavour::Bambu0209 => &[
                3.16, 2.79, 4.26, 4.44, 4.13, 2.98, 1.99, 1.51, 1.12, 1.05, 1.05, 1.0, 0.74,
            ],
            // Same 40 observations, per band (the GUI plate: 0.71 mm at most).
            Flavour::Snapmaker240 | Flavour::Orca242 => &[
                1.48, 1.99, 1.76, 0.84, 0.81, 0.89, 0.61, 0.69, 0.91, 0.94, 0.49, 0.6, 0.49,
            ],
            Flavour::Orca250 => &[
                7.34, 7.34, 7.34, 5.87, 5.06, 3.89, 2.3, 1.71, 1.71, 1.33, 1.16, 0.63, 0.5,
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

impl ResolvedFlavour {
    /// The flavour's first-layer margin, or for an approximated flavour (a version without
    /// its own calibration) the widest of every calibrated build of its family: nothing says
    /// which build an unknown version behaves like.
    pub fn first_layer_margin_mm(&self) -> f64 {
        self.calibrated()
            .map(Flavour::first_layer_margin_mm)
            .fold(0.0, f64::max)
    }

    /// δ_band of absolute band `band`, widest over the family when approximated.
    pub fn band_margin_mm(&self, band: usize) -> f64 {
        self.calibrated()
            .map(|flavour| flavour.band_margin_mm(band))
            .fold(0.0, f64::max)
    }

    /// The margins as one pair: the first layer's and the largest band's.
    pub fn margins(&self) -> Margins {
        Margins {
            first_layer_mm: self.first_layer_margin_mm(),
            band_mm: self
                .calibrated()
                .flat_map(|flavour| flavour.band_margins_mm().iter().copied())
                .fold(0.0, f64::max),
        }
    }

    /// The builds whose margins apply: the flavour itself, or its whole family.
    fn calibrated(&self) -> impl Iterator<Item = Flavour> + '_ {
        Flavour::ALL.into_iter().filter(move |flavour| {
            *flavour == self.flavour
                || (self.approximated && flavour.family() == self.flavour.family())
        })
    }
}

/// The four-part version of a Bambu-shaped `Application` (`BambuStudio-02.09.01.70`).
fn bambu_application_version(application: &str) -> Option<[u32; 4]> {
    let (name, version) = split_application(application.trim());
    let parts = version_parts(version);
    (name.to_ascii_lowercase().contains("bambu") && version.starts_with('0'))
        .then(|| parts.try_into().ok())
        .flatten()
}

fn version_parts(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map_while(|part| part.parse().ok())
        .collect()
}

/// The application that wrote a project, from its `Application` metadata and the `OrcaSlicer`
/// metadata next to it. OrcaSlicer (2.3.2 on) writes a BambuStudio compatibility string into
/// `Application` (`BambuStudio-02.06.00.51` from 2.4.2, `BambuStudio-02.08.01.55` from
/// 2.5.0-rc, both Bambu-shaped) and its own version into `OrcaSlicer`: read alone, an Orca
/// export passed for BambuStudio 02.08, so an STL project exported by Orca for a U1 was packed
/// by BambuStudio's GUI rules. Such a pair reads as `OrcaSlicer-<version>`; any other
/// `Application` (Snapmaker's `Snapmaker_Orca-2.4.0`, BambuStudio's own) stays as it is.
pub fn project_generator(application: &str, orca_version: Option<&str>) -> String {
    let application = application.trim();
    match orca_version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(version) if application.starts_with("BambuStudio-") => {
            format!("OrcaSlicer-{version}")
        }
        _ => application.to_owned(),
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

    #[test]
    fn an_orca_tagged_project_on_a_bambu_profile_targets_bambu_studio() {
        let resolved = Flavour::resolve(None, Some("OrcaSlicer-2.5.0-dev"), Some("bambu-02.08")).unwrap();
        assert_eq!(resolved.flavour, Flavour::Bambu0208);
        assert_eq!(resolved.source, FlavourSource::Profile);
        let orca_profile = Flavour::resolve(None, Some("OrcaSlicer-2.4.2"), Some("orca-2.4")).unwrap();
        assert_eq!(orca_profile.flavour.family(), Family::Orca);
        let explicit = Flavour::resolve(Some("orca-2.4"), Some("OrcaSlicer-2.5.0-dev"), Some("bambu-02.08")).unwrap();
        assert_eq!(explicit.flavour, Flavour::Orca242);
    }
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
            Flavour::from_application("BambuStudio-02.09.01.70"),
            Some((Flavour::Bambu0209, true))
        );
        // Uncalibrated Bambu versions, older or newer, stand in as the newest calibrated one.
        assert_eq!(
            Flavour::from_application("BambuStudio-02.07.00.55"),
            Some((Flavour::NEWEST_BAMBU, false))
        );
        assert_eq!(
            Flavour::from_application("BambuStudio-02.10.00.12"),
            Some((Flavour::NEWEST_BAMBU, false))
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
    fn an_orca_export_reads_as_orca_not_as_its_bambu_compatibility_string() {
        let orca = project_generator("BambuStudio-02.06.00.51", Some("2.4.2"));
        assert_eq!(orca, "OrcaSlicer-2.4.2");
        assert_eq!(Flavour::from_application(&orca), Some((Flavour::Orca242, true)));
        let local = project_generator("BambuStudio-02.08.01.55", Some("2.5.0-rc"));
        assert_eq!(Flavour::from_application(&local), Some((Flavour::Orca250, true)));
        // BambuStudio's own project and Snapmaker's stamp are not rewritten.
        assert_eq!(project_generator("BambuStudio-02.08.02.61", None), "BambuStudio-02.08.02.61");
        assert_eq!(project_generator("Snapmaker_Orca-2.4.0", Some("2.4.2")), "Snapmaker_Orca-2.4.0");
        assert_eq!(project_generator("BambuStudio-02.08.02.61", Some(" ")), "BambuStudio-02.08.02.61");
    }

    #[test]
    fn own_applications_resolve_to_their_flavour() {
        for flavour in Flavour::ALL {
            if let Some(application) = flavour.own_application() {
                assert_eq!(Flavour::from_application(application), Some((flavour, true)));
            }
        }
    }

    #[test]
    fn an_older_bambu_project_is_sliced_in_the_profiles_newer_build() {
        // The owner's goat projects say 02.08.02.61 and are sliced in 02.09.01.70.
        let older = Flavour::resolve(None, Some("BambuStudio-02.08.02.61"), Some("bambu-02.09")).unwrap();
        assert_eq!(
            (older.flavour, older.source, older.approximated),
            (Flavour::Bambu0209, FlavourSource::Profile, false)
        );
        let uncalibrated_older =
            Flavour::resolve(None, Some("BambuStudio-02.05.00.66"), Some("bambu-02.09")).unwrap();
        assert_eq!(uncalibrated_older.flavour, Flavour::Bambu0209);
        assert!(!uncalibrated_older.approximated);
        // A project of a newer build than the profile's is the project's.
        let newer = Flavour::resolve(None, Some("BambuStudio-02.09.01.70"), Some("bambu-02.08")).unwrap();
        assert_eq!(
            (newer.flavour, newer.source, newer.approximated),
            (Flavour::Bambu0209, FlavourSource::Application, false)
        );
        let unknown = Flavour::resolve(None, Some("BambuStudio-02.10.00.12"), Some("bambu-02.09")).unwrap();
        assert_eq!(
            (unknown.flavour, unknown.source, unknown.approximated),
            (Flavour::NEWEST_BAMBU, FlavourSource::Application, true)
        );
        // Without a Bambu profile the project decides.
        let alone = Flavour::resolve(None, Some("BambuStudio-02.08.02.61"), None).unwrap();
        assert_eq!((alone.flavour, alone.approximated), (Flavour::Bambu0208, false));
    }

    #[test]
    fn an_approximated_flavour_takes_the_widest_margins_of_its_family() {
        let exact = ResolvedFlavour {
            flavour: Flavour::Bambu0209,
            source: FlavourSource::Application,
            approximated: false,
        };
        assert_eq!(exact.first_layer_margin_mm(), Flavour::Bambu0209.first_layer_margin_mm());
        assert_eq!(exact.band_margin_mm(3), Flavour::Bambu0209.band_margin_mm(3));
        let approximated = ResolvedFlavour { approximated: true, ..exact };
        let bambu = [Flavour::Bambu0208, Flavour::Bambu0209];
        let widest = |margin: &dyn Fn(Flavour) -> f64| bambu.into_iter().map(margin).fold(0.0, f64::max);
        assert_eq!(approximated.first_layer_margin_mm(), widest(&|f| f.first_layer_margin_mm()));
        for band in 0..16 {
            assert_eq!(approximated.band_margin_mm(band), widest(&|f| f.band_margin_mm(band)), "{band}");
        }
        let margins = approximated.margins();
        assert_eq!(margins.band_mm, widest(&|f| f.margins().band_mm));
        // Another family's margins never leak in.
        assert!(margins.first_layer_mm < Flavour::Orca250.first_layer_margin_mm());
    }

    #[test]
    fn names_round_trip() {
        for flavour in Flavour::ALL {
            assert_eq!(Flavour::from_name(flavour.name()), Some(flavour));
        }
    }
}

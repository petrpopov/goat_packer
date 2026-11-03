//! The landing: what a normal object prints on layer 0 (spec 05 §2): the union of the normal
//! parts' section at the middle of layer 0, grown by the wall overshoot ε_L with round joins.
//! Elephant-foot compensation is never subtracted: the slicer switches it off locally on
//! narrow features, where the print reaches the section itself (05 §2.1, 0.19 mm at one goat
//! corner).

use super::grow::grow_round;
use super::project::{ProjectSettings, SlicerBuild, Unsupported};
use crate::embedded_mesh::ObjectMesh;
use crate::mesh_section::{SectionOptions, object_section};
use crate::object_print::Loops;

/// Classic walls: the outer wall's material edge lies on the contour; 0.05 mm covers the
/// 0.026 mm corpus maximum past the section (120 slices, 05 §3.1) with 0.024 mm to spare.
pub const CLASSIC_OVERSHOOT_MM: f64 = 0.05;

/// `slice_closing_radius` of every owner project, used when a project omits it.
const DEFAULT_CLOSING_RADIUS_MM: f64 = 0.049;

/// The first-layer body of one rotation class, in its plate-aligned frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Landing {
    pub loops: Loops,
    /// Height of the section plane: the middle of layer 0 (05 §2).
    pub slice_z_mm: f64,
    /// ε_L the section was grown by.
    pub overshoot_mm: f64,
    /// False for Arachne walls: their bound comes from the setting definitions and has no
    /// corpus measurement yet (05 §5 item L2); the report says so.
    pub measured: bool,
}

/// The landing of `mesh` turned by `rotation_deg` about its frame centre (design §4.1).
pub fn landing(
    mesh: &ObjectMesh,
    settings: &ProjectSettings,
    build: SlicerBuild,
    rotation_deg: f64,
) -> Result<Landing, Unsupported> {
    check_landing_settings(settings, build)?;
    let first_layer = first_layer_height(settings)?;
    let (overshoot_mm, measured) = wall_overshoot(settings)?;
    let closing = settings
        .number("slice_closing_radius")
        .unwrap_or(DEFAULT_CLOSING_RADIUS_MM);
    let slice_z_mm = first_layer / 2.0;
    let section = object_section(
        &mesh.rotated(rotation_deg),
        slice_z_mm,
        &SectionOptions::slicer(closing),
    );
    Ok(Landing {
        loops: grow_round(&section, overshoot_mm),
        slice_z_mm,
        overshoot_mm,
        measured,
    })
}

/// `initial_layer_print_height`, required: the slice plane depends on it.
pub fn first_layer_height(settings: &ProjectSettings) -> Result<f64, Unsupported> {
    match settings.number("initial_layer_print_height") {
        Some(height) if height > 0.0 => Ok(height),
        other => Err(Unsupported::new(
            "initial_layer_print_height",
            format!("{other:?}"),
            "the first layer needs a positive height",
        )),
    }
}

/// ε_L and whether it is measured (05 §2.2): classic walls 0.05 mm; Arachne widens a bead
/// between `min_feature_size` and `min_bead_width` to the latter, centred on the medial axis,
/// so it can pass the section by at most ½·(mbw − mfs). Both are percentages of the nozzle.
fn wall_overshoot(settings: &ProjectSettings) -> Result<(f64, bool), Unsupported> {
    match settings.text("wall_generator").unwrap_or("classic") {
        "classic" => Ok((CLASSIC_OVERSHOOT_MM, true)),
        "arachne" => {
            let nozzle = settings.number("nozzle_diameter").unwrap_or(0.4);
            let bead = settings
                .length_or_percent("min_bead_width", nozzle)
                .unwrap_or(0.85 * nozzle);
            let feature = settings
                .length_or_percent("min_feature_size", nozzle)
                .unwrap_or(0.25 * nozzle);
            Ok((CLASSIC_OVERSHOOT_MM.max(0.5 * (bead - feature)), false))
        }
        other => Err(Unsupported::new(
            "wall_generator",
            other,
            "only classic and arachne walls are described (05 §2.2)",
        )),
    }
}

/// The settings that change the first-layer outline in ways nobody measured (05 §2.3,
/// design §5.7).
fn check_landing_settings(
    settings: &ProjectSettings,
    build: SlicerBuild,
) -> Result<(), Unsupported> {
    for key in ["xy_contour_compensation", "xy_hole_compensation"] {
        if let Some(value) = settings.number(key).filter(|value| *value != 0.0) {
            return Err(Unsupported::new(key, value, "it moves the section outline"));
        }
    }
    let classic = settings.text("wall_generator").unwrap_or("classic") == "classic";
    if classic && settings.flag("detect_thin_wall") == Some(true) {
        return Err(Unsupported::new(
            "detect_thin_wall",
            1,
            "thin-wall widths on layer 0 are unmeasured",
        ));
    }
    let fuzzy = settings.text("fuzzy_skin").unwrap_or("none");
    if fuzzy != "none" && settings.flag("fuzzy_skin_first_layer") == Some(true) {
        return Err(Unsupported::new(
            "fuzzy_skin",
            fuzzy,
            "fuzzy skin on layer 0 moves the outline",
        ));
    }
    if let Some(mode) = settings
        .text("slicing_mode")
        .filter(|mode| *mode != "regular")
    {
        return Err(Unsupported::new(
            "slicing_mode",
            mode,
            "only the non-zero section rule is described",
        ));
    }
    if let Some(rafts) = settings.number("raft_layers").filter(|rafts| *rafts > 0.0) {
        return Err(Unsupported::new(
            "raft_layers",
            rafts,
            "a raft changes the first layer",
        ));
    }
    if build == SlicerBuild::BambuStudio
        && settings.flag("enable_circle_compensation") == Some(true)
    {
        return Err(Unsupported::new(
            "enable_circle_compensation",
            1,
            "BambuStudio's hole compensation moves the section",
        ));
    }
    Ok(())
}

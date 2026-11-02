//! Settings the model does not specify (design §5.7): each returns `Unsupported` naming the
//! setting before stage A runs, and the pipeline falls back to the slicer or fails. The model
//! never guesses. Mesh-dependent conditions (variable layer height, disjoint parts, R
//! blockers, the overhang island guard) are checked by the stages that see them.

use super::SupportSettings;
use crate::support_model::SupportError;
use crate::support_model::flavour::Flavour;

/// The support styles that resolve to Organic tree supports (spec 01 §1.2.1).
const ORGANIC_STYLES: [&str; 3] = ["default", "organic", "tree_organic"];

/// Base patterns that select another tip-line generator (spec 01 §4).
const OTHER_BASE_GENERATORS: [&str; 3] = ["honeycomb", "lightning", "hollow"];

/// Ok when every setting is one the behaviour specs describe for `flavour`.
pub fn check_supported(settings: &SupportSettings, flavour: Flavour) -> Result<(), SupportError> {
    check_plate_independent(settings)?;
    if settings.enable_support {
        check_support_style(settings)?;
        check_support_geometry(settings)?;
        check_flavour_specific(settings, flavour)?;
    }
    if flavour.has_dithering_modes() {
        check_snapmaker_modes(settings)?;
    }
    Ok(())
}

/// Conditions that matter even without supports: they change the first layer, the slice
/// planes or the sections every kind is built from.
fn check_plate_independent(settings: &SupportSettings) -> Result<(), SupportError> {
    refuse(
        settings.precise_z_height,
        "precise_z_height",
        "1",
        "rescales the top layers; slice planes change (spec 01 §1.2 rule 6)",
    )?;
    refuse(
        settings.zaa_enabled,
        "zaa_enabled",
        "1",
        "Z anti-aliasing moves the slice planes (spec 01 §1.2 rule 4)",
    )?;
    refuse(
        settings.raft_layers > 0,
        "raft_layers",
        &settings.raft_layers.to_string(),
        "rafts change the first layer and the z gaps (spec 01 §4, 03 §4)",
    )?;
    refuse(
        settings.slicing_mode != "regular",
        "slicing_mode",
        &settings.slicing_mode,
        "only the non-zero fill rule is specified (spec 01 §1.3)",
    )?;
    refuse(
        settings.xy_contour_compensation != 0.0,
        "xy_contour_compensation",
        &settings.xy_contour_compensation.to_string(),
        "changes the object sections (spec 01 §1.4)",
    )?;
    refuse(
        settings.xy_hole_compensation != 0.0,
        "xy_hole_compensation",
        &settings.xy_hole_compensation.to_string(),
        "changes the object sections (spec 01 §1.4)",
    )?;
    refuse(
        settings.circle_compensation,
        "enable_circle_compensation",
        "1",
        "changes the object sections (spec 01 §1.4)",
    )?;
    refuse(
        settings.make_overhang_printable,
        "make_overhang_printable",
        "1",
        "rewrites the sections under overhangs (spec 01 §1.4)",
    )?;
    refuse(
        settings.filament_shrink != 100.0,
        "filament_shrink",
        &format!("{}%", settings.filament_shrink),
        "shrink-scaled slices are not modelled (spec 01 §1.1)",
    )?;
    refuse(
        settings.skirt_loops > 0,
        "skirt_loops",
        &settings.skirt_loops.to_string(),
        "the slicer does not clip the skirt and it is not modelled (design §4.2, Q11)",
    )?;
    refuse(
        settings.detect_thin_wall && settings.wall_generator == "classic",
        "detect_thin_wall",
        "1",
        "thin-wall gap fill can print outside the landing (spec 05 §2.3)",
    )?;
    refuse(
        settings.fuzzy_skin_first_layer,
        "fuzzy_skin_first_layer",
        "1",
        "fuzzy skin on layer 0 moves the landing outline (spec 05 §2.3)",
    )
}

fn check_support_style(settings: &SupportSettings) -> Result<(), SupportError> {
    refuse(
        !settings.support_type.starts_with("tree("),
        "support_type",
        &settings.support_type,
        "only organic tree supports are modelled (spec 01 §1.2.1)",
    )?;
    refuse(
        !ORGANIC_STYLES.contains(&settings.support_style.as_str()),
        "support_style",
        &settings.support_style,
        "only organic tree supports are modelled (spec 01 §1.2.1)",
    )
}

fn check_support_geometry(settings: &SupportSettings) -> Result<(), SupportError> {
    refuse(
        !settings.on_build_plate_only,
        "support_on_build_plate_only",
        "0",
        "supports resting on the model need to-model volumes, unspecified (spec 02 §2.6)",
    )?;
    refuse(
        settings.top_z_distance <= 0.0,
        "support_top_z_distance",
        &settings.top_z_distance.to_string(),
        "zero-gap interfaces follow other roof and z-gap rules (spec 01 §4)",
    )?;
    refuse(
        settings.independent_support_layer_height,
        "independent_support_layer_height",
        "1",
        "support layers off the object grid are not modelled (spec 01 §4)",
    )?;
    refuse(
        OTHER_BASE_GENERATORS.contains(&settings.base_pattern.as_str()),
        "support_base_pattern",
        &settings.base_pattern,
        "this base pattern selects another tip-line generator (spec 01 §4)",
    )?;
    refuse(
        settings.support_filament != 0,
        "support_filament",
        &settings.support_filament.to_string(),
        "a support filament makes the tower depend on the tree (spec 04 §7 N1)",
    )?;
    refuse(
        settings.support_interface_filament != 0,
        "support_interface_filament",
        &settings.support_interface_filament.to_string(),
        "interface filaments (soluble or not) change the roof rules and the tower (spec 01 §4)",
    )
}

/// Conditions only one build treats specially.
fn check_flavour_specific(
    settings: &SupportSettings,
    flavour: Flavour,
) -> Result<(), SupportError> {
    // Orca builds read a zero bottom gap as a zero-gap bottom interface (O `Slicing.cpp`
    // gap assignment); B falls back to the top gap.
    refuse(
        !flavour.fixed_tree_constants() && settings.bottom_z_distance <= 0.0,
        "support_bottom_z_distance",
        &settings.bottom_z_distance.to_string(),
        "zero-gap bottom interfaces follow other rules (spec 01 §4)",
    )?;
    // B unites the wrapping-detection area into its bed obstacle (spec 02 §2.8 step 4).
    refuse(
        flavour.bed_is_obstacle() && settings.wrapping_detection,
        "enable_wrapping_detection",
        "1",
        "the wrapping-area obstacle is not modelled (spec 02 §2.8)",
    )
}

/// Snapmaker's mixed-filament gradient and Z-dithering modes rewrite the layer ranges.
fn check_snapmaker_modes(settings: &SupportSettings) -> Result<(), SupportError> {
    refuse(
        settings.mixed_filament_gradient_mode,
        "mixed_filament_gradient_mode",
        "1",
        "rewrites the layer ranges (spec 01 §1.2.1)",
    )?;
    refuse(
        settings.dithering_z_step_size != 0.0,
        "dithering_z_step_size",
        &settings.dithering_z_step_size.to_string(),
        "rewrites the layer ranges (spec 01 §1.2.1)",
    )?;
    refuse(
        settings.dithering_local_z_mode,
        "dithering_local_z_mode",
        "1",
        "rewrites the layer ranges (spec 01 §1.2.1)",
    )
}

fn refuse(condition: bool, setting: &str, value: &str, reason: &str) -> Result<(), SupportError> {
    if condition {
        Err(SupportError::unsupported(setting, value, reason))
    } else {
        Ok(())
    }
}

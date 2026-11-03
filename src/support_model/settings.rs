//! Support model, stage A inputs: every slicer setting the support behaviour depends on, read
//! once from the project (`config`), checked against what the model specifies (`checks`,
//! design §5.7), and turned into the slicer's exact integer constants per flavour (`derived`,
//! spec 01 §4 and spec 02 §1.2).
//!
//! The settings are flavour-independent raw values; both key families are kept (Bambu's plain
//! tree keys and Orca's `*_organic` ones) and `derived` picks by flavour.

mod checks;
mod config;
mod derived;

use serde::{Deserialize, Serialize};

pub use checks::check_supported;
pub use config::{LayerRange, ProjectConfig};
pub use derived::{Derived, FillParameters, MINIMUM_ROOF_AREA_MM2};

/// The keys `SupportSettings` reads: the support-relevant part of a project, for the cache
/// key of design §5.6 (with the layer ranges and the stored layer profile).
pub const SUPPORT_KEYS: &[&str] = &[
    "layer_height",
    "initial_layer_print_height",
    "min_layer_height",
    "max_layer_height",
    "precise_z_height",
    "zaa_enabled",
    "raft_layers",
    "slice_closing_radius",
    "resolution",
    "slicing_mode",
    "elefant_foot_compensation",
    "elefant_foot_compensation_layers",
    "xy_contour_compensation",
    "xy_hole_compensation",
    "enable_circle_compensation",
    "make_overhang_printable",
    "filament_shrink",
    "nozzle_diameter",
    "line_width",
    "support_line_width",
    "outer_wall_line_width",
    "initial_layer_line_width",
    "raft_first_layer_expansion",
    "enable_support",
    "support_type",
    "support_style",
    "support_threshold_angle",
    "support_critical_regions_only",
    "support_remove_small_overhang",
    "enforce_support_layers",
    "max_bridge_length",
    "tree_support_branch_angle",
    "tree_support_branch_angle_organic",
    "tree_support_branch_distance",
    "tree_support_branch_distance_organic",
    "tree_support_branch_diameter",
    "tree_support_branch_diameter_organic",
    "tree_support_branch_diameter_angle",
    "tree_support_angle_slow",
    "tree_support_tip_diameter",
    "tree_support_top_rate",
    "tree_support_wall_count",
    "support_object_xy_distance",
    "support_object_first_layer_gap",
    "support_top_z_distance",
    "support_bottom_z_distance",
    "top_z_overrides_xy_distance",
    "independent_support_layer_height",
    "support_on_build_plate_only",
    "support_expansion",
    "support_interface_top_layers",
    "support_interface_bottom_layers",
    "support_interface_spacing",
    "support_bottom_interface_spacing",
    "support_base_pattern",
    "support_base_pattern_spacing",
    "support_angle",
    "support_filament",
    "support_interface_filament",
    "min_feature_size",
    "printable_area",
    "bed_exclude_area",
    "enable_wrapping_detection",
    "skirt_loops",
    "detect_thin_wall",
    "fuzzy_skin",
    "fuzzy_skin_first_layer",
    "wall_generator",
    "mixed_filament_gradient_mode",
    "dithering_z_step_size",
    "dithering_local_z_mode",
];

/// A line width as written: absolute mm, or a percentage of the nozzle diameter; 0 = auto.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Width {
    pub value: f64,
    pub percent: bool,
}

impl Width {
    /// The width in mm against `nozzle` (`get_abs_value`); 0 stays 0.
    pub fn absolute(self, nozzle: f64) -> f64 {
        if self.percent {
            nozzle * self.value / 100.0
        } else {
            self.value
        }
    }
}

/// The raw support-relevant settings of one object (project settings plus object overrides).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SupportSettings {
    // Layer grid (spec 01 §1.2).
    pub layer_height: f64,
    /// `initial_layer_print_height`; ≤ 0 means the layer height.
    pub first_layer_height: f64,
    pub min_layer_height: f64,
    pub max_layer_height: f64,
    /// `(min_z, max_z, layer_height)` of each height range.
    pub layer_ranges: Vec<(f64, f64, Option<f64>)>,
    pub layer_profile: Option<Vec<f64>>,
    pub precise_z_height: bool,
    pub zaa_enabled: bool,
    pub raft_layers: i64,
    // Sections (spec 01 §1.3-§1.6).
    pub slice_closing_radius: f64,
    pub resolution: f64,
    pub slicing_mode: String,
    pub elephant_foot_compensation: f64,
    pub elephant_foot_layers: i64,
    pub xy_contour_compensation: f64,
    pub xy_hole_compensation: f64,
    pub circle_compensation: bool,
    pub make_overhang_printable: bool,
    /// `filament_shrink` in percent (100 = none).
    pub filament_shrink: f64,
    // Widths.
    pub nozzle_diameter: f64,
    pub line_width: Width,
    pub support_line_width: Width,
    pub outer_wall_line_width: Width,
    /// `initial_layer_line_width`: the first layer's flow width, the base's step count.
    pub first_layer_line_width: Width,
    // Overhang detection (spec 01 §3).
    pub enable_support: bool,
    pub support_type: String,
    pub support_style: String,
    pub threshold_angle: f64,
    pub critical_regions_only: bool,
    pub remove_small_overhang: bool,
    pub enforce_support_layers: i64,
    pub max_bridge_length: f64,
    // Tree settings: Bambu keys and Orca `*_organic` keys (spec 01 §4).
    pub branch_angle: f64,
    pub branch_angle_organic: f64,
    pub branch_distance: f64,
    pub branch_distance_organic: f64,
    pub branch_diameter: f64,
    pub branch_diameter_organic: f64,
    pub branch_diameter_angle: f64,
    pub angle_slow: f64,
    pub tip_diameter: f64,
    /// `tree_support_top_rate` in percent.
    pub top_rate: f64,
    pub wall_count: i64,
    // Gaps.
    pub xy_distance: f64,
    pub first_layer_gap: f64,
    pub top_z_distance: f64,
    pub bottom_z_distance: f64,
    pub top_z_overrides_xy_distance: bool,
    pub independent_support_layer_height: bool,
    pub on_build_plate_only: bool,
    pub support_expansion: f64,
    /// `raft_first_layer_expansion`: how far the first-layer support base grows (B reads -1
    /// as an automatic 2 mm, Orca builds as none).
    pub first_layer_expansion: f64,
    // Interfaces and fill.
    pub interface_top_layers: i64,
    pub interface_bottom_layers: i64,
    pub interface_spacing: f64,
    pub bottom_interface_spacing: f64,
    pub base_pattern: String,
    pub base_pattern_spacing: f64,
    pub support_angle: f64,
    pub support_filament: i64,
    pub support_interface_filament: i64,
    /// `min_feature_size` as the bare number: the slicers read the percentage as mm (02 §0.4).
    pub min_feature_size: f64,
    // Plate.
    pub printable_area: Vec<[f64; 2]>,
    pub bed_exclude_area: Vec<[f64; 2]>,
    pub wrapping_detection: bool,
    // Read only to refuse what the model does not specify (design §5.7).
    pub skirt_loops: i64,
    pub detect_thin_wall: bool,
    pub fuzzy_skin_first_layer: bool,
    pub wall_generator: String,
    pub mixed_filament_gradient_mode: bool,
    pub dithering_z_step_size: f64,
    pub dithering_local_z_mode: bool,
}

impl SupportSettings {
    /// Reads the settings; missing optional keys take the slicers' defaults.
    pub fn from_config(config: &ProjectConfig) -> Result<SupportSettings, String> {
        let required = |key: &str| {
            config
                .number(key)?
                .ok_or_else(|| format!("project settings lack {key}"))
        };
        let number = |key: &str, default: f64| config.number_or(key, default);
        let integer = |key: &str, default: i64| {
            Ok::<i64, String>(config.number_or(key, default as f64)? as i64)
        };
        let flag = |key: &str, default: bool| config.flag_or(key, default);
        let width = |key: &str| -> Result<Width, String> {
            Ok(Width {
                value: config.number_or(key, 0.0)?,
                percent: config.is_percent(key),
            })
        };
        let layer_height = required("layer_height")?;
        Ok(SupportSettings {
            layer_height,
            first_layer_height: number("initial_layer_print_height", layer_height)?,
            min_layer_height: number("min_layer_height", 0.07)?,
            max_layer_height: number("max_layer_height", 0.0)?,
            layer_ranges: config
                .layer_ranges
                .iter()
                .map(|range| (range.min_z, range.max_z, range.layer_height))
                .collect(),
            layer_profile: config.layer_profile.clone(),
            precise_z_height: flag("precise_z_height", false)?,
            zaa_enabled: flag("zaa_enabled", false)?,
            raft_layers: integer("raft_layers", 0)?,
            slice_closing_radius: number("slice_closing_radius", 0.049)?,
            resolution: number("resolution", 0.01)?,
            slicing_mode: config.text_or("slicing_mode", "regular"),
            elephant_foot_compensation: number("elefant_foot_compensation", 0.0)?,
            elephant_foot_layers: integer("elefant_foot_compensation_layers", 1)?,
            xy_contour_compensation: number("xy_contour_compensation", 0.0)?,
            xy_hole_compensation: number("xy_hole_compensation", 0.0)?,
            circle_compensation: flag("enable_circle_compensation", false)?,
            make_overhang_printable: flag("make_overhang_printable", false)?,
            filament_shrink: number("filament_shrink", 100.0)?,
            nozzle_diameter: required("nozzle_diameter")?,
            line_width: width("line_width")?,
            support_line_width: width("support_line_width")?,
            outer_wall_line_width: width("outer_wall_line_width")?,
            first_layer_line_width: width("initial_layer_line_width")?,
            enable_support: flag("enable_support", false)?,
            support_type: config.text_or("support_type", "normal(auto)"),
            support_style: config.text_or("support_style", "default"),
            threshold_angle: number("support_threshold_angle", 30.0)?,
            critical_regions_only: flag("support_critical_regions_only", false)?,
            remove_small_overhang: flag("support_remove_small_overhang", true)?,
            enforce_support_layers: integer("enforce_support_layers", 0)?,
            max_bridge_length: number("max_bridge_length", 0.0)?,
            branch_angle: number("tree_support_branch_angle", 45.0)?,
            branch_angle_organic: number("tree_support_branch_angle_organic", 40.0)?,
            branch_distance: number("tree_support_branch_distance", 5.0)?,
            branch_distance_organic: number("tree_support_branch_distance_organic", 1.0)?,
            branch_diameter: number("tree_support_branch_diameter", 2.0)?,
            branch_diameter_organic: number("tree_support_branch_diameter_organic", 2.0)?,
            branch_diameter_angle: number("tree_support_branch_diameter_angle", 5.0)?,
            angle_slow: number("tree_support_angle_slow", 25.0)?,
            tip_diameter: number("tree_support_tip_diameter", 0.8)?,
            top_rate: number("tree_support_top_rate", 30.0)?,
            wall_count: integer("tree_support_wall_count", 0)?,
            xy_distance: number("support_object_xy_distance", 0.35)?,
            first_layer_gap: number("support_object_first_layer_gap", 0.2)?,
            top_z_distance: number("support_top_z_distance", 0.2)?,
            bottom_z_distance: number("support_bottom_z_distance", 0.2)?,
            top_z_overrides_xy_distance: flag("top_z_overrides_xy_distance", false)?,
            independent_support_layer_height: flag("independent_support_layer_height", false)?,
            on_build_plate_only: flag("support_on_build_plate_only", false)?,
            support_expansion: number("support_expansion", 0.0)?,
            first_layer_expansion: number("raft_first_layer_expansion", -1.0)?,
            interface_top_layers: integer("support_interface_top_layers", 3)?,
            interface_bottom_layers: integer("support_interface_bottom_layers", -1)?,
            interface_spacing: number("support_interface_spacing", 0.5)?,
            bottom_interface_spacing: number("support_bottom_interface_spacing", 0.5)?,
            base_pattern: config.text_or("support_base_pattern", "default"),
            base_pattern_spacing: number("support_base_pattern_spacing", 2.5)?,
            support_angle: number("support_angle", 0.0)?,
            support_filament: integer("support_filament", 0)?,
            support_interface_filament: integer("support_interface_filament", 0)?,
            min_feature_size: number("min_feature_size", 25.0)?,
            printable_area: config.points("printable_area")?,
            bed_exclude_area: config.points("bed_exclude_area")?,
            wrapping_detection: flag("enable_wrapping_detection", false)?,
            skirt_loops: integer("skirt_loops", 0)?,
            detect_thin_wall: flag("detect_thin_wall", false)?,
            fuzzy_skin_first_layer: config.text_or("fuzzy_skin", "none") != "none"
                && flag("fuzzy_skin_first_layer", false)?,
            wall_generator: config.text_or("wall_generator", "classic"),
            mixed_filament_gradient_mode: flag("mixed_filament_gradient_mode", false)?,
            dithering_z_step_size: number("dithering_z_step_size", 0.0)?,
            dithering_local_z_mode: flag("dithering_local_z_mode", false)?,
        })
    }

    /// The first-layer height the grid uses (`initial_layer_print_height` ≤ 0 means h).
    pub fn first_layer(&self) -> f64 {
        if self.first_layer_height > 0.0 {
            self.first_layer_height
        } else {
            self.layer_height
        }
    }

    /// `support_type` is one of the automatic types (`*(auto)`).
    pub fn is_auto(&self) -> bool {
        self.support_type.ends_with("(auto)")
    }
}

/// Settings of the owner's U1 goat project (the keys stage A reads), for unit tests.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::{ProjectConfig, SupportSettings};

    pub const U1_SETTINGS: &str = r#"{
        "layer_height": "0.28", "initial_layer_print_height": "0.25",
        "min_layer_height": ["0.08"], "max_layer_height": ["0.32"],
        "slice_closing_radius": "0.049", "resolution": "0.03", "slicing_mode": "regular",
        "elefant_foot_compensation": "0.15", "elefant_foot_compensation_layers": "1",
        "nozzle_diameter": ["0.4"], "line_width": "0.42", "support_line_width": "0.42",
        "outer_wall_line_width": "0.42", "enable_support": "1", "support_type": "tree(auto)",
        "support_style": "default", "support_threshold_angle": "30",
        "support_critical_regions_only": "1", "support_remove_small_overhang": "1",
        "max_bridge_length": "10", "tree_support_branch_angle_organic": "40",
        "tree_support_branch_distance_organic": "1", "tree_support_branch_diameter_organic": "2",
        "tree_support_branch_diameter_angle": "5", "tree_support_angle_slow": "25",
        "tree_support_tip_diameter": "0.8", "tree_support_top_rate": "30%",
        "tree_support_wall_count": "0", "support_object_xy_distance": "0.75",
        "support_object_first_layer_gap": "0.75", "support_top_z_distance": "0.24",
        "support_bottom_z_distance": "0.24", "support_on_build_plate_only": "1",
        "support_interface_top_layers": "3", "support_interface_bottom_layers": "3",
        "support_interface_spacing": "0.5", "support_base_pattern": "rectilinear",
        "support_base_pattern_spacing": "2.5", "min_feature_size": "25%",
        "printable_area": ["0.5x1", "270.5x1", "270.5x271", "0.5x271"],
        "bed_exclude_area": ["0x0"]
    }"#;

    pub fn settings() -> SupportSettings {
        SupportSettings::from_config(&ProjectConfig::from_json(U1_SETTINGS).unwrap()).unwrap()
    }
}

//! The slicer's derived support constants on the flavour grid (spec 01 §4, spec 02 §1.2).
//! Every mm → unit conversion truncates the double quotient toward zero (`Grid::length`),
//! so B's integers lose a unit where upstream's do (r_b = 99 999, V_f = 27 999). Values that
//! upstream computes in single precision (flow widths, fill densities) are computed in f32.

use serde::{Deserialize, Serialize};

use super::SupportSettings;
use crate::polygon_ops::Grid;
use crate::support_model::flavour::Flavour;

/// The clamp margin of angles below 90° and of the slow angle below the branch angle (rad).
const ANGLE_EPSILON: f64 = 1e-4;

/// B fixes these in code (B `TreeSupportCommon.hpp:80-89`).
const BAMBU_SLOW_ANGLE_DEG: f64 = 25.0;
const BAMBU_TIP_DIAMETER_MM: f64 = 0.8;
const BAMBU_TOP_RATE_PERCENT: f64 = 30.0;

/// Trunk foot target diameter (`support_tree_bp_diameter`, spec 01 §4).
const FOOT_DIAMETER_MM: f64 = 7.5;

/// Foot growth is capped at this angle per layer (rad, `tan(0.7)`).
const FOOT_GROWTH_ANGLE: f64 = 0.7;

/// To-model settings (inactive while supports stand on the bed only).
const MIN_HEIGHT_TO_MODEL_MM: f64 = 1.0;
const MAX_MODEL_DIAMETER_GROWTH_MM: f64 = 1.0;

/// The automatic first-layer flow width, in nozzle diameters.
const AUTO_FIRST_LAYER_WIDTH_FACTOR: f64 = 1.25;

/// The roof stack ends where the area left is below this (spec 01 §4 "minimum roof area").
pub const MINIMUM_ROOF_AREA_MM2: f64 = 1.0;

/// Support-fill parameters of the tip line generator (spec 01 §4, second table).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct FillParameters {
    /// Flow spacing s = w − h(1 − π/4), f32 mm.
    pub flow_spacing: f32,
    /// s / (s + D), f32.
    pub base_density: f32,
    /// trunc(s / density), grid units (≈ s + D).
    pub base_line_spacing: i64,
    /// min(1, s / (interface spacing + s)), f32.
    pub interface_density: f32,
    /// trunc(s / interface density), grid units (≈ s + 0.5).
    pub roof_line_spacing: i64,
    /// `support_angle` and `support_angle` + 90°, rad.
    pub base_angle: f64,
    pub interface_angle: f64,
}

/// Every derived constant of spec 01 §4 / 02 §1.2. Lengths are grid units unless named `_mm`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Derived {
    pub flavour: Flavour,
    pub grid: Grid,
    /// lh.
    pub layer_height: i64,
    /// w_s: the support flow width (f32) truncated to the grid.
    pub line_width: i64,
    pub roof_line_width: i64,
    /// r_b.
    pub branch_radius: i64,
    /// r_min, the tip radius.
    pub tip_radius: i64,
    /// α and α_s, rad.
    pub branch_angle: f64,
    pub slow_angle: f64,
    /// V_f, V_s: trunc(tan · lh).
    pub move_fast: i64,
    pub move_slow: i64,
    /// The clearance volumes' copies, V − 2 ("rounding guard").
    pub volume_move_fast: i64,
    pub volume_move_slow: i64,
    pub tip_layers: i64,
    /// g: radius growth per layer above the tip (units, double).
    pub radius_growth: f64,
    /// r_bp: the trunk foot target radius.
    pub foot_radius: i64,
    /// g_bp: foot growth per layer.
    pub foot_growth: f64,
    /// L_bp: layers of the foot zone.
    pub foot_layers: i64,
    /// g_ft = max(g_bp − g, 0) and its truncation g_ft′.
    pub foot_increment: f64,
    pub foot_increment_units: i64,
    /// The foot count grows while a branch descends (`Flavour::foot_grows_in_descent`).
    pub foot_in_descent: bool,
    pub increase_until_radius: i64,
    pub increase_until_layer: i64,
    pub to_model_allowed: bool,
    pub min_depth_to_model: i64,
    pub max_model_growth: i64,
    pub xy: i64,
    pub xy_min: i64,
    /// Tips may use the reduced xy distance (xy > xy_min).
    pub tight_xy: bool,
    /// δ = xy − xy_min, added to radii in the regular collision keys (02 §2.2).
    pub xy_delta: i64,
    pub first_layer_gap_mm: f64,
    /// Gaps snapped to the layer grid (`Slicing.cpp` gap assignment).
    pub top_gap_mm: f64,
    pub bottom_gap_mm: f64,
    pub z_top_layers: usize,
    pub z_bottom_layers: usize,
    /// Tips for the overhang of layer k go to layer k − z_delta.
    pub z_delta: usize,
    pub roof_layers: usize,
    pub floor_layers: i64,
    pub bottom_layers: i64,
    /// D: the tip line spacing input.
    pub branch_distance: i64,
    pub top_rate: f64,
    pub connect_length: i64,
    pub roof_line_distance: i64,
    pub base_pattern_spacing: i64,
    pub wall_count: i64,
    pub resolution: i64,
    /// mfs: the `min_feature_size` percentage read as mm (02 §0.4).
    pub min_feature_size: i64,
    pub force_tip_to_roof: bool,
    pub extra_outset: i64,
    pub outset_step: i64,
    pub circle_half_line_change: i64,
    pub tip_dedup_cell: i64,
    pub fill: FillParameters,
    /// `raft_first_layer_expansion` as written (B reads a negative value as automatic).
    pub first_layer_expansion_mm: f64,
    /// The first-layer flow width, mm (the base's step count).
    pub first_layer_width_mm: f64,
}

impl Derived {
    pub fn new(settings: &SupportSettings, flavour: Flavour) -> Derived {
        let grid = flavour.grid();
        let fixed = flavour.fixed_tree_constants();
        let organic = flavour.uses_organic_keys();
        let pick = |plain: f64, organic_value: f64| if organic { organic_value } else { plain };
        let layer_height = grid.length(settings.layer_height);
        let flow_width = support_flow_width(settings);
        let line_width = grid.length(f64::from(flow_width));
        let branch_diameter = grid.length(pick(
            settings.branch_diameter,
            settings.branch_diameter_organic,
        ));
        let tip_diameter = if fixed && !flavour.reads_tip_diameter() {
            BAMBU_TIP_DIAMETER_MM
        } else {
            settings.tip_diameter
        };
        let branch_radius = branch_diameter / 2;
        let tip_radius = if flavour.reads_tip_diameter() {
            grid.length(tip_diameter)
                .clamp(line_width, line_width.max(branch_diameter))
                / 2
        } else {
            grid.length(tip_diameter).clamp(0, branch_diameter) / 2
        };
        let branch_angle = angle(pick(settings.branch_angle, settings.branch_angle_organic))
            .clamp(0.0, std::f64::consts::FRAC_PI_2 - ANGLE_EPSILON);
        let slow_degrees = if fixed {
            BAMBU_SLOW_ANGLE_DEG
        } else {
            settings.angle_slow
        };
        let slow_angle = angle(slow_degrees).clamp(0.0, branch_angle - ANGLE_EPSILON);
        let move_fast = (libm::tan(branch_angle) * layer_height as f64) as i64;
        let move_slow = (libm::tan(slow_angle) * layer_height as f64) as i64;
        let tip_layers =
            ((branch_radius - tip_radius) / (line_width / 3)).max(branch_radius / layer_height);
        let diameter_angle = angle(settings.branch_diameter_angle)
            .clamp(0.0, std::f64::consts::FRAC_PI_2 - ANGLE_EPSILON);
        let radius_growth = libm::tan(diameter_angle) * layer_height as f64;
        let foot_radius = grid.length(FOOT_DIAMETER_MM) / 2;
        let foot_growth =
            (libm::tan(FOOT_GROWTH_ANGLE) * layer_height as f64).min(0.5 * line_width as f64);
        let foot_increment = (foot_growth - radius_growth).max(0.0);
        let (xy, xy_min) = xy_distances(settings, flavour);
        let (top_gap_mm, bottom_gap_mm) = gaps(settings);
        let z_top_layers = layers_of(grid.length(top_gap_mm), layer_height);
        let z_bottom_layers = layers_of(grid.length(bottom_gap_mm), layer_height);
        let (floor_layers, bottom_height) = floor(settings, flavour, layer_height);
        let roof_layers = settings.interface_top_layers.max(0) as usize;
        let top_rate = if fixed {
            BAMBU_TOP_RATE_PERCENT
        } else {
            settings.top_rate
        };
        let tight_xy = xy > xy_min;
        let tip_radius_mm = grid.mm(tip_radius as f64);
        let branch_distance_mm = pick(settings.branch_distance, settings.branch_distance_organic);
        let increase_until_radius = branch_radius;
        Derived {
            flavour,
            grid,
            layer_height,
            line_width,
            roof_line_width: line_width,
            branch_radius,
            tip_radius,
            branch_angle,
            slow_angle,
            move_fast,
            move_slow,
            volume_move_fast: move_fast - 2,
            volume_move_slow: move_slow - 2,
            tip_layers,
            radius_growth,
            foot_radius,
            foot_growth,
            foot_layers: ((foot_radius - branch_radius) as f64 / foot_growth) as i64,
            foot_increment,
            foot_increment_units: foot_increment as i64,
            foot_in_descent: flavour.foot_grows_in_descent(),
            increase_until_radius,
            increase_until_layer: tip_layers * (increase_until_radius / branch_radius),
            to_model_allowed: !settings.on_build_plate_only,
            min_depth_to_model: round_up_divide(grid.length(MIN_HEIGHT_TO_MODEL_MM), layer_height),
            max_model_growth: grid.length(MAX_MODEL_DIAMETER_GROWTH_MM) / 2,
            xy,
            xy_min,
            tight_xy,
            xy_delta: xy - xy_min,
            first_layer_gap_mm: settings.first_layer_gap,
            top_gap_mm,
            bottom_gap_mm,
            z_top_layers,
            z_bottom_layers,
            z_delta: z_top_layers + 1,
            roof_layers,
            floor_layers,
            bottom_layers: if bottom_height > 0 {
                (bottom_height + layer_height / 2) / layer_height
            } else {
                0
            },
            branch_distance: grid.length(branch_distance_mm),
            top_rate,
            connect_length: (line_width as f64 * 100.0 / top_rate
                + (2.0 * tip_radius as f64 - line_width as f64).max(0.0))
                as i64,
            roof_line_distance: grid.length(settings.interface_spacing) + line_width,
            base_pattern_spacing: grid.length(settings.base_pattern_spacing),
            wall_count: settings.wall_count.max(1),
            resolution: grid.length(settings.resolution),
            min_feature_size: grid.length(settings.min_feature_size),
            force_tip_to_roof: roof_layers > 0
                && tip_radius_mm * tip_radius_mm * std::f64::consts::PI > MINIMUM_ROOF_AREA_MM2,
            extra_outset: (tip_radius - line_width / 2).max(0)
                + if tight_xy { line_width / 2 } else { 0 },
            outset_step: line_width / 8,
            circle_half_line_change: circle_half_line_change(grid, tip_radius, line_width),
            tip_dedup_cell: (tip_radius + 1) / 10,
            fill: fill_parameters(settings, grid, flow_width, branch_distance_mm),
            first_layer_expansion_mm: settings.first_layer_expansion,
            first_layer_width_mm: first_layer_width(settings),
        }
    }

    /// R(d, f): the radius of a node with radius height `depth` and foot count `foot`
    /// (spec 02 §1.2): integer interpolation in the tip, then g per layer, plus f·g_ft.
    pub fn radius(&self, depth: i64, foot: f64) -> i64 {
        let base = if depth <= self.tip_layers {
            (self.tip_radius + (self.branch_radius - self.tip_radius) * depth / self.tip_layers)
                as f64
        } else {
            self.branch_radius as f64 + (depth - self.tip_layers) as f64 * self.radius_growth
        };
        (base + foot * self.foot_increment) as i64
    }

    /// r_rec(l): the radius every element at layer `layer` should reach (0 above the foot
    /// zone).
    pub fn recommended_min_radius(&self, layer: i64) -> i64 {
        let widened = (self.foot_layers - layer) as f64;
        if widened > 0.0 {
            (self.branch_radius as f64 + widened * self.foot_growth) as i64
        } else {
            0
        }
    }

    /// Vertex count of a circle of `radius` units drawn with a chord error of `error` units
    /// (upstream passes 25 units where an error is expected; spec 01 §2.6, 02 §1.3).
    pub fn circle_vertices(radius: f64, error: f64) -> usize {
        let step = 2.0 * libm::acos(1.0 - error / radius);
        libm::ceil(2.0 * std::f64::consts::PI / step) as usize
    }
}

fn angle(degrees: f64) -> f64 {
    degrees * std::f64::consts::PI / 180.0
}

fn round_up_divide(value: i64, divisor: i64) -> i64 {
    (value + divisor - 1) / divisor
}

fn layers_of(gap: i64, layer_height: i64) -> usize {
    libm::round(gap as f64 / layer_height as f64) as usize
}

/// The support flow width in mm as the slicer's f32 `Flow` holds it: `support_line_width`,
/// else `line_width`, a percentage of the nozzle, 0 = the nozzle diameter (auto width).
fn support_flow_width(settings: &SupportSettings) -> f32 {
    let nozzle = settings.nozzle_diameter;
    let chosen = if settings.support_line_width.value > 0.0 {
        settings.support_line_width
    } else {
        settings.line_width
    };
    let width = chosen.absolute(nozzle);
    if width > 0.0 {
        width as f32
    } else {
        nozzle as f32
    }
}

/// The external perimeter flow width (f32): `outer_wall_line_width`, else `line_width`,
/// 0 = 1.125 × nozzle (the perimeter auto width).
fn external_perimeter_width(settings: &SupportSettings) -> f32 {
    let nozzle = settings.nozzle_diameter;
    let chosen = if settings.outer_wall_line_width.value > 0.0 {
        settings.outer_wall_line_width
    } else {
        settings.line_width
    };
    let width = chosen.absolute(nozzle);
    if width > 0.0 {
        width as f32
    } else {
        1.125 * nozzle as f32
    }
}

/// `(xy, xy_min)`: B raises xy to 0.01 mm and may cap it by the top gap; xy_min is half the
/// external perimeter width, at most xy (spec 01 §4).
fn xy_distances(settings: &SupportSettings, flavour: Flavour) -> (i64, i64) {
    let grid = flavour.grid();
    let mut xy = if flavour.fixed_tree_constants() {
        grid.length(settings.xy_distance.max(0.01))
    } else {
        grid.length(settings.xy_distance)
    };
    if flavour.fixed_tree_constants() && settings.top_z_overrides_xy_distance {
        let top = grid.length(gaps(settings).0);
        xy = xy.min(top.max(grid.scaled(0.2) as i64));
    }
    let half_wall = grid.length(0.5 * f64::from(external_perimeter_width(settings)));
    (xy, xy.min(half_wall))
}

/// Top and bottom z gaps snapped to the layer grid: round(gap / lh + 1e-4) · lh; a bottom
/// gap of 0 or less takes the top gap (B `Slicing.cpp:115-123`).
fn gaps(settings: &SupportSettings) -> (f64, f64) {
    let top = settings.top_z_distance;
    let bottom = if settings.bottom_z_distance <= 0.0 {
        top
    } else {
        settings.bottom_z_distance
    };
    if settings.independent_support_layer_height {
        return (top, bottom);
    }
    let snap = |gap: f64| libm::round(gap / settings.layer_height + 1e-4) * settings.layer_height;
    (snap(top), snap(bottom))
}

/// `(floor layers, bottom interface height in units)`. B: bottom layers when > 0, else the
/// top count while bottom ≠ 0; Orca: a negative bottom count means "as the top".
fn floor(settings: &SupportSettings, flavour: Flavour, layer_height: i64) -> (i64, i64) {
    let top = settings.interface_top_layers;
    let bottom = settings.interface_bottom_layers;
    if flavour.fixed_tree_constants() {
        let enabled = top > 0 && bottom != 0;
        let count = if bottom > 0 { bottom } else { top };
        (bottom, if enabled { count * layer_height } else { 0 })
    } else {
        let count = if bottom < 0 { top } else { bottom };
        let enabled = top > 0 && count > 0;
        (count, if enabled { count * layer_height } else { 0 })
    }
}

/// How far along a tip circle the line edge moves by half a line width (tip seeding).
fn circle_half_line_change(grid: Grid, tip_radius: i64, line_width: i64) -> i64 {
    if tip_radius < line_width {
        return tip_radius / 2;
    }
    let radius = grid.mm(tip_radius as f64);
    let inner = grid.mm((tip_radius - line_width / 2) as f64);
    grid.scaled(libm::sqrt(radius * radius - inner * inner)) as i64
}

fn fill_parameters(
    settings: &SupportSettings,
    grid: Grid,
    flow_width: f32,
    branch_distance_mm: f64,
) -> FillParameters {
    let height = settings.layer_height as f32;
    let spacing = flow_width - height * (1.0 - 0.25 * std::f64::consts::PI) as f32;
    let base_density = spacing / (spacing + branch_distance_mm as f32);
    let interface_density = (spacing / (settings.interface_spacing as f32 + spacing)).min(1.0);
    FillParameters {
        flow_spacing: spacing,
        base_density,
        base_line_spacing: grid.length(f64::from(spacing) / f64::from(base_density)),
        interface_density,
        roof_line_spacing: grid.length(f64::from(spacing) / f64::from(interface_density)),
        base_angle: angle(settings.support_angle),
        interface_angle: angle(settings.support_angle + 90.0),
    }
}

/// The first-layer line width in mm; automatic (0) is 1.25 nozzle diameters.
fn first_layer_width(settings: &SupportSettings) -> f64 {
    let width = settings
        .first_layer_line_width
        .absolute(settings.nozzle_diameter);
    if width > 0.0 {
        width
    } else {
        AUTO_FIRST_LAYER_WIDTH_FACTOR * settings.nozzle_diameter
    }
}

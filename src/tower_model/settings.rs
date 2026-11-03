//! The tower inputs of `project_settings.config` (spec 04 §2), with each build's names and
//! defaults, and the configurations the model refuses (design §5.7).

use crate::first_layer::{ProjectSettings, SlicerBuild, Unsupported, first_layer_height};

/// Tower line width as a multiple of the nozzle: fixed inside every build's tower code, which
/// ignores the line width settings (04 Appendix A).
const TOWER_LINE_WIDTH_RATIO: f64 = 1.25;
/// Purge per change in BambuStudio's "Saving" prime volume mode (B `Print.cpp:3819-3824`).
const SAVING_PRIME_VOLUME: f64 = 15.0;
/// Snapmaker Orca's default chamfer limit when the project does not store it.
const DEFAULT_CHAMFER_MAX_MM: f64 = 4.0;

/// Multi-tool ramming of one filament (Type 2 builds).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ramming {
    pub enabled: bool,
    /// `filament_multitool_ramming_volume`, mm³.
    pub volume: f64,
    /// `filament_multitool_ramming_flow`, mm³/s.
    pub flow: f64,
}

/// Everything the tower size depends on besides the tool schedule.
#[derive(Clone, Debug, PartialEq)]
pub struct TowerSettings {
    /// Tower line width `pw` (1.25 × nozzle).
    pub line_width: f64,
    pub first_layer_height: f64,
    /// `prime_tower_width`, the configured width W0.
    pub width: f64,
    /// `prime_tower_brim_width`; −1 = auto by tower height (B/O/R).
    pub brim_width: f64,
    /// `wipe_tower_x/y` of plate 1: where the tower's anchor corner is stored.
    pub anchor: (f64, f64),
    /// B: prime volume of each filament (index = filament − 1); every change is sized with
    /// the largest (`bambu`).
    pub prime_volumes: Vec<f64>,
    /// B: `prime_tower_infill_gap` as a factor (1.5 for 150 %).
    pub infill_gap: f64,
    /// B: adhesion category of each filament; changes into one category share a block.
    pub categories: Vec<String>,
    pub rib_wall: bool,
    pub rib_width: f64,
    pub extra_rib_length: f64,
    pub fillet_wall: bool,
    /// Type 2: `prime_volume`, one purge per change.
    pub prime_volume: f64,
    /// Type 2: `wipe_tower_extra_spacing` and `wipe_tower_extra_flow` as factors.
    pub extra_spacing: f64,
    pub extra_flow: f64,
    /// Type 2, per filament.
    pub ramming: Vec<Ramming>,
    /// O: the ramming band is reserved only with `enable_filament_ramming`.
    pub filament_ramming: bool,
    /// O/R: `filament_minimal_purge_on_wipe_tower`, per filament.
    pub minimal_purge: Vec<f64>,
    /// R: `prime_tower_skip_points` (default on) removes the first-line credit.
    pub skip_points: bool,
    /// S: `prime_tower_brim_chamfer` and its width limit.
    pub chamfer: bool,
    pub chamfer_max_width: f64,
}

impl TowerSettings {
    /// `None` when the project prints no tower (`enable_prime_tower` = 0).
    pub fn read(
        settings: &ProjectSettings,
        build: SlicerBuild,
    ) -> Result<Option<TowerSettings>, Unsupported> {
        if settings.flag("enable_prime_tower") == Some(false) {
            return Ok(None);
        }
        refuse_unmodelled(settings, build)?;
        let nozzle = settings.number("nozzle_diameter").unwrap_or(0.4);
        let prime_volumes = if settings.text("prime_volume_mode") == Some("Saving") {
            vec![SAVING_PRIME_VOLUME; settings.texts("filament_prime_volume").len().max(1)]
        } else {
            settings.numbers("filament_prime_volume")
        };
        let filaments = settings.texts("filament_colour").len().max(1);
        Ok(Some(TowerSettings {
            line_width: TOWER_LINE_WIDTH_RATIO * nozzle,
            first_layer_height: first_layer_height(settings)?,
            width: required(settings, "prime_tower_width")?,
            brim_width: settings.number("prime_tower_brim_width").unwrap_or(0.0),
            anchor: (
                settings.number("wipe_tower_x").unwrap_or(0.0),
                settings.number("wipe_tower_y").unwrap_or(0.0),
            ),
            prime_volumes,
            infill_gap: settings.number("prime_tower_infill_gap").unwrap_or(100.0) / 100.0,
            categories: settings
                .texts("filament_adhesiveness_category")
                .iter()
                .map(|c| c.to_string())
                .collect(),
            rib_wall: build == SlicerBuild::BambuStudio
                && settings.flag("prime_tower_rib_wall").unwrap_or(true),
            rib_width: settings.number("prime_tower_rib_width").unwrap_or(8.0),
            extra_rib_length: settings
                .number("prime_tower_extra_rib_length")
                .unwrap_or(0.0),
            fillet_wall: settings.flag("prime_tower_fillet_wall").unwrap_or(true),
            prime_volume: settings.number("prime_volume").unwrap_or(45.0),
            extra_spacing: settings.number("wipe_tower_extra_spacing").unwrap_or(100.0) / 100.0,
            extra_flow: settings.number("wipe_tower_extra_flow").unwrap_or(100.0) / 100.0,
            ramming: ramming(settings, filaments),
            filament_ramming: settings.flag("enable_filament_ramming").unwrap_or(true),
            minimal_purge: settings.numbers("filament_minimal_purge_on_wipe_tower"),
            skip_points: settings.flag("prime_tower_skip_points").unwrap_or(true),
            chamfer: build == SlicerBuild::SnapmakerOrca
                && settings.flag("prime_tower_brim_chamfer").unwrap_or(true),
            chamfer_max_width: settings
                .number("prime_tower_brim_chamfer_max_width")
                .unwrap_or(DEFAULT_CHAMFER_MAX_MM),
        }))
    }

    /// The largest prime volume of the project's filaments, B (15 mm³ in "Saving" mode).
    pub fn largest_prime_volume(&self) -> f64 {
        self.prime_volumes
            .iter()
            .copied()
            .reduce(f64::max)
            .unwrap_or(SAVING_PRIME_VOLUME)
    }

    /// Adhesion category of `filament` (1-based), B; filaments without one share a category.
    pub fn category_of(&self, filament: u32) -> &str {
        let index = (filament as usize).saturating_sub(1);
        self.categories.get(index).map_or("", String::as_str)
    }
}

/// The largest value of a per-filament setting (index = filament − 1) over `filaments`, or
/// `fallback` when none has one: changes on a layer can go into any of its filaments, so the
/// largest is the safe side.
pub fn largest_of(values: &[f64], filaments: &[u32], fallback: f64) -> f64 {
    filaments
        .iter()
        .filter_map(|f| values.get((*f as usize).checked_sub(1)?).copied())
        .reduce(f64::max)
        .unwrap_or(fallback)
}

fn required(settings: &ProjectSettings, key: &str) -> Result<f64, Unsupported> {
    settings
        .number(key)
        .filter(|value| *value > 0.0)
        .ok_or_else(|| Unsupported::new(key, "missing", "the tower size needs it"))
}

fn ramming(settings: &ProjectSettings, filaments: usize) -> Vec<Ramming> {
    let enabled = settings.texts("filament_multitool_ramming");
    let volume = settings.numbers("filament_multitool_ramming_volume");
    let flow = settings.numbers("filament_multitool_ramming_flow");
    (0..filaments)
        .map(|index| Ramming {
            enabled: enabled
                .get(index)
                .is_some_and(|v| *v == "1" || *v == "true"),
            volume: volume.get(index).copied().unwrap_or(0.0),
            flow: flow.get(index).copied().unwrap_or(0.0),
        })
        .collect()
}

/// Tower configurations whose geometry nobody measured or the model does not describe.
fn refuse_unmodelled(settings: &ProjectSettings, build: SlicerBuild) -> Result<(), Unsupported> {
    let refuse = |key: &str, value: &str, reason: &str| Err(Unsupported::new(key, value, reason));
    if settings.number("wipe_tower_rotation_angle").unwrap_or(0.0) != 0.0 {
        return refuse(
            "wipe_tower_rotation_angle",
            settings.text("wipe_tower_rotation_angle").unwrap_or(""),
            "rotated towers are not modelled",
        );
    }
    if settings.flag("wipe_tower_no_sparse_layers") == Some(true) {
        return refuse(
            "wipe_tower_no_sparse_layers",
            "1",
            "skipped sparse layers change the tower layer schedule and its chamfer",
        );
    }
    if settings.text("timelapse_type") == Some("1")
        || settings.flag("enable_wrapping_detection") == Some(true)
    {
        return refuse(
            "timelapse_type",
            "smooth",
            "a tower on every layer without tool changes is not modelled",
        );
    }
    for key in ["support_filament", "support_interface_filament"] {
        if settings.number(key).unwrap_or(0.0) != 0.0
            && settings.flag("enable_support") == Some(true)
        {
            return refuse(
                key,
                settings.text(key).unwrap_or(""),
                "the tower would depend on the tree realization (04 §7 N1)",
            );
        }
    }
    match build {
        SlicerBuild::BambuStudio => {
            if settings.texts("nozzle_diameter").len() > 1 {
                return refuse(
                    "nozzle_diameter",
                    "2 nozzles",
                    "nozzle-change depth is not modelled",
                );
            }
        }
        _ => {
            if settings.flag("single_extruder_multi_material") == Some(true) {
                return refuse(
                    "single_extruder_multi_material",
                    "1",
                    "Type 2 purge sizing from flush volumes is not modelled",
                );
            }
            let wall = settings.text("wipe_tower_wall_type").unwrap_or("rectangle");
            if wall != "rectangle" {
                return refuse(
                    "wipe_tower_wall_type",
                    wall,
                    "only rectangular Type 2 walls were measured (04 §10 Q6)",
                );
            }
        }
    }
    Ok(())
}

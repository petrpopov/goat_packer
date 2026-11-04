//! The tower shape the packer forces into the project (`--tower-width`): a plain rectangle of
//! a chosen width whose depth the slicer sizes.
//!
//! - Width: `prime_tower_width`. BambuStudio's rib wall squares the tower off and ignores the
//!   width (`bambu`), so it is switched off; Type 2 builds have no rib wall.
//! - Depth: the slicer's own (purge volume over the width, at least the depth by tower
//!   height). On the owner's P1S goat plate (Saving, 15 mm³ a change) the purge fits under
//!   the height floor, so widths 20 and 30 both print 11.2 mm deep (BambuStudio CLI slices
//!   2026-11-04); the owner prints 12-20 mm towers in the GUI.
//! - Purge: left as the project has it. BambuStudio's GUI rewrites "Saving" to "Default"
//!   when its printer model changes on load and the printer has one nozzle per extruder
//!   (B `GUI/Tab.cpp:5906-5916`), which makes a width-20 tower 25.2 mm deep instead of 11.2.
//!   Writing the same purge as "Default" with every filament's prime volume at the Saving
//!   15 mm³ prints the identical tower, but the CLI path-conflict check then reports
//!   "WipeTower <-> goat" on the owner's P1S plate with the nearest goat 12 mm away
//!   (02.08.02.61 and 02.09.01.70, 4 of 4 runs; Saving 3 of 3 clean), so it is not done.

use serde_json::Value;

use crate::first_layer::SlicerBuild;
use crate::project_settings_edit::format_setting_mm;

/// `--tower-width` default: the owner's narrowest tower that prints well and saves room.
pub const DEFAULT_TOWER_WIDTH_MM: f64 = 20.0;

/// Rewrites the tower settings of `settings` (the project's `project_settings.config`) to a
/// rectangle `width_mm` wide; `true` when anything changed.
pub fn force_tower_shape(settings: &mut Value, build: SlicerBuild, width_mm: f64) -> bool {
    let Some(map) = settings.as_object_mut() else {
        return false;
    };
    let mut changed = set(map, "prime_tower_width", Value::String(format_setting_mm(width_mm)));
    if build == SlicerBuild::BambuStudio {
        changed |= set(map, "prime_tower_rib_wall", Value::String("0".to_owned()));
    }
    changed
}

fn set(map: &mut serde_json::Map<String, Value>, key: &str, value: Value) -> bool {
    if map.get(key) == Some(&value) {
        return false;
    }
    map.insert(key.to_owned(), value);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_bambu_project_gets_a_plain_tower_and_keeps_its_purge() {
        let mut settings = json!({
            "prime_tower_width": "35", "prime_tower_rib_wall": "1",
            "prime_volume_mode": "Saving", "filament_prime_volume": ["45", "45", "30"],
            "filament_colour": ["#FFFFFF", "#000000", "#FF0000"],
        });
        assert!(force_tower_shape(&mut settings, SlicerBuild::BambuStudio, 20.0));
        assert_eq!(settings["prime_tower_width"], "20");
        assert_eq!(settings["prime_tower_rib_wall"], "0");
        assert_eq!(settings["prime_volume_mode"], "Saving");
        assert_eq!(settings["filament_prime_volume"], json!(["45", "45", "30"]));
        assert!(!force_tower_shape(&mut settings, SlicerBuild::BambuStudio, 20.0));
    }

    #[test]
    fn default_purge_and_type2_builds_keep_their_volumes() {
        let mut bambu = json!({
            "prime_tower_width": "20", "prime_tower_rib_wall": "1",
            "prime_volume_mode": "Default", "filament_prime_volume": ["45", "30"],
        });
        assert!(force_tower_shape(&mut bambu, SlicerBuild::BambuStudio, 20.0));
        assert_eq!(bambu["filament_prime_volume"], json!(["45", "30"]));
        let mut orca = json!({"prime_tower_width": "20", "prime_volume": "20"});
        assert!(!force_tower_shape(&mut orca, SlicerBuild::SnapmakerOrca, 20.0));
        assert!(force_tower_shape(&mut orca, SlicerBuild::SnapmakerOrca, 12.5));
        assert_eq!(orca["prime_tower_width"], "12.5");
        assert!(orca.get("prime_tower_rib_wall").is_none());
    }
}

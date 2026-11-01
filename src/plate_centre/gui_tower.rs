//! What the slicer GUI does to a stored wipe tower position when it opens a 3MF.

use crate::geometry::Rect;

/// Distance the slicer GUI keeps between the stored tower position and the plate edges.
/// BambuStudio (2.8.2.61, `WIPE_TOWER_MARGIN` in libslic3r.h) re-clamps `wipe_tower_x/y` when
/// it opens a 3MF (`Plater` → `set_default_wipe_tower_pos_for_plate(keep_existing)`): a P1S
/// corner tower stored at (6.24, 228.52) opened at (15, 223.84) — the GUI's own position
/// readout, centre (23.58, 232.42) — and its sliced tower met the top-left goat's ear, while
/// the CLI slice (which keeps the stored position) was clean. OrcaSlicer's margin is 1 mm.
fn gui_tower_margin_mm(slicer_kind: &str) -> f64 {
    if slicer_kind == "orca" { 1.0 } else { 15.0 }
}

fn setting_value<'a>(settings: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match settings.get(key)? {
        serde_json::Value::Array(items) => items.first(),
        other => Some(other),
    }
}

fn setting_f64(settings: &serde_json::Value, key: &str) -> Option<f64> {
    match setting_value(settings, key)? {
        serde_json::Value::String(text) => text.trim().trim_end_matches('%').trim().parse().ok(),
        serde_json::Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

fn setting_all_f64(settings: &serde_json::Value, key: &str) -> Vec<f64> {
    match settings.get(key) {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().and_then(|text| text.trim().parse().ok()))
            .collect(),
        _ => setting_f64(settings, key).into_iter().collect(),
    }
}

fn setting_str<'a>(settings: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    setting_value(settings, key)?.as_str()
}

/// BambuStudio's `WipeTower::get_limit_depth_by_height`: minimum tower depth by object height,
/// linear between the table points, clamped to the first/last.
fn limit_depth_by_height(height: f64) -> f64 {
    const TABLE: [(f64, f64); 4] = [(5.0, 5.0), (100.0, 20.0), (250.0, 40.0), (350.0, 60.0)];
    if height <= TABLE[0].0 {
        return TABLE[0].1;
    }
    for pair in TABLE.windows(2) {
        let ((h0, d0), (h1, d1)) = (pair[0], pair[1]);
        if height < h1 {
            return d0 + (height - h0) / (h1 - h0) * (d1 - d0);
        }
    }
    TABLE[TABLE.len() - 1].1
}

/// The tower size the slicer GUI assumes before slicing (`PartPlate::estimate_wipe_tower_size`
/// of BambuStudio 2.8.2.61), which is what its open-time clamp uses — not the sliced size.
/// Every filament of the project counts as used on the plate (the GUI counts the plate's real
/// extruders): more filaments only make the estimate larger, the clamp stricter. `None` when
/// the GUI would show no tower or a setting is missing.
pub fn gui_tower_estimate(settings: &serde_json::Value, object_height: f64) -> Option<(f64, f64)> {
    let layer_height = setting_f64(settings, "layer_height")?;
    let extra_spacing = setting_f64(settings, "prime_tower_infill_gap").unwrap_or(100.0) / 100.0;
    let rib_wall = setting_str(settings, "prime_tower_rib_wall").is_none_or(|value| value == "1");
    let width = setting_f64(settings, "prime_tower_width")?;
    let filaments = settings.get("filament_colour")?.as_array()?.len();
    let nozzles = settings
        .get("nozzle_diameter")
        .and_then(|v| v.as_array())
        .map_or(1, Vec::len);
    let mut wipe_volume = setting_all_f64(settings, "filament_prime_volume")
        .into_iter()
        .fold(0.0, f64::max);
    if setting_str(settings, "prime_volume_mode") == Some("Saving") {
        wipe_volume = 15.0;
    }
    // `timelapse_type` 1 = smooth: the GUI then shows a tower even for one filament.
    let need_tower = setting_str(settings, "timelapse_type") == Some("1")
        || setting_str(settings, "enable_wrapping_detection") == Some("1");
    let mut volume = wipe_volume
        * if nozzles == 2 {
            filaments
        } else {
            filaments.saturating_sub(1)
        } as f64;
    if nozzles == 2 {
        let length = setting_all_f64(settings, "filament_change_length")
            .into_iter()
            .fold(0.0, f64::max);
        let diameter = setting_all_f64(settings, "filament_diameter")
            .into_iter()
            .fold(0.0, f64::max);
        let diameter = if diameter > 0.0 { diameter } else { 1.75 };
        volume +=
            length * std::f64::consts::PI * diameter * diameter / 4.0 * (filaments / 2) as f64;
    }
    if layer_height <= 0.0 {
        return None;
    }
    let min_depth = limit_depth_by_height(object_height);
    if rib_wall {
        if !need_tower && filaments < 2 {
            return None;
        }
        let volume_depth = (volume / layer_height * extra_spacing).sqrt();
        let depth = volume_depth.max(min_depth);
        let rib_width = setting_f64(settings, "prime_tower_rib_width")
            .unwrap_or(8.0)
            .min(depth / 2.0);
        let extra_rib = setting_f64(settings, "prime_tower_extra_rib_length").unwrap_or(0.0);
        let side = rib_width / std::f64::consts::SQRT_2 + (depth + extra_rib).max(volume_depth);
        Some((side, side))
    } else {
        let depth = volume / (layer_height * width) * extra_spacing;
        if !need_tower && depth <= 1e-9 {
            return None;
        }
        Some((width, depth.max(min_depth)))
    }
}

/// Where the slicer GUI keeps a stored tower position (`wipe_tower_x/y`, the tower's min
/// corner) on open: `[plate min + margin, plate max − estimated size − margin]` per axis.
/// `None` when the range is empty (the GUI would pin the tower anyway) or no estimate exists.
pub fn gui_tower_anchor_range(
    settings: &serde_json::Value,
    slicer_kind: &str,
    plate: Rect,
    object_height: f64,
) -> Option<Rect> {
    let (size_x, size_y) = gui_tower_estimate(settings, object_height)?;
    let margin = gui_tower_margin_mm(slicer_kind);
    let range = Rect::new(
        plate.min_x + margin,
        plate.min_y + margin,
        plate.max_x - size_x - margin,
        plate.max_y - size_y - margin,
    );
    (range.min_x <= range.max_x && range.min_y <= range.max_y).then_some(range)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The P1S goat project of job b3970318 (BambuStudio 2.8.2.61): its GUI opened the tower
    /// stored at (6.236, 228.524) with the centre readout (23.58, 232.42) — min corner
    /// (15, 223.84) for the estimated 17.16 mm rib tower.
    fn goat_settings() -> serde_json::Value {
        serde_json::json!({
            "layer_height": "0.28",
            "prime_tower_infill_gap": "150%",
            "prime_tower_rib_wall": "1",
            "prime_tower_rib_width": "8",
            "prime_tower_extra_rib_length": "0",
            "prime_tower_width": "20",
            "filament_colour": ["#FFFFFF", "#000000", "#C12E1F"],
            "nozzle_diameter": ["0.4"],
            "filament_prime_volume": ["45", "45", "30"],
            "prime_volume_mode": "Saving",
            "timelapse_type": "0",
            "enable_wrapping_detection": "0",
            "wipe_tower_x": ["6.236"],
            "wipe_tower_y": ["228.524"],
        })
    }

    #[test]
    fn gui_estimate_matches_bambu_studio_readout() {
        let (x, y) = gui_tower_estimate(&goat_settings(), 44.74).unwrap();
        assert!(
            (x - 17.16).abs() < 0.01 && (y - 17.16).abs() < 0.01,
            "{x} {y}"
        );
        let range = gui_tower_anchor_range(
            &goat_settings(),
            "bambu",
            Rect::new(0.0, 0.0, 256.0, 256.0),
            44.74,
        )
        .unwrap();
        assert!((range.min_x - 15.0).abs() < 1e-9 && (range.min_y - 15.0).abs() < 1e-9);
        assert!((range.max_x - 223.84).abs() < 0.01 && (range.max_y - 223.84).abs() < 0.01);
        // The GUI centre readout: clamped min corner plus half the estimate.
        assert!((range.min_x + x / 2.0 - 23.58).abs() < 0.01);
        assert!((range.max_y + y / 2.0 - 232.42).abs() < 0.01);
    }

    #[test]
    fn single_filament_shows_no_gui_tower() {
        let mut settings = goat_settings();
        settings["filament_colour"] = serde_json::json!(["#FFFFFF"]);
        assert_eq!(gui_tower_estimate(&settings, 44.74), None);
    }

    #[test]
    fn orca_keeps_one_millimetre() {
        let range = gui_tower_anchor_range(
            &goat_settings(),
            "orca",
            Rect::new(0.0, 0.0, 256.0, 256.0),
            44.74,
        )
        .unwrap();
        assert!(
            (range.min_x - 1.0).abs() < 1e-9 && (range.max_x - (256.0 - 17.16 - 1.0)).abs() < 0.01
        );
    }
}

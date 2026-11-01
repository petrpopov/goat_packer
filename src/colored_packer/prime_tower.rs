//! Where the slicer put the prime (wipe) tower, read from its G-code.

use serde::{Deserialize, Serialize};

use crate::gcode_text::axis_value;
use crate::geometry::Rect;

/// The tower bounds are re-measured from the final slice and a repack is forced when they
/// grow, so the reserve just covers probe-vs-final measurement noise — 4 mm here used to
/// throw away grid cells whose patch cleared the actual tower by a wide margin.
const TOWER_KEEPOUT_SAFETY_MM: f64 = 1.0;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct PrimeTowerBounds {
    pub centerline: Rect,
    pub keepout: Rect,
    pub extrusion_width_mm: f64,
}

pub fn parse_prime_tower_bounds(
    gcode: &[u8],
    fallback_line_width_mm: f64,
    clearance_mm: f64,
) -> Option<PrimeTowerBounds> {
    let text = String::from_utf8_lossy(gcode);
    let mut scan = TowerScan::new(fallback_line_width_mm);
    for raw_line in text.lines() {
        scan.read_line(raw_line.trim());
    }
    let centerline = scan.bounds.filter(|bounds| bounds.min_x.is_finite())?;
    let expansion = scan.max_width * 0.5 + clearance_mm + TOWER_KEEPOUT_SAFETY_MM;
    Some(PrimeTowerBounds {
        centerline,
        keepout: centerline.grown(expansion),
        extrusion_width_mm: scan.max_width,
    })
}

/// The reading state: the head position under G90/G91, the extruder mode, and the extent of the
/// extruding moves inside the tower sections so far.
struct TowerScan {
    absolute_mode: bool,
    /// Extruder mode only from M82/M83 (not G90/G91): Bambu/Orca emit M83 once and keep
    /// issuing G90/G91 for XY moves, so tying E to G90 would misread every later move.
    absolute_e: bool,
    last_e: f64,
    x: f64,
    y: f64,
    /// Inside `WIPE_TOWER_START` .. `WIPE_TOWER_END`.
    active: bool,
    /// Inside `WIPE_START` .. `WIPE_END` (the nozzle wipe, which is not the tower).
    nozzle_wipe: bool,
    line_width: f64,
    max_width: f64,
    bounds: Option<Rect>,
}

impl TowerScan {
    fn new(fallback_line_width_mm: f64) -> Self {
        Self {
            absolute_mode: true,
            absolute_e: false,
            last_e: 0.0,
            x: 0.0,
            y: 0.0,
            active: false,
            nozzle_wipe: false,
            line_width: fallback_line_width_mm,
            max_width: fallback_line_width_mm,
            bounds: None,
        }
    }

    fn read_line(&mut self, line: &str) {
        if line.starts_with("G90") {
            self.absolute_mode = true;
        } else if line.starts_with("M82") {
            self.absolute_e = true;
        } else if line.starts_with("M83") {
            self.absolute_e = false;
        } else if line.starts_with("G92") {
            if let Some(value) = axis_value(line, 'E') {
                self.last_e = value;
            }
        } else if line.starts_with("G91") {
            self.absolute_mode = false;
        } else if let Some(value) = line.strip_prefix("; LINE_WIDTH:") {
            if let Ok(width) = value.trim().parse::<f64>() {
                self.line_width = width;
            }
        } else if line.contains("WIPE_TOWER_END") || line.contains("WIPE_TOWER_BRIM_END") {
            self.active = false;
        } else if line.contains("WIPE_TOWER_START") || line.contains("WIPE_TOWER_BRIM_START") {
            self.active = true;
        } else if let Some(wipe) = nozzle_wipe_marker(line) {
            self.nozzle_wipe = wipe;
        } else if line.starts_with("G0") || line.starts_with("G1") {
            self.read_move(line);
        }
    }

    fn read_move(&mut self, line: &str) {
        let extrudes = self.extrudes(line);
        let parsed_x = axis_value(line, 'X');
        let parsed_y = axis_value(line, 'Y');
        if self.absolute_mode {
            self.x = parsed_x.unwrap_or(self.x);
            self.y = parsed_y.unwrap_or(self.y);
        } else {
            self.x += parsed_x.unwrap_or(0.0);
            self.y += parsed_y.unwrap_or(0.0);
        }
        if self.active
            && !self.nozzle_wipe
            && (parsed_x.is_some() || parsed_y.is_some())
            && extrudes
        {
            let point = Rect::new(self.x, self.y, self.x, self.y);
            self.bounds = Some(self.bounds.map_or(point, |bounds| bounds.union(point)));
            self.max_width = self.max_width.max(self.line_width);
        }
    }

    /// Only a move that really lays down filament is tower. Bambu retract-wipes at the object
    /// just printed right after `WIPE_TOWER_START` (real A2L koza probe:
    /// `G1 X125.282 Y116.54 E-.4256` while the tower sits at x=170..205); counting that move
    /// inflated the keepout to ~82x88 mm over the object and caused "prime tower changed again
    /// after the permitted re-pack" on every archived multi-colour koza source. Relative E:
    /// E > 0; absolute E: E above the previous value.
    fn extrudes(&mut self, line: &str) -> bool {
        match axis_value(line, 'E') {
            Some(e) if self.absolute_e => {
                let rising = e > self.last_e;
                self.last_e = e;
                rising
            }
            Some(e) => e > 0.0,
            None => false,
        }
    }
}

/// OrcaSlicer's IDEX tool change (Snapmaker U1) empties and wipes the nozzle at a garbage point
/// far away from the tower between `;WIPE_START` and `;WIPE_END`, still inside the WIPE_TOWER
/// section. Those moves are not the tower: counting them made the "tower bounds" half the plate
/// wide on a real project. BambuStudio spells the same marker `; WIPE_START` (with a space) and
/// retract-wipes at the object it just printed, right after `; WIPE_TOWER_START`: on an A2L
/// koza plate that stretched the tower over the objects and made a relocated tower look
/// ignored. `Some(true)` starts a nozzle wipe, `Some(false)` ends it.
fn nozzle_wipe_marker(line: &str) -> Option<bool> {
    match line.trim_start_matches(';').trim() {
        "WIPE_START" => Some(true),
        "WIPE_END" => Some(false),
        _ => None,
    }
}

pub fn tower_repack_required(reserved_keepout: Rect, final_tower: PrimeTowerBounds) -> bool {
    final_tower.keepout.min_x < reserved_keepout.min_x
        || final_tower.keepout.min_y < reserved_keepout.min_y
        || final_tower.keepout.max_x > reserved_keepout.max_x
        || final_tower.keepout.max_y > reserved_keepout.max_y
}

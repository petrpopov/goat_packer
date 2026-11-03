//! The printed tower (spec 04 §4.5-§4.8, design §4.2): the column outline, the L0 brim reach,
//! the chamfer rings of layers ≥ 1, and the plate zones they make at an anchor: the column on
//! every tower layer and each chamfer ring on its own layer are tower body; only the L0 brim
//! ring is the tower brim that supports may overlap (lead decision, design §14 Q18).

use std::ops::RangeInclusive;

use super::anchor::{TowerExtent, anchor_range};
use super::bambu::{Column, bambu_column};
use super::formulas::{auto_brim_width, brim_loops};
use super::outline::{grow_mitred, rectangle, rib_outline};
use super::schedule::ToolSchedule;
use super::settings::TowerSettings;
use super::type2::type2_column;
use crate::first_layer::{ProjectSettings, SlicerBuild, Unsupported, bead_spacing, subtract_loops};
use crate::gcode_collisions::LAYER_BAND_MM;
use crate::geometry::{Point, Rect};
use crate::object_print::{Loops, PrintKind};

/// Material past the wall and loop centre lines: half the tower line plus 0.1 mm (lead
/// decision, 04 §9 item 2). The 0.1 mm covers every measured residual: Bambu rib towers reach
/// up to 0.052 mm past the anchor corner where a skip-point gap removed the layer-0 extreme
/// vertex, and stay within 0.093 mm elsewhere (04 §8.3).
pub const TOWER_EDGE_TOLERANCE_MM: f64 = 0.1;
/// BambuStudio's chamfer is always on and at most this wide (04 §4.6).
const BAMBU_CHAMFER_MAX_MM: f64 = 3.0;

/// Where the plate puts the tower.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TowerPlate {
    /// The printable area (bed-shape bbox) in plate coordinates.
    pub printable: Rect,
    /// Height of the tallest object: the GUI estimates size the tower by it.
    pub object_height: f64,
}

/// One chamfer ring: on tower layer `(z_bottom, z_top]`, loops reaching `reach_mm` past the
/// wall centre line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChamferRing {
    pub z_bottom: f64,
    pub z_top: f64,
    pub reach_mm: f64,
}

/// A plate zone of the tower: a kind, the layer interval it prints in and its material
/// outline (even-odd loops, plate coordinates).
#[derive(Clone, Debug, PartialEq)]
pub struct TowerZone {
    pub kind: PrintKind,
    /// `(z_bottom, z_top]` of the layers the zone prints on.
    pub z_range: (f64, f64),
    pub loops: Loops,
}

/// The prime tower one build prints for a project.
#[derive(Clone, Debug, PartialEq)]
pub struct TowerPrint {
    pub build: SlicerBuild,
    /// Wall centre line of the first layer, closed and counter-clockwise, relative to the
    /// anchor (the anchor is its bbox minimum: B places the layer-0 wall bbox there, Type 2
    /// the box corner).
    pub outline: Vec<Point>,
    /// W × D of the wall centre line's bbox.
    pub body_size: (f64, f64),
    /// W × D of the box the purge rows fill (equals `body_size` for rectangular walls).
    pub box_size: (f64, f64),
    pub line_width: f64,
    /// z of the tower top H_t.
    pub height: f64,
    pub first_layer_height: f64,
    /// Reach of the outermost L0 brim loop's centre line past the wall centre line.
    pub l0_brim_mm: f64,
    pub chamfer: Vec<ChamferRing>,
    /// The stored `wipe_tower_x/y` of plate 1.
    pub source_anchor: (f64, f64),
    /// Range of anchors the build keeps where they are (`None`: empty range).
    pub anchor_range: Option<Rect>,
    pub max_changes: usize,
}

/// The tower `build` prints for a project with `schedule` (`None` without a tower: disabled,
/// or no tool change).
pub fn printed_tower(
    settings: &ProjectSettings,
    schedule: &ToolSchedule,
    build: SlicerBuild,
    plate: &TowerPlate,
) -> Result<Option<TowerPrint>, Unsupported> {
    let Some(tower) = TowerSettings::read(settings, build)? else {
        return Ok(None);
    };
    let Some(height) = schedule.tower_top() else {
        return Ok(None);
    };
    let column = match build {
        SlicerBuild::BambuStudio => bambu_column(schedule, &tower, height),
        _ => type2_column(schedule, &tower, build),
    };
    let outline = column_outline(&column, &tower);
    let bounds = Rect::of_points(outline.iter());
    let brim = brim_width(&tower, build, height);
    let first_spacing = bead_spacing(tower.line_width, tower.first_layer_height);
    let l0_brim_mm = brim_loops(brim, first_spacing) as f64 * first_spacing;
    let extent = TowerExtent {
        width: bounds.max_x - bounds.min_x,
        depth: bounds.max_y - bounds.min_y,
        brim_reach: l0_brim_mm,
    };
    let filaments = schedule.used.len();
    Ok(Some(TowerPrint {
        build,
        body_size: (extent.width, extent.depth),
        box_size: column.box_size,
        line_width: tower.line_width,
        height,
        first_layer_height: tower.first_layer_height,
        l0_brim_mm,
        chamfer: chamfer_rings(schedule, &tower, build, brim),
        source_anchor: tower.anchor,
        anchor_range: anchor_range(
            build,
            settings,
            extent,
            plate.printable,
            plate.object_height,
            filaments,
        ),
        max_changes: schedule.max_changes(),
        outline,
    }))
}

fn column_outline(column: &Column, tower: &TowerSettings) -> Vec<Point> {
    let (width, depth) = column.box_size;
    match column.rib {
        Some(rib) => rib_outline(width, depth, rib, tower.fillet_wall),
        None => rectangle(width, depth),
    }
}

/// The brim width the tower is printed with: B, O and R size an auto brim (−1) by the tower
/// height; Snapmaker Orca has no auto value (its minimum is 0).
fn brim_width(tower: &TowerSettings, build: SlicerBuild, height: f64) -> f64 {
    match (build, tower.brim_width < 0.0) {
        (SlicerBuild::SnapmakerOrca, true) => 0.0,
        (_, true) => auto_brim_width(height),
        _ => tower.brim_width,
    }
}

/// Chamfer rings on tower layers j ≥ 1 (04 §4.6): j fewer loops than fit both the brim and
/// the chamfer limit, until none are left. B always (3 mm), S with its option, O/R never.
/// The slicer also stops once a layer is shallower than the first; that is ignored, which
/// only keeps rings the slicer may drop (the safe side).
fn chamfer_rings(
    schedule: &ToolSchedule,
    tower: &TowerSettings,
    build: SlicerBuild,
    brim: f64,
) -> Vec<ChamferRing> {
    let limit = match build {
        SlicerBuild::BambuStudio => BAMBU_CHAMFER_MAX_MM,
        SlicerBuild::SnapmakerOrca if tower.chamfer => tower.chamfer_max_width,
        _ => return Vec::new(),
    };
    let mut rings = Vec::new();
    for (index, layer) in schedule.tower_layers().iter().enumerate().skip(1) {
        let spacing = bead_spacing(tower.line_width, layer.z_top - layer.z_bottom);
        let fit = brim_loops(brim, spacing).min((limit / spacing).floor() as usize);
        let Some(loops) = fit.checked_sub(index).filter(|loops| *loops > 0) else {
            break;
        };
        rings.push(ChamferRing {
            z_bottom: layer.z_bottom,
            z_top: layer.z_top,
            reach_mm: loops as f64 * spacing,
        });
    }
    rings
}

impl TowerZone {
    /// The absolute 2 mm bands the zone meets under the band rule of design §4.1: every band
    /// its layer interval `(z_bottom, z_top]` overlaps with positive length, so a chamfer ring
    /// on (1.88, 2.16] is in bands 0 and 1 and one ending exactly at 2.0 only in band 0.
    pub fn bands(&self) -> RangeInclusive<usize> {
        let (from, to) = self.z_range;
        let first = libm::floor(from / LAYER_BAND_MM) as usize;
        let last = (libm::ceil(to / LAYER_BAND_MM) as usize).saturating_sub(1);
        first..=last.max(first)
    }
}

impl TowerPrint {
    /// Material grown past a centre line reaching `reach` beyond the wall.
    fn material(&self, anchor: (f64, f64), reach: f64) -> Loops {
        let grown = grow_mitred(
            &self.outline,
            reach + self.line_width / 2.0 + TOWER_EDGE_TOLERANCE_MM,
        );
        grown
            .into_iter()
            .map(|ring| {
                ring.iter()
                    .map(|p| Point {
                        x: p.x + anchor.0,
                        y: p.y + anchor.1,
                    })
                    .collect()
            })
            .collect()
    }

    /// The tower body at `anchor`: the column on every tower layer, then every chamfer ring
    /// (with the column inside it) on its own layer.
    pub fn body_zones(&self, anchor: (f64, f64)) -> Vec<TowerZone> {
        let mut zones = vec![TowerZone {
            kind: PrintKind::TowerBody,
            z_range: (0.0, self.height),
            loops: self.material(anchor, 0.0),
        }];
        zones.extend(self.chamfer.iter().map(|ring| TowerZone {
            kind: PrintKind::TowerBody,
            z_range: (ring.z_bottom, ring.z_top),
            loops: self.material(anchor, ring.reach_mm),
        }));
        zones
    }

    /// The L0 brim ring at `anchor` (without the column), `None` without a brim.
    pub fn brim_zone(&self, anchor: (f64, f64)) -> Option<TowerZone> {
        if self.l0_brim_mm <= 0.0 {
            return None;
        }
        let outer = self.material(anchor, self.l0_brim_mm);
        let column = self.material(anchor, 0.0);
        let ring = subtract_loops(&outer, &column);
        Some(TowerZone {
            kind: PrintKind::TowerBrim,
            z_range: (0.0, self.first_layer_height),
            loops: ring,
        })
    }

    /// Every zone at `anchor`: body pieces first, then the brim.
    pub fn zones(&self, anchor: (f64, f64)) -> Vec<TowerZone> {
        let mut zones = self.body_zones(anchor);
        zones.extend(self.brim_zone(anchor));
        zones
    }

    /// Bbox of all tower material relative to the anchor (the L0 brim or the widest ring).
    pub fn material_bounds(&self) -> Rect {
        let reach = self
            .chamfer
            .iter()
            .map(|ring| ring.reach_mm)
            .fold(self.l0_brim_mm, f64::max);
        Rect::of_points(self.material((0.0, 0.0), reach).iter().flatten())
    }
}

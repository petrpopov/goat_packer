//! The prime tower as plate zones whose position the search chooses (design 2026-11-02, §6,
//! §9.3). `TowerSpec` is plain data (work package I3 derives it from the project settings);
//! at an anchor it becomes `TowerBody` zones (the column and the chamfer rings of layers >= 1,
//! each with its z range) and the `TowerBrim` zone (the layer-0 brim only), and what they
//! forbid comes from `keepout::permission` like every other zone.

use crate::geometry::{Point, Rect, polygon_from_loop};
use crate::keepout::{ForeignGaps, Permission, ZoneClass, permission};
use crate::kind_masks::{PlateZone, ZoneArea};
use crate::object_print::PrintKind;
use geo::Intersects;
use serde::{Deserialize, Serialize};

/// Float slack of the box tests, mm: equality passes.
const FIT_EPS_MM: f64 = 1e-9;

/// The printed prime tower, relative to its anchor. The anchor is the point the 3MF's
/// `wipe_tower_x/y` names (the body's min corner as the slicer counts it).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TowerSpec {
    /// The tower body (the column above the chamfer) relative to the anchor, mm: material
    /// extent with the line width, so it may start a little before the anchor (measured
    /// -0.25 mm on every goat tower).
    pub body: Rect,
    /// Layer-0 brim beyond the body on every side, mm (0 = none).
    pub brim_l0_mm: f64,
    /// Chamfer rings above layer 0 as `[print z, width beyond the body]`, mm.
    #[serde(default)]
    pub chamfer: Vec<[f64; 2]>,
    /// The target GUI's path-conflict check compares the layer-0 brim with an object's support
    /// lines (`Flavour::conflict_tower_lines`): supports must keep out of the brim ring.
    /// False = they may overlap it.
    #[serde(default)]
    pub brim_conflicts: bool,
    /// Top of the tower, mm: bodies above it may pass over the tower's footprint.
    pub top_z_mm: f64,
    /// Where the anchor may go (inclusive): the GUI re-clamps a stored anchor into this range
    /// when it opens the 3MF.
    pub anchor_range: Rect,
    /// The project's own anchor.
    pub source_anchor: Point,
}

/// One printed part of the tower: its kind, rectangle (relative to the anchor) and z range.
struct TowerPart {
    id: String,
    kind: PrintKind,
    rect: Rect,
    z_range_mm: [f64; 2],
}

impl TowerSpec {
    /// Finite numbers, non-empty rectangles, non-negative widths.
    pub fn check(&self) -> Result<(), String> {
        let rect_ok = |r: &Rect| {
            [r.min_x, r.min_y, r.max_x, r.max_y]
                .iter()
                .all(|v| v.is_finite())
                && r.min_x <= r.max_x
                && r.min_y <= r.max_y
        };
        let width_ok = |w: f64| w.is_finite() && w >= 0.0;
        let ok = rect_ok(&self.body)
            && rect_ok(&self.anchor_range)
            && width_ok(self.brim_l0_mm)
            && width_ok(self.top_z_mm)
            && self
                .chamfer
                .iter()
                .all(|&[z, w]| width_ok(z) && width_ok(w))
            && self.source_anchor.x.is_finite()
            && self.source_anchor.y.is_finite();
        if ok {
            Ok(())
        } else {
            Err("invalid tower spec".to_owned())
        }
    }

    /// The body at every height up to the top, the layer-0 brim and each chamfer ring at its
    /// own height. A brim or ring is given as the whole grown rectangle: the body inside it is
    /// forbidden to every kind anyway.
    fn parts(&self) -> Vec<TowerPart> {
        let mut parts = vec![TowerPart {
            id: "tower_body".to_owned(),
            kind: PrintKind::TowerBody,
            rect: self.body,
            z_range_mm: [0.0, self.top_z_mm],
        }];
        if self.brim_l0_mm > 0.0 {
            parts.push(TowerPart {
                id: "tower_brim".to_owned(),
                kind: PrintKind::TowerBrim,
                rect: self.body.grown(self.brim_l0_mm),
                z_range_mm: [0.0, 0.0],
            });
        }
        for (index, &[z, width]) in self.chamfer.iter().enumerate() {
            parts.push(TowerPart {
                id: format!("tower_chamfer#{index}"),
                // Tower BODY (design 4.2, Q18): only the layer-0 brim may be overlapped by
                // supports; a ring on a later layer is printed material they would hit.
                kind: PrintKind::TowerBody,
                rect: self.body.grown(width),
                z_range_mm: [z, z],
            });
        }
        parts
    }

    /// The tower's zones with its anchor at `anchor`, plate frame.
    pub fn zones(&self, anchor: Point) -> Vec<PlateZone> {
        let brim_class = if self.brim_conflicts {
            ZoneClass::TowerBrimChecked
        } else {
            ZoneClass::TowerBrim
        };
        self.parts()
            .into_iter()
            .map(|part| PlateZone {
                id: part.id,
                class: match part.kind {
                    PrintKind::TowerBody => ZoneClass::TowerBody,
                    _ => brim_class,
                },
                area: ZoneArea::Rect(part.rect.translated(anchor.x, anchor.y)),
                z_range_mm: Some(part.z_range_mm),
            })
            .collect()
    }

    /// The first rule of `plate` the tower breaks at `anchor` (the tower rows of the matrix:
    /// inside the printable area, off hard zones, the body's layer 0 off soft zones).
    pub fn conflict(&self, anchor: Point, plate: &[PlateZone]) -> Option<String> {
        for part in self.parts() {
            let rect = part.rect.translated(anchor.x, anchor.y);
            for zone in plate
                .iter()
                .filter(|z| z_overlap(part.z_range_mm, z.z_range_mm))
            {
                let Permission::Forbidden { gap } = permission(part.kind, zone.class) else {
                    continue;
                };
                // The tower rows against plate zones only use gap 0 (touching passes).
                let rect = rect.grown(gap.mm(0.0, ForeignGaps::default()));
                if breaks(rect, &zone.area) {
                    return Some(format!(
                        "{} at ({:.2}, {:.2}) breaks the {:?} zone '{}'",
                        part.id, anchor.x, anchor.y, zone.class, zone.id
                    ));
                }
            }
        }
        None
    }

    /// Anchors where every tower part stays inside every `Outside` zone that forbids it (the
    /// printable area, hard edge bands), within `anchor_range`; `None` when there are none.
    pub(crate) fn fit_range(&self, plate: &[PlateZone]) -> Option<Rect> {
        let mut range = self.anchor_range;
        for part in self.parts() {
            for zone in plate
                .iter()
                .filter(|z| z_overlap(part.z_range_mm, z.z_range_mm))
            {
                let ZoneArea::Outside {
                    inside,
                    material_inset_mm,
                    ..
                } = zone.area
                else {
                    continue;
                };
                if !matches!(
                    permission(part.kind, zone.class),
                    Permission::Forbidden { .. }
                ) {
                    continue;
                }
                let inside = inside.grown(-material_inset_mm);
                range.min_x = range.min_x.max(inside.min_x - part.rect.min_x);
                range.min_y = range.min_y.max(inside.min_y - part.rect.min_y);
                range.max_x = range.max_x.min(inside.max_x - part.rect.max_x);
                range.max_y = range.max_y.min(inside.max_y - part.rect.max_y);
            }
        }
        (range.min_x <= range.max_x && range.min_y <= range.max_y).then_some(range)
    }
}

/// The anchors this package tries, in tie order: the source anchor clamped into the range
/// (where the GUI would put it), then the four corners of the range where the tower fits the
/// printable area. Anchors that break a plate zone are dropped. Work package P4's
/// `tower_scan` replaces this short list with a scan of the whole range.
pub fn anchor_candidates(spec: &TowerSpec, plate: &[PlateZone]) -> Result<Vec<Point>, String> {
    spec.check()?;
    let range = spec.anchor_range;
    let source = Point {
        x: spec.source_anchor.x.clamp(range.min_x, range.max_x),
        y: spec.source_anchor.y.clamp(range.min_y, range.max_y),
    };
    let mut anchors = vec![source];
    if let Some(fit) = spec.fit_range(plate) {
        for (x, y) in [
            (fit.min_x, fit.min_y),
            (fit.max_x, fit.min_y),
            (fit.min_x, fit.max_y),
            (fit.max_x, fit.max_y),
        ] {
            let corner = Point { x, y };
            if !anchors.contains(&corner) {
                anchors.push(corner);
            }
        }
    }
    let conflicts: Vec<String> = anchors
        .iter()
        .filter_map(|&a| spec.conflict(a, plate))
        .collect();
    anchors.retain(|&a| spec.conflict(a, plate).is_none());
    if anchors.is_empty() {
        return Err(format!(
            "no tower anchor fits the plate: {}",
            conflicts.join("; ")
        ));
    }
    Ok(anchors)
}

/// Z ranges meet (a zone without a range is at every height).
fn z_overlap([from, to]: [f64; 2], zone: Option<[f64; 2]>) -> bool {
    zone.is_none_or(|[zone_from, zone_to]| from <= zone_to && zone_from <= to)
}

/// A tower rectangle breaks a forbidding zone: it leaves an `Outside` zone's inside, or meets
/// a rectangle or polygon zone (overlap with area; for polygons touching counts, which only
/// errs on the safe side).
fn breaks(rect: Rect, area: &ZoneArea) -> bool {
    match area {
        ZoneArea::Outside {
            inside,
            material_inset_mm,
            ..
        } => rect.sticks_out_of(inside.grown(-material_inset_mm), FIT_EPS_MM),
        ZoneArea::Rect(zone) => {
            rect.min_x < zone.max_x - FIT_EPS_MM
                && rect.max_x > zone.min_x + FIT_EPS_MM
                && rect.min_y < zone.max_y - FIT_EPS_MM
                && rect.max_y > zone.min_y + FIT_EPS_MM
        }
        ZoneArea::Loops(loops) => {
            let own = polygon_from_loop(&rect_ring(rect));
            loops
                .iter()
                .any(|ring| polygon_from_loop(ring).intersects(&own))
        }
    }
}

fn rect_ring(r: Rect) -> Vec<Point> {
    [
        (r.min_x, r.min_y),
        (r.max_x, r.min_y),
        (r.max_x, r.max_y),
        (r.min_x, r.max_y),
        (r.min_x, r.min_y),
    ]
    .iter()
    .map(|&(x, y)| Point { x, y })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keepout::Gap;

    fn spec(brim_conflicts: bool) -> TowerSpec {
        TowerSpec {
            body: Rect::new(0.0, 0.0, 10.0, 8.0),
            brim_l0_mm: 2.0,
            chamfer: vec![[0.4, 1.5]],
            brim_conflicts,
            top_z_mm: 4.0,
            anchor_range: Rect::new(0.0, 0.0, 50.0, 50.0),
            source_anchor: Point { x: 20.0, y: 20.0 },
        }
    }

    fn class_of(zones: &[PlateZone], id: &str) -> ZoneClass {
        zones.iter().find(|zone| zone.id == id).unwrap().class
    }

    #[test]
    fn chamfer_rings_are_tower_body_and_only_the_first_layer_brim_is_a_brim() {
        let zones = spec(false).zones(Point { x: 0.0, y: 0.0 });
        assert_eq!(class_of(&zones, "tower_chamfer#0"), ZoneClass::TowerBody);
        assert_eq!(class_of(&zones, "tower_brim"), ZoneClass::TowerBrim);
        // A support keeps out of a ring (printed material) but may overlap the brim.
        let support = PrintKind::SupportBase;
        assert_eq!(
            permission(support, ZoneClass::TowerBody),
            Permission::Forbidden { gap: Gap::Support }
        );
        assert_eq!(permission(support, ZoneClass::TowerBrim), Permission::Allowed);
    }

    #[test]
    fn a_brim_the_gui_compares_keeps_supports_out_without_a_gap() {
        let zones = spec(true).zones(Point { x: 0.0, y: 0.0 });
        assert_eq!(class_of(&zones, "tower_brim"), ZoneClass::TowerBrimChecked);
        assert_eq!(class_of(&zones, "tower_chamfer#0"), ZoneClass::TowerBody);
        for kind in [
            PrintKind::SupportBase,
            PrintKind::SupportBand,
            PrintKind::InterfaceBand,
        ] {
            assert_eq!(
                permission(kind, ZoneClass::TowerBrimChecked),
                Permission::Forbidden { gap: Gap::Zero }
            );
        }
        // Object brims are trimmed by the slicer and never read by its conflict check.
        assert_eq!(
            permission(PrintKind::Brim, ZoneClass::TowerBrimChecked),
            Permission::Allowed
        );
    }

    #[test]
    fn an_old_state_without_the_flag_keeps_supports_over_the_brim() {
        let mut json = serde_json::to_value(spec(true)).unwrap();
        json.as_object_mut().unwrap().remove("brim_conflicts");
        let spec: TowerSpec = serde_json::from_value(json).unwrap();
        assert!(!spec.brim_conflicts);
    }
}

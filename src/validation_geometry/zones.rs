//! The permission matrix against the plate zones, exactly: part hulls must not overlap a hard
//! zone (touching passes), part boxes keep the restricted band and the printable area
//! (equality passes), material keeps its gap from forbidden zones (strips, the tower).

use super::GAP_EPS;
use super::class::{ClassShapes, rect_ring};
use super::region::{Region, moved_box, negated, overlaps};
use crate::geometry::{Point, Rect};
use crate::keepout::{ForeignGaps, Permission, ZoneClass, permission};
use crate::kind_masks::{PlateZone, ZoneArea, zone_material};
use crate::object_print::PrintKind;

/// The kinds of an object (the tower kinds belong to the plate).
const OBJECT_KINDS: [PrintKind; 7] = [
    PrintKind::Silhouette,
    PrintKind::BodyBand,
    PrintKind::Landing,
    PrintKind::Brim,
    PrintKind::SupportBase,
    PrintKind::SupportBand,
    PrintKind::InterfaceBand,
];

/// A plate zone with its area as a region (plate frame), built once per check.
pub(super) struct ZoneShape {
    pub(super) zone: PlateZone,
    /// `None` for `Outside` zones, which are box tests.
    area: Option<Region>,
}

impl ZoneShape {
    pub(super) fn new(zone: &PlateZone) -> ZoneShape {
        let area = match &zone.area {
            ZoneArea::Outside { .. } => None,
            ZoneArea::Rect(rect) => Some(Region::rect(*rect)),
            ZoneArea::Loops(loops) => Some(Region::new(loops)),
        };
        ZoneShape {
            zone: zone.clone(),
            area,
        }
    }
}

/// Whether a copy of `class` at translation `at` breaks a row of the matrix against `zone`.
pub(super) fn breaks_zone(
    class: &ClassShapes,
    at: Point,
    zone: &ZoneShape,
    clearance: f64,
    foreign: ForeignGaps,
) -> bool {
    let (mut boxes, mut hulls) = (false, false);
    let mut materials = Vec::new();
    for kind in OBJECT_KINDS {
        match permission(kind, zone.zone.class) {
            Permission::Allowed => {}
            Permission::PartBox => boxes = true,
            Permission::PartHull => hulls = true,
            Permission::Forbidden { gap } => materials.push((kind, gap.mm(clearance, foreign))),
        }
    }
    extent_breaks(class, at, zone)
        || (boxes && parts_break(class, at, zone, false))
        || (hulls && parts_break(class, at, zone, true))
        || materials
            .iter()
            .any(|&(kind, gap)| material_breaks(class, at, kind, zone, gap))
}

/// A legacy footprint's whole printed extent leaves the printable area.
fn extent_breaks(class: &ClassShapes, at: Point, zone: &ZoneShape) -> bool {
    let ZoneArea::Outside { inside, .. } = zone.zone.area else {
        return false;
    };
    zone.zone.class == ZoneClass::Boundary
        && class
            .extent
            .is_some_and(|e| moved_box(e, at).sticks_out_of(inside, GAP_EPS))
}

/// Part boxes (or hulls) against the zone. Against an `Outside` zone a hull fits exactly
/// when its box does; a box against a rectangle is a range test with equality passing.
fn parts_break(class: &ClassShapes, at: Point, zone: &ZoneShape, hulls: bool) -> bool {
    let boxes = || class.boxes.iter().map(move |b| moved_box(*b, at));
    match (&zone.zone.area, &zone.area) {
        (ZoneArea::Outside { inside, .. }, _) => boxes().any(|b| b.sticks_out_of(*inside, GAP_EPS)),
        (ZoneArea::Rect(rect), _) if !hulls => boxes().any(|b| boxes_overlap(b, *rect)),
        (_, Some(area)) => {
            // The zone is in the plate frame: move it into the print frame.
            let shift = negated(at);
            if hulls && !class.hulls.is_empty() {
                class.hulls.iter().any(|hull| overlaps(hull, area, shift))
            } else {
                let own = |b: &Rect| Region::new(&[rect_ring(*b)]);
                class.boxes.iter().any(|b| overlaps(&own(b), area, shift))
            }
        }
        (_, None) => unreachable!("only outside zones have no area"),
    }
}

/// The kind's material that can meet the zone keeps `gap` from it.
fn material_breaks(
    class: &ClassShapes,
    at: Point,
    kind: PrintKind,
    zone: &ZoneShape,
    gap: f64,
) -> bool {
    let regions = zone_material(&class.print, kind, &zone.zone)
        .into_iter()
        .filter_map(|(band, _)| class.region(kind, band));
    match (&zone.zone.area, &zone.area) {
        (
            ZoneArea::Outside {
                inside,
                material_inset_mm,
            },
            _,
        ) => {
            let allowed = inside.grown(-material_inset_mm);
            regions
                .map(|r| moved_box(r.bounds(), at).grown(gap))
                .any(|b| b.sticks_out_of(allowed, GAP_EPS))
        }
        (_, Some(area)) => regions
            .into_iter()
            .any(|r| super::breaks_gap(r, area, negated(at), gap)),
        (_, None) => unreachable!("only outside zones have no area"),
    }
}

/// Overlap with positive area beyond float noise; touching passes.
fn boxes_overlap(b: Rect, rect: Rect) -> bool {
    b.min_x < rect.max_x - GAP_EPS
        && b.max_x > rect.min_x + GAP_EPS
        && b.min_y < rect.max_y - GAP_EPS
        && b.max_y > rect.min_y + GAP_EPS
}

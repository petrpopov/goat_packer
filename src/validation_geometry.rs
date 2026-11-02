//! Vector-space publication check, independent of rasterization and no-fit maps: the exact
//! gaps of `keepout::pair_rules` between placed kinds, and the exact class rules of
//! `keepout::permission` against the plate zones (hull ∩ zone, part box distances, material
//! against strips). This validates the supplied envelope, not supports a future slicer
//! might generate.

use geo::{Area, BooleanOps, BoundingRect, Distance, Euclidean, MultiPolygon};

use crate::footprint::Footprint;
use crate::geometry::{Point, Rect, polygon_from_loop};
use crate::keepout::{Permission, ZoneClass, pair_rules, permission};
use crate::kind_masks::{PlateZone, ZoneArea, join_sag_mm, plate_zones, zone_material};
use crate::object_print::{Loops, ObjectPrint, PrintKind};
use crate::options::{PackOptions, PackResult, PackedPlacement};

const GAP_EPS: f64 = 1e-7;
const AREA_EPS: f64 = 1e-9;

/// Material of one kind (or one zone), placed on the plate.
struct Zone {
    polygon: MultiPolygon,
    bounds: Option<geo::Rect>,
}

impl Zone {
    fn new(loops: &[Vec<Point>]) -> Self {
        // Kinds use even-odd rings, not necessarily one exterior followed by holes.
        // XOR preserves holes, islands and winding without sharing raster fill logic.
        let mut polygon = MultiPolygon::new(vec![]);
        for ring in loops {
            polygon = polygon.xor(&MultiPolygon::new(vec![polygon_from_loop(ring)]));
        }
        let bounds = polygon.bounding_rect();
        Self { polygon, bounds }
    }

    fn conflicts(&self, other: &Self, gap: f64) -> bool {
        let (Some(a), Some(b)) = (self.bounds, other.bounds) else {
            return false;
        };
        // Almost every pair is far apart, including most height-band pairs.
        if a.max().x + gap < b.min().x
            || b.max().x + gap < a.min().x
            || a.max().y + gap < b.min().y
            || b.max().y + gap < a.min().y
        {
            return false;
        }
        // The masks draw round joins as inscribed chords, so the search may place two
        // kinds up to the chord sag closer than the gap (`join_sag_mm`, under a micron).
        let tolerance = GAP_EPS + join_sag_mm(gap);
        if gap > tolerance && Euclidean.distance(&self.polygon, &other.polygon) < gap - tolerance {
            return true;
        }
        self.polygon.intersection(&other.polygon).unsigned_area() > AREA_EPS
    }
}

/// One placement: its print on the plate and every kind as a zone.
struct Placed {
    print: ObjectPrint,
    kinds: Vec<(PrintKind, usize, Zone)>,
    hulls: Vec<Zone>,
    /// The legacy bed extent (everything the footprint prints), when the legacy bed rule holds.
    extent: Option<Rect>,
}

impl Placed {
    fn new(print: &ObjectPrint, extent: &Loops, p: &PackedPlacement) -> Self {
        let placed = place_print(print, p);
        let mut kinds = vec![
            (PrintKind::Silhouette, 0, Zone::new(&placed.silhouette)),
            (PrintKind::Landing, 0, Zone::new(&placed.landing)),
            (PrintKind::Brim, 0, Zone::new(&placed.brim)),
            (PrintKind::SupportBase, 0, Zone::new(&placed.support_base)),
        ];
        for (kind, bands) in [
            (PrintKind::BodyBand, &placed.body),
            (PrintKind::SupportBand, &placed.support),
            (PrintKind::InterfaceBand, &placed.interface),
        ] {
            kinds.extend(
                bands
                    .iter()
                    .enumerate()
                    .map(|(k, loops)| (kind, k, Zone::new(loops))),
            );
        }
        let hulls = placed
            .part_hulls
            .iter()
            .map(|hull| Zone::new(std::slice::from_ref(hull)))
            .collect();
        let extent =
            (!extent.is_empty()).then(|| Rect::of_points(p.place_loops(extent).iter().flatten()));
        Placed {
            print: placed,
            kinds,
            hulls,
            extent,
        }
    }

    fn kind(&self, kind: PrintKind, band: usize) -> Option<&Zone> {
        self.kinds
            .iter()
            .find(|(k, b, _)| *k == kind && *b == band)
            .map(|(.., zone)| zone)
    }
}

pub(crate) fn validate_geometry(
    fp: &Footprint,
    options: &PackOptions,
    result: &PackResult,
) -> Result<(), String> {
    let rules = options.effective_plate_rules();
    let zones = plate_zones(&rules, None)?;
    // Original coordinates, not the rounded canonical frame used by the raster search.
    let print = ObjectPrint::from_legacy(fp);
    let extent = if options.plate_rules.is_none() {
        legacy_extent(fp)
    } else {
        Loops::new()
    };
    let bands = print.band_count();
    let mut placed: Vec<Placed> = Vec::with_capacity(result.placements.len());
    for (index, p) in result.placements.iter().enumerate() {
        let current = Placed::new(&print, &extent, p);
        if let Some(zone) = zones
            .iter()
            .find(|zone| breaks_zone(&current, zone, options.clearance))
        {
            return Err(format!(
                "placement {index} breaks the {:?} zone '{}' (vector check)",
                zone.class, zone.id
            ));
        }
        for (previous, other) in placed.iter().enumerate() {
            if pairs_conflict(&current, other, rules.body_rule, bands, options.clearance) {
                return Err(format!(
                    "placements {previous} and {index} overlap or violate clearance (vector check)"
                ));
            }
        }
        placed.push(current);
    }
    Ok(())
}

/// Every loop of the legacy footprint: the legacy bed rule keeps all of it on the bed.
fn legacy_extent(fp: &Footprint) -> Loops {
    let zones = fp
        .layer_zones
        .iter()
        .flat_map(|z| z.support_loops.iter().chain(&z.body_loops));
    fp.loops
        .iter()
        .chain(&fp.body_loops)
        .chain(&fp.support_landing_loops)
        .chain(fp.bed_loops.iter().flatten())
        .chain(zones)
        .cloned()
        .collect()
}

/// A pair rule of `keepout::pair_rules` broken between two placements, both ways.
fn pairs_conflict(
    a: &Placed,
    b: &Placed,
    body_rule: crate::keepout::BodyRule,
    bands: usize,
    clearance: f64,
) -> bool {
    pair_rules(body_rule).iter().any(|rule| {
        let banded = [rule.a, rule.b].iter().any(|kind| is_band_kind(*kind));
        let gap = rule.gap.mm(clearance);
        (0..if banded { bands } else { 1 }).any(|k| {
            let pair = |x: &Placed, y: &Placed| match (x.kind(rule.a, k), y.kind(rule.b, k)) {
                (Some(za), Some(zb)) => za.conflicts(zb, gap),
                _ => false,
            };
            pair(a, b) || pair(b, a)
        })
    })
}

fn is_band_kind(kind: PrintKind) -> bool {
    matches!(
        kind,
        PrintKind::BodyBand | PrintKind::SupportBand | PrintKind::InterfaceBand
    )
}

/// Whether the placement breaks a row of the permission matrix against `zone`.
fn breaks_zone(placed: &Placed, zone: &PlateZone, clearance: f64) -> bool {
    let kinds = [
        PrintKind::Silhouette,
        PrintKind::BodyBand,
        PrintKind::Landing,
        PrintKind::Brim,
        PrintKind::SupportBase,
        PrintKind::SupportBand,
        PrintKind::InterfaceBand,
    ];
    let extent_out = zone.class == ZoneClass::Boundary
        && matches!(zone.area, ZoneArea::Outside { inside, .. }
            if placed.extent.is_some_and(|e| e.sticks_out_of(inside, GAP_EPS)));
    extent_out
        || kinds
            .iter()
            .any(|&kind| match permission(kind, zone.class) {
                Permission::Allowed => false,
                Permission::PartBox => parts_break(placed, zone, false),
                Permission::PartHull => parts_break(placed, zone, true),
                Permission::Forbidden { gap } => {
                    material_breaks(placed, kind, zone, gap.mm(clearance))
                }
            })
}

/// Part boxes (or hulls; against an `Outside` zone a hull fits exactly when its box does).
fn parts_break(placed: &Placed, zone: &PlateZone, hulls: bool) -> bool {
    let boxes = &placed.print.part_boxes;
    match &zone.area {
        ZoneArea::Outside { inside, .. } => boxes.iter().any(|b| b.sticks_out_of(*inside, GAP_EPS)),
        ZoneArea::Rect(rect) if !hulls => boxes.iter().any(|b| {
            b.min_x < rect.max_x - GAP_EPS
                && b.max_x > rect.min_x + GAP_EPS
                && b.min_y < rect.max_y - GAP_EPS
                && b.max_y > rect.min_y + GAP_EPS
        }),
        area => {
            let zone = area_zone(area);
            if hulls && !placed.hulls.is_empty() {
                placed.hulls.iter().any(|hull| hull.conflicts(&zone, 0.0))
            } else {
                boxes
                    .iter()
                    .any(|b| Zone::new(&[rect_ring(*b)]).conflicts(&zone, 0.0))
            }
        }
    }
}

/// The kind's material that can meet `zone` keeps `gap` from it.
fn material_breaks(placed: &Placed, kind: PrintKind, zone: &PlateZone, gap: f64) -> bool {
    let materials = zone_material(&placed.print, kind, zone);
    match &zone.area {
        ZoneArea::Outside {
            inside,
            material_inset_mm,
        } => {
            let allowed = inside.grown(-material_inset_mm);
            materials.iter().any(|(_, loops)| {
                Rect::of_points(loops.iter().flatten())
                    .grown(gap)
                    .sticks_out_of(allowed, GAP_EPS)
            })
        }
        area => {
            let zone = area_zone(area);
            materials.iter().any(|&(band, _)| {
                placed
                    .kind(kind, band)
                    .is_some_and(|m| m.conflicts(&zone, gap))
            })
        }
    }
}

fn area_zone(area: &ZoneArea) -> Zone {
    match area {
        ZoneArea::Rect(rect) => Zone::new(&[rect_ring(*rect)]),
        ZoneArea::Loops(loops) => Zone::new(loops),
        ZoneArea::Outside { .. } => unreachable!("outside zones are box tests"),
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

/// `print` placed by `p`: every kind turned and moved, part boxes rebuilt from the placed
/// hulls (or the placed box corners when the print has no hull per box).
fn place_print(print: &ObjectPrint, p: &PackedPlacement) -> ObjectPrint {
    let place = |loops: &Loops| p.place_loops(loops);
    let bands = |bands: &[Loops]| bands.iter().map(place).collect();
    let part_hulls = place(&print.part_hulls);
    let part_boxes = if part_hulls.len() == print.part_boxes.len() {
        part_hulls
            .iter()
            .map(|hull| Rect::of_points(hull.iter()))
            .collect()
    } else {
        let corners = |b: &Rect| vec![rect_ring(*b)];
        let boxes = print.part_boxes.iter().map(|b| place(&corners(b)));
        boxes
            .map(|ring| Rect::of_points(ring.iter().flatten()))
            .collect()
    };
    ObjectPrint {
        rotation_deg: print.rotation_deg + p.angle_deg,
        part_boxes,
        silhouette: place(&print.silhouette),
        part_hulls,
        landing: place(&print.landing),
        brim: place(&print.brim),
        body: bands(&print.body),
        support_base: place(&print.support_base),
        support: bands(&print.support),
        interface: bands(&print.interface),
        provenance: print.provenance.clone(),
    }
}

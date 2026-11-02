//! The §7.2 matrix for the profile zones (printable boundary, hard, restricted, soft) on one
//! placed object, in exact vector geometry: the reference semantics of what `bed_rules` allow.
//! The tower columns belong to the plate-level tower model and are not checked here.
//!
//! An `ObjectPrint` is built for one rotation class, and its loops, hulls and boxes are already
//! in the plate-aligned frame of that rotation (the mesh rotated about the frame centre; supports
//! are not rotation-equivariant, so they are computed in that frame). Placing it is a pure
//! translation by `origin`.

use geo::{Area, BooleanOps, BoundingRect, Distance, Euclidean, MultiPolygon, Polygon};

use crate::geometry::{Point, Rect, polygon_from_loop, strip_polygon};
use crate::keepout::{Keepout, KeepoutShape, Permission, PlateRules, ZoneClass, permission};
use crate::object_print::{ObjectPrint, PrintKind};

/// Overlap below this area is touching, which every profile-zone rule allows.
const AREA_EPS_MM2: f64 = 1e-9;
/// Equality passes for edge bands and the print reserve.
const EDGE_EPS_MM: f64 = 1e-9;
/// The GUIs' tolerance for a part bbox against the printable bounds (design §7.2: 1e-4).
const PART_INSIDE_EPS_MM: f64 = 1e-4;

/// One kind of a placed object where a plate zone does not allow it.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneHit {
    /// The keepout id, or `printable_area` / `body_edge_band` for the implicit zones.
    pub zone: String,
    pub kind: PrintKind,
    /// The band of a band kind (0 for the first-layer kinds); `None` for the silhouette.
    pub band: Option<usize>,
}

/// Every profile-zone violation of `print` placed at `origin` (plate frame, the frame
/// `bed_rules::plate_rules` returns). Part boxes and hulls are tested once, as the silhouette:
/// the body band rows of the matrix are the same and the boxes cover every height.
pub fn zone_hits(rules: &PlateRules, print: &ObjectPrint, origin: (f64, f64)) -> Vec<ZoneHit> {
    let placed = Placed::new(print, origin);
    let mut hits = Vec::new();
    for zone in zones(rules) {
        for kind in &placed.kinds {
            if zone.meets(kind.band) && zone.hit_by(&placed, kind) {
                hits.push(ZoneHit {
                    zone: zone.id.clone(),
                    kind: kind.kind,
                    band: kind.band,
                });
            }
        }
    }
    hits
}

/// One kind of the placed object as material.
struct PlacedKind {
    kind: PrintKind,
    band: Option<usize>,
    material: MultiPolygon,
}

struct Placed {
    boxes: Vec<Rect>,
    hulls: Vec<Polygon>,
    kinds: Vec<PlacedKind>,
}

impl Placed {
    fn new(print: &ObjectPrint, (dx, dy): (f64, f64)) -> Self {
        let shift = |ring: &Vec<Point>| -> Vec<Point> {
            ring.iter()
                .map(|point| Point {
                    x: point.x + dx,
                    y: point.y + dy,
                })
                .collect()
        };
        let material = |loops: &[Vec<Point>]| {
            // Even-odd rings: XOR keeps holes and islands whatever their winding.
            loops
                .iter()
                .fold(MultiPolygon::new(Vec::new()), |area, ring| {
                    area.xor(&MultiPolygon::new(vec![polygon_from_loop(&shift(ring))]))
                })
        };
        let mut kinds = Vec::new();
        let mut add = |kind, band, loops: &[Vec<Point>]| {
            if !loops.is_empty() {
                kinds.push(PlacedKind {
                    kind,
                    band,
                    material: material(loops),
                });
            }
        };
        add(PrintKind::Silhouette, None, &print.silhouette);
        // The first layer is printed inside band 0.
        add(PrintKind::Landing, Some(0), &print.landing);
        add(PrintKind::Brim, Some(0), &print.brim);
        add(PrintKind::SupportBase, Some(0), &print.support_base);
        for band in 0..print.band_count() {
            for kind in [
                PrintKind::BodyBand,
                PrintKind::SupportBand,
                PrintKind::InterfaceBand,
            ] {
                add(kind, Some(band), print.loops_of(kind, band));
            }
        }
        Self {
            boxes: print
                .part_boxes
                .iter()
                .map(|bounds| bounds.translated(dx, dy))
                .collect(),
            hulls: print
                .part_hulls
                .iter()
                .map(|hull| polygon_from_loop(&shift(hull)))
                .collect(),
            kinds,
        }
    }
}

enum ZoneArea {
    /// Everything outside `inside`: part boxes and hulls may reach `tolerance` past it, other
    /// material stays inside it inset by `reserve` (equality passes).
    Outside {
        inside: Rect,
        tolerance: f64,
        reserve: f64,
    },
    Region(MultiPolygon),
}

struct Zone<'a> {
    id: String,
    class: ZoneClass,
    area: ZoneArea,
    /// The profile keepout, for its z range; `None` for the implicit zones (every height).
    keepout: Option<&'a Keepout>,
}

fn zones(rules: &PlateRules) -> Vec<Zone<'_>> {
    let printable = rules.printable_area;
    let mut zones = vec![Zone {
        id: "printable_area".to_owned(),
        class: ZoneClass::Boundary,
        area: ZoneArea::Outside {
            inside: printable,
            tolerance: PART_INSIDE_EPS_MM,
            reserve: rules.print_edge_reserve_mm,
        },
        keepout: None,
    }];
    if rules.body_edge_band_mm > 0.0 {
        zones.push(Zone {
            id: "body_edge_band".to_owned(),
            class: ZoneClass::Restricted,
            area: edge_band(printable, rules.body_edge_band_mm),
            keepout: None,
        });
    }
    zones.extend(rules.keepouts.iter().map(|keepout| Zone {
        id: keepout.id.clone(),
        class: keepout.class.into(),
        area: keepout_area(&keepout.shape, printable),
        keepout: Some(keepout),
    }));
    zones
}

/// A band along the printable edges: what stays inside is the area inset by `width`. A band
/// wider than half the bed leaves only its centre point.
fn edge_band(printable: Rect, width: f64) -> ZoneArea {
    let centre_x = (printable.min_x + printable.max_x) / 2.0;
    let centre_y = (printable.min_y + printable.max_y) / 2.0;
    ZoneArea::Outside {
        inside: printable
            .inset(width)
            .unwrap_or(Rect::new(centre_x, centre_y, centre_x, centre_y)),
        tolerance: EDGE_EPS_MM,
        reserve: 0.0,
    }
}

fn keepout_area(shape: &KeepoutShape, printable: Rect) -> ZoneArea {
    let region = |polygons: Vec<Polygon>| {
        let area = polygons
            .into_iter()
            .fold(MultiPolygon::new(Vec::new()), |area, polygon| {
                area.union(&MultiPolygon::new(vec![polygon]))
            });
        ZoneArea::Region(area)
    };
    match shape {
        KeepoutShape::Rect(rect) => region(vec![rect_polygon(*rect)]),
        KeepoutShape::Polygon(ring) => region(vec![polygon_from_loop(ring)]),
        KeepoutShape::Strips(strips) => region(
            strips
                .iter()
                .map(|&[x0, y0, x1, y1, width]| printed_line(x0, y0, x1, y1, width))
                .collect(),
        ),
        KeepoutShape::EdgeBand(width) => edge_band(printable, *width),
    }
}

/// A printed line's material: the nozzle's track plus half a width past each end.
fn printed_line(x0: f64, y0: f64, x1: f64, y1: f64, width: f64) -> Polygon {
    let length = (x1 - x0).hypot(y1 - y0);
    let (dx, dy) = if length > 0.0 {
        ((x1 - x0) / length, (y1 - y0) / length)
    } else {
        (1.0, 0.0)
    };
    let cap = width / 2.0;
    strip_polygon(
        x0 - dx * cap,
        y0 - dy * cap,
        x1 + dx * cap,
        y1 + dy * cap,
        width,
    )
}

impl Zone<'_> {
    fn meets(&self, band: Option<usize>) -> bool {
        match (self.keepout, band) {
            (Some(keepout), Some(band)) => keepout.meets_band(band),
            _ => true,
        }
    }

    fn hit_by(&self, placed: &Placed, kind: &PlacedKind) -> bool {
        // Part boxes and hulls carry every height, so only the silhouette row tests them.
        let body_rows = kind.kind == PrintKind::Silhouette;
        match permission(kind.kind, self.class) {
            Permission::Allowed => false,
            Permission::PartBox => {
                body_rows
                    && placed
                        .boxes
                        .iter()
                        .any(|bounds| self.reaches(*bounds, &rect_polygon(*bounds).into()))
            }
            Permission::PartHull => {
                body_rows
                    && placed.hulls.iter().any(|hull| match hull.bounding_rect() {
                        Some(bounds) => self.reaches(rect_of(bounds), &hull.clone().into()),
                        None => false,
                    })
            }
            // Profile-zone cells forbid with `Gap::Zero` only (keepout tests pin the matrix);
            // the clearance and support gaps belong to the tower columns.
            Permission::Forbidden { gap } => self.covers(&kind.material, gap.mm(0.0)),
        }
    }

    /// A part box or hull (with its `bounds`) inside the zone; touching passes.
    fn reaches(&self, bounds: Rect, shape: &MultiPolygon) -> bool {
        match &self.area {
            ZoneArea::Outside {
                inside, tolerance, ..
            } => bounds.sticks_out_of(*inside, *tolerance),
            ZoneArea::Region(region) => overlap(region, shape, 0.0),
        }
    }

    /// Material inside the zone or closer than `gap` to it.
    fn covers(&self, material: &MultiPolygon, gap: f64) -> bool {
        let Some(bounds) = material.bounding_rect() else {
            return false;
        };
        match &self.area {
            ZoneArea::Outside {
                inside, reserve, ..
            } => match inside.inset(reserve + gap) {
                Ok(kept) => rect_of(bounds).sticks_out_of(kept, EDGE_EPS_MM),
                Err(_) => true,
            },
            ZoneArea::Region(region) => overlap(region, material, gap),
        }
    }
}

fn overlap(a: &MultiPolygon, b: &MultiPolygon, gap: f64) -> bool {
    if gap > 0.0 && Euclidean.distance(a, b) < gap - EDGE_EPS_MM {
        return true;
    }
    a.intersection(b).unsigned_area() > AREA_EPS_MM2
}

fn rect_polygon(rect: Rect) -> Polygon {
    polygon_from_loop(&[
        Point {
            x: rect.min_x,
            y: rect.min_y,
        },
        Point {
            x: rect.max_x,
            y: rect.min_y,
        },
        Point {
            x: rect.max_x,
            y: rect.max_y,
        },
        Point {
            x: rect.min_x,
            y: rect.max_y,
        },
    ])
}

fn rect_of(bounds: geo::Rect) -> Rect {
    Rect::new(
        bounds.min().x,
        bounds.min().y,
        bounds.max().x,
        bounds.max().y,
    )
}

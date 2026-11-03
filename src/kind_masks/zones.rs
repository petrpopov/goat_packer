//! Plate zones as geometry in the plate frame: the printable boundary, the edge bands and the
//! profile's keepouts (`PlateRules`), plus whatever the caller adds (the prime tower's body
//! and brim, whose position the search chooses). What a zone forbids comes from its class
//! through `keepout::permission`, never from the zone.

use crate::geometry::{Point, Rect};
use crate::keepout::{Keepout, KeepoutFrame, KeepoutShape, PlateRules, ZoneClass};
use crate::object_print::{Loops, PrintKind, band_z_range};

#[derive(Clone, Debug, PartialEq)]
pub enum ZoneArea {
    /// Everything outside `inside`: the printable boundary and edge bands. Rules about
    /// material keep it inside `inside` shrunk by `material_inset_mm` (the print edge
    /// reserve, boundary only); part boxes use `inside` itself. Support material (and only
    /// it) may reach `support_clip_mm` past that line, because the slicer clips supports at
    /// the bed (`PlateRules::support_clip_mm`).
    Outside {
        inside: Rect,
        material_inset_mm: f64,
        /// Per edge: min x, min y, max x, max y.
        support_clip_mm: [f64; 4],
    },
    Rect(Rect),
    /// Even-odd rings.
    Loops(Loops),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlateZone {
    /// The keepout id, or a fixed name for the implicit zones (in error messages).
    pub id: String,
    pub class: ZoneClass,
    pub area: ZoneArea,
    /// `[from, to]` in mm; `None` = every height.
    pub z_range_mm: Option<[f64; 2]>,
}

impl ZoneArea {
    /// The rectangle material of `kind` must stay inside for an `Outside` area: `inside` shrunk
    /// by the print reserve, grown again by the clip for support material. `None` for the
    /// other areas.
    pub fn material_limit(&self, kind: PrintKind) -> Option<Rect> {
        match self {
            ZoneArea::Outside {
                inside,
                material_inset_mm,
                support_clip_mm,
            } => {
                let clip = if kind.is_support() { *support_clip_mm } else { [0.0; 4] };
                Some(Rect::new(
                    inside.min_x - clip[0] + material_inset_mm,
                    inside.min_y - clip[1] + material_inset_mm,
                    inside.max_x + clip[2] - material_inset_mm,
                    inside.max_y + clip[3] - material_inset_mm,
                ))
            }
            ZoneArea::Rect(_) | ZoneArea::Loops(_) => None,
        }
    }
}

impl PlateZone {
    /// Whether a kind printed in absolute band `band` can meet this zone (the rule of
    /// `Keepout::meets_band`). First-layer kinds count as band 0.
    pub fn meets_band(&self, band: usize) -> bool {
        let (band_from, band_to) = band_z_range(band);
        self.z_range_mm
            .is_none_or(|[from, to]| from < band_to && to >= band_from)
    }
}

/// The zones of `rules`: the printable boundary, the body edge band and every keepout.
/// `gcode_offset` is the project's `extruder_offset` (BambuStudio writes G-code minus it);
/// a keepout in the G-code frame without one is an error, never a silent guess.
pub fn plate_zones(
    rules: &PlateRules,
    gcode_offset: Option<Point>,
) -> Result<Vec<PlateZone>, String> {
    let printable = rules.printable_area;
    let mut zones = vec![PlateZone {
        id: "printable_area".to_owned(),
        class: ZoneClass::Boundary,
        area: ZoneArea::Outside {
            inside: printable,
            material_inset_mm: rules.print_edge_reserve_mm,
            support_clip_mm: rules.support_clip_per_side(),
        },
        z_range_mm: None,
    }];
    if rules.body_edge_band_mm > 0.0 {
        zones.push(PlateZone {
            id: "body_edge_band".to_owned(),
            class: ZoneClass::Restricted,
            area: edge_band(printable, rules.body_edge_band_mm)?,
            z_range_mm: None,
        });
    }
    for keepout in &rules.keepouts {
        zones.extend(keepout_zones(keepout, printable, gcode_offset)?);
    }
    Ok(zones)
}

/// One keepout as zones: strips become one zone each, since overlapping strip corners would
/// cancel under the even-odd rule.
fn keepout_zones(
    keepout: &Keepout,
    printable: Rect,
    gcode_offset: Option<Point>,
) -> Result<Vec<PlateZone>, String> {
    let shift = match keepout.frame {
        KeepoutFrame::Plate => Point { x: 0.0, y: 0.0 },
        KeepoutFrame::Gcode => gcode_offset.ok_or_else(|| {
            format!(
                "keepout {} is in the G-code frame: the project's extruder offset is needed",
                keepout.id
            )
        })?,
    };
    let zone = |id: String, area| PlateZone {
        id,
        class: keepout.class.into(),
        area,
        z_range_mm: keepout.z_range_mm,
    };
    let areas = match &keepout.shape {
        KeepoutShape::Rect(rect) => vec![ZoneArea::Rect(rect.translated(shift.x, shift.y))],
        KeepoutShape::Polygon(ring) => vec![ZoneArea::Loops(vec![closed(ring, shift)])],
        KeepoutShape::Strips(strips) => strips
            .iter()
            .map(|strip| ZoneArea::Loops(vec![strip_ring(*strip, shift)]))
            .collect(),
        KeepoutShape::EdgeBand(width) => vec![edge_band(printable, *width)?],
    };
    let single = areas.len() == 1;
    Ok(areas
        .into_iter()
        .enumerate()
        .map(|(index, area)| {
            let id = if single {
                keepout.id.clone()
            } else {
                format!("{}#{index}", keepout.id)
            };
            zone(id, area)
        })
        .collect())
}

fn edge_band(printable: Rect, width: f64) -> Result<ZoneArea, String> {
    Ok(ZoneArea::Outside {
        inside: printable.inset(width)?,
        material_inset_mm: 0.0,
        support_clip_mm: [0.0; 4],
    })
}

fn closed(ring: &[Point], shift: Point) -> Vec<Point> {
    let mut moved: Vec<Point> = ring
        .iter()
        .map(|p| Point {
            x: p.x + shift.x,
            y: p.y + shift.y,
        })
        .collect();
    if moved.first() != moved.last()
        && let Some(&first) = moved.first()
    {
        moved.push(first);
    }
    moved
}

/// A printed line `[x1, y1, x2, y2, width]` as a rectangle reaching half the width past both
/// ends: it contains the line with round, square or butt caps.
fn strip_ring([x1, y1, x2, y2, width]: [f64; 5], shift: Point) -> Vec<Point> {
    let half = width * 0.5;
    let length = (x2 - x1).hypot(y2 - y1);
    let (dx, dy) = if length > 0.0 {
        ((x2 - x1) / length * half, (y2 - y1) / length * half)
    } else {
        (half, 0.0)
    };
    let (nx, ny) = (-dy, dx);
    let corner = |x: f64, y: f64, along: f64, across: f64| Point {
        x: x + along * dx + across * nx + shift.x,
        y: y + along * dy + across * ny + shift.y,
    };
    let ring = vec![
        corner(x1, y1, -1.0, -1.0),
        corner(x2, y2, 1.0, -1.0),
        corner(x2, y2, 1.0, 1.0),
        corner(x1, y1, -1.0, 1.0),
    ];
    closed(&ring, Point { x: 0.0, y: 0.0 })
}

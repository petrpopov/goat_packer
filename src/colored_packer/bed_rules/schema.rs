//! The profile's `bed_rules` block as written in `profiles/*.json` (design §7.4): flat keys a
//! person can edit, checked at load and turned into `keepout::Keepout`s per job.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::flavour::GuiFlavour;
use crate::geometry::{Point, Rect};
use crate::keepout::{Keepout, KeepoutClass, KeepoutFrame, KeepoutShape};

/// `bed_rules`: the printer's plate zones. An absent block = the old fields through the legacy
/// table (`legacy_bed_rules`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ProfileBedRules {
    /// Restricted band along the printable edges: every model part's bbox keeps at least this
    /// distance from the edge (equality passes). A job uses the wider of this and its GUI's
    /// own band (`GuiFlavour::body_edge_band_mm`).
    #[serde(default)]
    pub body_edge_band_mm: f64,
    /// Supports and the tower stay inside the printable area inset by this.
    #[serde(default)]
    pub print_edge_reserve_mm: f64,
    #[serde(default)]
    pub keepouts: Vec<ProfileKeepout>,
}

/// One plate zone. Exactly one of `rect`, `polygon`, `strips` and `edge_band_mm` gives its
/// shape; what its class forbids is `keepout::matrix`, never profile data.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProfileKeepout {
    pub id: String,
    pub class: KeepoutClass,
    /// `[x0, y0, x1, y1]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<[f64; 4]>,
    /// One closed ring of `[x, y]` points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polygon: Option<Vec<[f64; 2]>>,
    /// Printed lines `[x1, y1, x2, y2, width]`, like `purge_lines`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strips: Option<Vec<[f64; 5]>>,
    /// A band of this width inside the printable area along its edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_band_mm: Option<f64>,
    /// `[from, to]` mm; absent = every height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_range_mm: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "is_plate_frame")]
    pub frame: KeepoutFrame,
    /// Where the zone comes from (`bed_exclude_area`, `machine_start_gcode`, ...), for people.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn is_plate_frame(frame: &KeepoutFrame) -> bool {
    *frame == KeepoutFrame::Plate
}

impl ProfileBedRules {
    /// Rejects what a person can get wrong by hand: negative bands, a zone with no shape or
    /// two, inverted ranges, duplicate ids.
    pub fn validate(&self) -> Result<(), String> {
        non_negative("body_edge_band_mm", self.body_edge_band_mm)?;
        non_negative("print_edge_reserve_mm", self.print_edge_reserve_mm)?;
        let mut ids = BTreeSet::new();
        for keepout in &self.keepouts {
            keepout
                .validate()
                .map_err(|error| format!("bed_rules keepout {:?}: {error}", keepout.id))?;
            if !ids.insert(keepout.id.as_str()) {
                return Err(format!("bed_rules: duplicate keepout id {:?}", keepout.id));
            }
        }
        Ok(())
    }

    /// How far every model part stays from the printable edge under these rules and `gui`:
    /// the widest of the restricted body band, the GUI's band and the hard edge bands. The
    /// old `edge_margin_mm` must say the same: the webapp shows it and `source_project
    /// --edge-margin` rejects models that cannot fit inside it.
    pub fn body_margin_mm(&self, gui: Option<&GuiFlavour>) -> f64 {
        let hard_bands = self
            .keepouts
            .iter()
            .filter(|keepout| keepout.class == KeepoutClass::Hard)
            .filter_map(|keepout| keepout.edge_band_mm);
        hard_bands
            .chain([
                self.body_edge_band_mm,
                gui.map_or(0.0, GuiFlavour::body_edge_band_mm),
            ])
            .fold(0.0, f64::max)
    }
}

impl ProfileKeepout {
    /// A zone of `class` with no shape yet; the `with_*` methods below add it.
    pub fn new(id: &str, class: KeepoutClass) -> Self {
        Self {
            id: id.to_owned(),
            class,
            rect: None,
            polygon: None,
            strips: None,
            edge_band_mm: None,
            z_range_mm: None,
            frame: KeepoutFrame::Plate,
            source: None,
        }
    }

    pub fn with_rect(mut self, rect: [f64; 4]) -> Self {
        self.rect = Some(rect);
        self
    }

    pub fn with_strips(mut self, strips: Vec<[f64; 5]>) -> Self {
        self.strips = Some(strips);
        self
    }

    pub fn with_edge_band(mut self, width_mm: f64) -> Self {
        self.edge_band_mm = Some(width_mm);
        self
    }

    /// Numbers in G-code coordinates (start G-code purges).
    pub fn in_gcode_frame(mut self) -> Self {
        self.frame = KeepoutFrame::Gcode;
        self
    }

    /// Only the heights up to `z_max_mm` (a purge printed at the first layer).
    pub fn up_to(mut self, z_max_mm: f64) -> Self {
        self.z_range_mm = Some([0.0, z_max_mm]);
        self
    }

    pub fn from_source(mut self, source: &str) -> Self {
        self.source = Some(source.to_owned());
        self
    }

    /// This zone in the plate frame: `frame: gcode` numbers move by the project's
    /// `gcode_offset` (`extruder_offset`), plate-frame ones stay.
    pub fn to_plate(&self, gcode_offset: (f64, f64)) -> Result<Keepout, String> {
        let offset = match self.frame {
            KeepoutFrame::Plate => (0.0, 0.0),
            KeepoutFrame::Gcode => gcode_offset,
        };
        Ok(Keepout {
            id: self.id.clone(),
            class: self.class,
            shape: self.shape(offset)?,
            z_range_mm: self.z_range_mm,
            frame: KeepoutFrame::Plate,
        })
    }

    fn shape(&self, (dx, dy): (f64, f64)) -> Result<KeepoutShape, String> {
        match (&self.rect, &self.polygon, &self.strips, self.edge_band_mm) {
            (Some([x0, y0, x1, y1]), None, None, None) => Ok(KeepoutShape::Rect(Rect::new(
                x0 + dx,
                y0 + dy,
                x1 + dx,
                y1 + dy,
            ))),
            (None, Some(points), None, None) => Ok(KeepoutShape::Polygon(
                points
                    .iter()
                    .map(|[x, y]| Point {
                        x: x + dx,
                        y: y + dy,
                    })
                    .collect(),
            )),
            (None, None, Some(strips), None) => Ok(KeepoutShape::Strips(
                strips
                    .iter()
                    .map(|[x1, y1, x2, y2, width]| [x1 + dx, y1 + dy, x2 + dx, y2 + dy, *width])
                    .collect(),
            )),
            (None, None, None, Some(width)) => Ok(KeepoutShape::EdgeBand(width)),
            _ => Err("needs exactly one of rect, polygon, strips, edge_band_mm".to_owned()),
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("empty id".to_owned());
        }
        self.shape((0.0, 0.0))?;
        if let Some([x0, y0, x1, y1]) = self.rect
            && !(all_finite(&[x0, y0, x1, y1]) && x0 < x1 && y0 < y1)
        {
            return Err("rect must be [x0, y0, x1, y1] with x0 < x1 and y0 < y1".to_owned());
        }
        if let Some(points) = &self.polygon
            && (points.len() < 3 || !points.iter().all(|point| all_finite(point)))
        {
            return Err("polygon needs at least three finite points".to_owned());
        }
        if let Some(strips) = &self.strips
            && (strips.is_empty()
                || !strips
                    .iter()
                    .all(|strip| all_finite(strip) && strip[4] > 0.0))
        {
            return Err("strips need finite [x1, y1, x2, y2, width] with width > 0".to_owned());
        }
        if let Some(width) = self.edge_band_mm {
            non_negative("edge_band_mm", width)?;
            // A band is measured from the printable edges; a G-code shift has no meaning there.
            if self.frame == KeepoutFrame::Gcode {
                return Err("an edge band is always in the plate frame".to_owned());
            }
        }
        if let Some([from, to]) = self.z_range_mm
            && !(all_finite(&[from, to]) && from <= to)
        {
            return Err("z_range_mm must be [from, to] with from <= to".to_owned());
        }
        Ok(())
    }
}

fn all_finite(values: &[f64]) -> bool {
    values.iter().all(|value| value.is_finite())
}

fn non_negative(name: &str, value: f64) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be a finite number >= 0, got {value}"))
    }
}

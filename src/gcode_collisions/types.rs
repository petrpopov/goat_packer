//! What the G-code check reports.

use serde::Serialize;

use crate::geometry::Rect;

/// One printed line on the first layer, in bed coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strip {
    pub from: (f64, f64),
    pub to: (f64, f64),
    pub width: f64,
    pub support: bool,
}

/// What one object laid down on the first layer.
#[derive(Clone, Debug, Default)]
pub struct ObjectFirstLayer {
    pub label: String,
    pub strips: Vec<Strip>,
}

impl ObjectFirstLayer {
    /// Bbox centre of the object's non-support first-layer lines (its landing zone): matches
    /// the object to its placement.
    pub fn body_center(&self) -> Option<(f64, f64)> {
        let mut bounds: Option<(f64, f64, f64, f64)> = None;
        for strip in self.strips.iter().filter(|strip| !strip.support) {
            for (x, y) in [strip.from, strip.to] {
                let b = bounds.get_or_insert((x, y, x, y));
                *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
        bounds.map(|(x0, y0, x1, y1)| ((x0 + x1) * 0.5, (y0 + y1) * 0.5))
    }
}

/// Support of `support_object` touching the body of `body_object`, aggregated over layers.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SupportCollision {
    pub support_object: String,
    pub body_object: String,
    /// 1-based layer numbers (layer 1 = first layer).
    pub first_layer: usize,
    pub last_layer: usize,
    pub layers: usize,
    /// Smallest edge-to-edge gap seen (negative = overlap depth), mm.
    pub min_gap_mm: f64,
}

#[derive(Clone, Debug, Default)]
pub struct PlateAnalysis {
    pub collisions: Vec<SupportCollision>,
    pub first_layer: Vec<ObjectFirstLayer>,
    /// Layers seen; 0 means the parser found no layer markers (not a slicer G-code).
    pub layers: usize,
    /// Printed lines attributed to an object; 0 means no object labels were found, so the
    /// check proved nothing.
    pub labelled_lines: usize,
    /// Smallest edge gap between a support above the first layer and a foreign body, among
    /// pairs closer than 3 mm (negative = overlap depth); `None` = none that close. Reported
    /// against the packer's band gap, never a failure.
    pub min_support_gap_mm: Option<f64>,
    /// Every object's material envelope over all layers: sampled arc/line centres plus
    /// half the extrusion width. Checking centre lines alone lets beads leave the bed.
    pub extents: Vec<(String, Rect)>,
}

impl PlateAnalysis {
    /// A partial/unlabelled slice cannot certify a full plate. This checks coverage, not
    /// collision freedom; callers must still inspect collisions and bed extents.
    pub fn require_object_coverage(&self, expected: usize) -> Result<(), String> {
        let labels: std::collections::HashSet<_> =
            self.extents.iter().map(|(label, _)| label).collect();
        let first: std::collections::HashSet<_> = self
            .first_layer
            .iter()
            .filter(|o| !o.strips.is_empty())
            .map(|o| &o.label)
            .collect();
        if expected == 0
            || self.layers == 0
            || self.labelled_lines == 0
            || labels.len() != expected
            || first != labels
        {
            return Err(format!(
                "cannot verify slice: expected {expected} labelled objects, found {} with {} layers",
                labels.len(),
                self.layers
            ));
        }
        Ok(())
    }

    /// Objects whose printed lines leave `area` (the printable area), with their extent. The
    /// slicer GUI refuses to export such a plate ("G-code path goes beyond plate
    /// boundaries"); its CLI writes it without a word.
    pub fn off_plate(&self, area: Rect) -> Vec<(String, Rect)> {
        const EPSILON_MM: f64 = 1e-3;
        self.extents
            .iter()
            .filter(|(_, extent)| extent.sticks_out_of(area, EPSILON_MM))
            .cloned()
            .collect()
    }
}

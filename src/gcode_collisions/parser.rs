//! Reading a sliced plate line by line: layers, features, labelled objects and moves.

use std::collections::{BTreeMap, HashMap};

use super::touching::{Line, layer_collisions};
use super::types::{ObjectFirstLayer, Strip, SupportCollision};
use crate::gcode_text::{
    ArcSweep, DENSITY_CHECK_MIN_LENGTH_MM, MIN_FLOW_PER_MM, axis_value, feature_label,
    is_layer_change, line_width_comment,
};
use crate::geometry::Rect;

/// Arc sampling step; well under a line width.
const ARC_STEP_MM: f64 = 0.3;

/// The state of reading one plate: the labelled objects, the layer and feature being printed,
/// the head position, and what has been found so far.
#[derive(Default)]
pub(super) struct Parser {
    pub(super) labels: Vec<String>,
    index_of: HashMap<String, usize>,
    current: Option<usize>,
    pub(super) layer: usize,
    feature: String,
    width: f64,
    x: f64,
    y: f64,
    lines: Vec<Line>,
    pub(super) first_layer: Vec<ObjectFirstLayer>,
    pub(super) collisions: BTreeMap<(usize, usize), SupportCollision>,
    pub(super) labelled_lines: usize,
    pub(super) extents: Vec<Option<Rect>>,
    z: f64,
    /// Every labelled line with its layer height, kept only for `layer_zones_from_sample`.
    keep_lines: bool,
    pub(super) kept: Vec<KeptLine>,
}

/// A labelled printed line of any layer (`Parser::keep_lines`).
pub(super) struct KeptLine {
    pub(super) object: usize,
    pub(super) z: f64,
    pub(super) strip: Strip,
    pub(super) infill: bool,
}

impl Parser {
    fn object(&mut self, label: &str) -> usize {
        if let Some(&index) = self.index_of.get(label) {
            return index;
        }
        let index = self.labels.len();
        self.labels.push(label.to_owned());
        self.index_of.insert(label.to_owned(), index);
        self.first_layer.push(ObjectFirstLayer {
            label: label.to_owned(),
            strips: Vec::new(),
        });
        self.extents.push(None);
        index
    }

    fn ink(&mut self, from: (f64, f64), to: (f64, f64)) {
        let Some(object) = self.current else {
            return;
        };
        let support = self.feature.to_lowercase().contains("support");
        let width = self.width;
        self.labelled_lines += 1;
        let extent = self.extents[object].get_or_insert(Rect::new(from.0, from.1, from.0, from.1));
        for (x, y) in [from, to] {
            extent.min_x = extent.min_x.min(x - width * 0.5);
            extent.min_y = extent.min_y.min(y - width * 0.5);
            extent.max_x = extent.max_x.max(x + width * 0.5);
            extent.max_y = extent.max_y.max(y + width * 0.5);
        }
        if self.keep_lines {
            self.kept.push(KeptLine {
                object,
                z: self.z,
                strip: Strip {
                    from,
                    to,
                    width,
                    support,
                },
                infill: self.feature.to_lowercase().contains("infill"),
            });
        }
        if self.layer == 1 {
            self.first_layer[object].strips.push(Strip {
                from,
                to,
                width,
                support,
            });
        }
        self.lines.push(Line {
            object,
            from,
            to,
            width,
            support,
        });
    }

    fn finish_layer(&mut self) {
        if self.layer > 0 && !self.lines.is_empty() {
            let lines = std::mem::take(&mut self.lines);
            for (support_object, body_object, gap) in layer_collisions(&lines) {
                let layer = self.layer;
                self.collisions
                    .entry((support_object, body_object))
                    .and_modify(|c| {
                        c.last_layer = layer;
                        c.layers += 1;
                        c.min_gap_mm = c.min_gap_mm.min(gap);
                    })
                    .or_insert_with(|| SupportCollision {
                        support_object: self.labels[support_object].clone(),
                        body_object: self.labels[body_object].clone(),
                        first_layer: layer,
                        last_layer: layer,
                        layers: 1,
                        min_gap_mm: gap,
                    });
            }
        }
        self.lines.clear();
    }

    fn line(&mut self, raw: &str) {
        let line = raw.trim();
        if is_layer_change(line) {
            self.finish_layer();
            self.layer += 1;
            return;
        }
        if let Some(z) = line
            .strip_prefix("; Z_HEIGHT:")
            .or_else(|| line.strip_prefix(";Z:"))
            .and_then(|value| value.trim().parse::<f64>().ok())
        {
            self.z = z;
            return;
        }
        if let Some(rest) = line.strip_prefix("; start printing object, unique label id:") {
            self.current = Some(self.object(rest.trim()));
            return;
        }
        if line.starts_with("; stop printing object") || line.starts_with("EXCLUDE_OBJECT_END") {
            self.current = None;
            return;
        }
        if let Some(rest) = line.strip_prefix("; printing object ") {
            self.current = Some(self.object(rest.trim()));
            return;
        }
        if let Some(rest) = line.strip_prefix("EXCLUDE_OBJECT_START") {
            let name = rest
                .split_whitespace()
                .find_map(|token| token.strip_prefix("NAME="))
                .unwrap_or(rest.trim());
            self.current = Some(self.object(name));
            return;
        }
        if let Some(kind) = feature_label(line) {
            self.feature = kind.to_owned();
            return;
        }
        if let Some(width) = line_width_comment(line) {
            self.width = width.unwrap_or(self.width);
            return;
        }
        let arc = line.starts_with("G2 ") || line.starts_with("G3 ");
        if arc || line.starts_with("G1 ") || line.starts_with("G0 ") {
            self.read_move(line, arc);
        }
    }

    fn read_move(&mut self, line: &str, arc: bool) {
        let x = axis_value(line, 'X');
        let y = axis_value(line, 'Y');
        let next = (x.unwrap_or(self.x), y.unwrap_or(self.y));
        let from = (self.x, self.y);
        (self.x, self.y) = next;
        let filament = axis_value(line, 'E').unwrap_or(0.0);
        // Un-retractions carry no X/Y; a move without an extrusion is travel.
        if filament <= 0.0 || (x.is_none() && y.is_none()) || self.layer == 0 {
            return;
        }
        if !arc {
            self.ink_straight(from, next, filament);
            return;
        }
        let (Some(i), Some(j)) = (axis_value(line, 'I'), axis_value(line, 'J')) else {
            return;
        };
        let Some(sweep) = ArcSweep::new(from, next, (i, j), line.starts_with("G2")) else {
            return;
        };
        let steps = ((sweep.length() / ARC_STEP_MM).ceil() as usize).max(1);
        let mut previous = from;
        for point in sweep.points(steps) {
            self.ink(previous, point);
            previous = point;
        }
    }

    fn ink_straight(&mut self, from: (f64, f64), to: (f64, f64), filament: f64) {
        let length = (to.0 - from.0).hypot(to.1 - from.1);
        // Same density cut as the first-layer patch parser: a travel-smeared smear prints
        // an order of magnitude below the thinnest real (Arachne) line.
        if length > DENSITY_CHECK_MIN_LENGTH_MM && filament / length <= MIN_FLOW_PER_MM {
            return;
        }
        self.ink(from, to);
    }
}

pub(super) fn extents_of(parser: &Parser) -> Vec<(String, Rect)> {
    parser
        .labels
        .iter()
        .zip(&parser.extents)
        .filter_map(|(label, extent)| Some((label.clone(), (*extent)?)))
        .collect()
}

pub(super) fn parse(gcode: &[u8], keep_lines: bool) -> Parser {
    let mut parser = Parser {
        width: 0.45,
        keep_lines,
        ..Parser::default()
    };
    for raw in gcode.split(|&byte| byte == b'\n') {
        // Only comments and moves matter; both are ASCII, so a lossy view is exact for them.
        parser.line(&String::from_utf8_lossy(raw));
    }
    parser.finish_layer();
    parser
}

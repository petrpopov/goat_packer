//! The material view of a sliced plate: every object's printed lines layer by layer, their
//! extents, the first layer, and support lines that touch a foreign body. The G-code itself is
//! walked by `reader`; this module only decides what the lines mean.

use std::collections::BTreeMap;

use super::reader::{Extrusion, ExtrusionSink, Role, read};
use super::touching::{
    FIRST_LAYER_BASE_OVERLAP_MM, GAP_STATISTIC_REACH_MM, Line, TOUCH_EPSILON_MM, layer_collisions,
};
use super::types::{ObjectFirstLayer, Strip, SupportCollision};
use crate::geometry::Rect;

/// What reading one plate found, per labelled object.
#[derive(Default)]
pub(super) struct Parser {
    pub(super) labels: Vec<String>,
    /// Layers seen.
    pub(super) layer: usize,
    lines: Vec<Line>,
    pub(super) first_layer: Vec<ObjectFirstLayer>,
    pub(super) collisions: BTreeMap<(usize, usize), SupportCollision>,
    /// Touching pairs by object index until the labels are known: first layer, last layer,
    /// layers, smallest gap.
    touching: BTreeMap<(usize, usize), (usize, usize, usize, f64)>,
    first_strips: Vec<Vec<Strip>>,
    pub(super) labelled_lines: usize,
    /// Smallest support-to-foreign-body gap above the first layer among pairs closer than
    /// `GAP_STATISTIC_REACH_MM`.
    pub(super) min_support_gap_mm: Option<f64>,
    pub(super) extents: Vec<Option<Rect>>,
    /// Every labelled line with its layer height, kept only for
    /// `gcode_footprint::realization_from_gcode`.
    keep_lines: bool,
    pub(super) kept: Vec<KeptLine>,
    /// Off when only the lines are wanted (`object_lines`): the contact search is the costly part.
    skip_contacts: bool,
}

/// A labelled printed line of any layer (`Parser::keep_lines`).
pub(crate) struct KeptLine {
    pub(crate) object: usize,
    /// 1-based layer; 1 is the printed first layer.
    pub(crate) layer: usize,
    pub(crate) z: f64,
    /// The previous layer's height: the layer occupies `(z_bottom, z]`.
    pub(crate) z_bottom: f64,
    pub(crate) strip: Strip,
    pub(crate) interface: bool,
}

impl ExtrusionSink for Parser {
    fn extrusion(&mut self, extrusion: &Extrusion) {
        let Some(object) = extrusion.object else {
            return;
        };
        // Machine start G-code prints before the first layer; a travel-smeared un-retraction
        // is no line at all.
        if extrusion.layer == 0 || extrusion.is_smear() {
            return;
        }
        self.grow_to(object);
        let extent = extrusion.extent.grown(extrusion.width * 0.5);
        let known = &mut self.extents[object];
        *known = Some(known.map_or(extent, |seen| seen.union(extent)));
        for pair in extrusion.path.windows(2) {
            self.ink(object, extrusion, pair[0], pair[1]);
        }
    }

    fn layer_done(&mut self, layer: usize) {
        self.layer = self.layer.max(layer);
        let lines = std::mem::take(&mut self.lines);
        if layer == 0 || lines.is_empty() || self.skip_contacts {
            return;
        }
        for (support_object, body_object, gap) in layer_collisions(&lines, GAP_STATISTIC_REACH_MM) {
            if layer > 1 {
                let seen = self.min_support_gap_mm.unwrap_or(f64::INFINITY);
                self.min_support_gap_mm = Some(seen.min(gap));
            }
            // The wider search only feeds the statistic; failures stay contact-only.
            if gap >= TOUCH_EPSILON_MM {
                continue;
            }
            if layer == 1 && gap >= -FIRST_LAYER_BASE_OVERLAP_MM {
                continue;
            }
            self.touching
                .entry((support_object, body_object))
                .and_modify(|seen| {
                    seen.1 = layer;
                    seen.2 += 1;
                    seen.3 = seen.3.min(gap);
                })
                .or_insert((layer, layer, 1, gap));
        }
    }
}

impl Parser {
    fn grow_to(&mut self, object: usize) {
        if self.extents.len() <= object {
            self.extents.resize(object + 1, None);
            self.first_strips.resize_with(object + 1, Vec::new);
        }
    }

    fn ink(&mut self, object: usize, extrusion: &Extrusion, from: (f64, f64), to: (f64, f64)) {
        let support = extrusion.role.is_support();
        let strip = Strip {
            from,
            to,
            width: extrusion.width,
            support,
        };
        self.labelled_lines += 1;
        if self.keep_lines {
            self.kept.push(KeptLine {
                object,
                layer: extrusion.layer,
                z: extrusion.z,
                z_bottom: extrusion.z_bottom,
                strip,
                interface: extrusion.role == Role::SupportInterface,
            });
        }
        if extrusion.layer == 1 {
            self.first_strips[object].push(strip);
        }
        self.lines.push(Line {
            object,
            from,
            to,
            width: extrusion.width,
            support,
        });
    }

    /// Names everything by label once the whole plate is read.
    fn finish(&mut self, labels: Vec<String>) {
        if !labels.is_empty() {
            self.grow_to(labels.len() - 1);
        }
        let strips = std::mem::take(&mut self.first_strips);
        self.first_layer = labels
            .iter()
            .zip(strips)
            .map(|(label, strips)| ObjectFirstLayer {
                label: label.clone(),
                strips,
            })
            .collect();
        self.collisions = std::mem::take(&mut self.touching)
            .into_iter()
            .map(|((support, body), (first, last, layers, gap))| {
                let collision = SupportCollision {
                    support_object: labels[support].clone(),
                    body_object: labels[body].clone(),
                    first_layer: first,
                    last_layer: last,
                    layers,
                    min_gap_mm: gap,
                };
                ((support, body), collision)
            })
            .collect();
        self.labels = labels;
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
        keep_lines,
        ..Parser::default()
    };
    let labels = read(gcode, &mut parser);
    parser.finish(labels);
    parser
}

/// Every labelled object's printed lines with their layer, height and role, plus the labels
/// they index.
pub(crate) fn object_lines(gcode: &[u8]) -> (Vec<String>, Vec<KeptLine>) {
    let mut parser = Parser {
        keep_lines: true,
        skip_contacts: true,
        ..Parser::default()
    };
    let labels = read(gcode, &mut parser);
    parser.finish(labels);
    (parser.labels, parser.kept)
}

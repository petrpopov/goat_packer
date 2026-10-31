//! Layer-by-layer check of a sliced plate: does any object's support touch another object's
//! printed body? (spec 2026-10-31-support-landing-and-progress-design, rule 4)
//!
//! The packer reserves a support patch measured on a one-object probe, but the slicer
//! re-generates tree supports for every plate and the realization drifts between slices of
//! the same project (565 / 617 / 643 / 657 mm² of first-layer support for one P1S goat). So the
//! final G-code itself is the evidence: every printed line is attributed to its object via
//! the slicer's object labels and classified as support or body, and per layer each support
//! line is tested against foreign body lines. Two lines collide when their centre lines come
//! closer than the sum of their half widths (they touch or overlap).
//!
//! Both dialects: BambuStudio labels objects with `; start printing object, unique label id:
//! N` / `; stop printing object, unique label id: N`, OrcaSlicer with `; printing object NAME
//! id:N copy M` / `; stop printing object ...` (plus Klipper `EXCLUDE_OBJECT_START NAME=...`
//! / `EXCLUDE_OBJECT_END`). Layers: `; CHANGE_LAYER` / `;LAYER_CHANGE`; features: `; FEATURE:`
//! / `;TYPE:`; widths: `; LINE_WIDTH:` / `;WIDTH:`. Lines outside any object (prime tower,
//! purge, skirt) are ignored.
use std::collections::{BTreeMap, HashMap};

use geo::Polygon;
use serde::Serialize;

use crate::{Footprint, PackedPlacement};

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
}

/// Edge gaps below this count as touching: absorbs float noise in the slicers' own
/// rounding (a wall and a support at exactly the touching distance).
const TOUCH_EPSILON_MM: f64 = 0.01;
/// Spatial hash cell for the body lines of one layer.
const CELL_MM: f64 = 2.0;
/// Arc sampling step; well under a line width.
const ARC_STEP_MM: f64 = 0.3;

struct Line {
    object: usize,
    from: (f64, f64),
    to: (f64, f64),
    width: f64,
    support: bool,
}

#[derive(Default)]
struct Parser {
    labels: Vec<String>,
    index_of: HashMap<String, usize>,
    current: Option<usize>,
    layer: usize,
    feature: String,
    width: f64,
    x: f64,
    y: f64,
    lines: Vec<Line>,
    first_layer: Vec<ObjectFirstLayer>,
    collisions: BTreeMap<(usize, usize), SupportCollision>,
    labelled_lines: usize,
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
        index
    }

    fn ink(&mut self, from: (f64, f64), to: (f64, f64)) {
        let Some(object) = self.current else {
            return;
        };
        let support = self.feature.to_lowercase().contains("support");
        let width = self.width;
        self.labelled_lines += 1;
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
        if line == "; CHANGE_LAYER" || line == ";LAYER_CHANGE" {
            self.finish_layer();
            self.layer += 1;
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
        if let Some(kind) = line
            .strip_prefix("; FEATURE:")
            .or_else(|| line.strip_prefix(";TYPE:"))
            .or_else(|| line.strip_prefix(";FEATURE:"))
        {
            self.feature = kind.trim().to_owned();
            return;
        }
        if let Some(value) = line
            .strip_prefix("; LINE_WIDTH:")
            .or_else(|| line.strip_prefix(";WIDTH:"))
            .or_else(|| line.strip_prefix(";LINE_WIDTH:"))
        {
            if let Ok(width) = value.trim().parse::<f64>() {
                self.width = width;
            }
            return;
        }
        let arc = line.starts_with("G2 ") || line.starts_with("G3 ");
        if !(arc || line.starts_with("G1 ") || line.starts_with("G0 ")) {
            return;
        }
        let x = axis(line, 'X');
        let y = axis(line, 'Y');
        let next = (x.unwrap_or(self.x), y.unwrap_or(self.y));
        let from = (self.x, self.y);
        self.x = next.0;
        self.y = next.1;
        let filament = axis(line, 'E').unwrap_or(0.0);
        // Un-retractions carry no X/Y; a move without an extrusion is travel.
        if filament <= 0.0 || (x.is_none() && y.is_none()) || self.layer == 0 {
            return;
        }
        if arc {
            let (Some(i), Some(j)) = (axis(line, 'I'), axis(line, 'J')) else {
                return;
            };
            let centre = (from.0 + i, from.1 + j);
            let radius = i.hypot(j);
            if radius < 1e-9 {
                return;
            }
            let start = (from.1 - centre.1).atan2(from.0 - centre.0);
            let end = (next.1 - centre.1).atan2(next.0 - centre.0);
            let mut sweep = end - start;
            if line.starts_with("G2") {
                if sweep >= 0.0 {
                    sweep -= std::f64::consts::TAU;
                }
            } else if sweep <= 0.0 {
                sweep += std::f64::consts::TAU;
            }
            let steps = ((radius * sweep.abs() / ARC_STEP_MM).ceil() as usize).max(1);
            let mut previous = from;
            for step in 1..=steps {
                let angle = start + sweep * step as f64 / steps as f64;
                let point = (
                    centre.0 + radius * angle.cos(),
                    centre.1 + radius * angle.sin(),
                );
                self.ink(previous, point);
                previous = point;
            }
        } else {
            let length = (next.0 - from.0).hypot(next.1 - from.1);
            // Same density cut as the first-layer patch parser: a travel-smeared smear prints
            // an order of magnitude below the thinnest real (Arachne) line.
            if length > 0.3 && filament / length <= 0.001 {
                return;
            }
            self.ink(from, next);
        }
    }
}

fn axis(line: &str, axis: char) -> Option<f64> {
    let code = line.split(';').next().unwrap_or("");
    code.split_whitespace()
        .skip(1)
        .find_map(|token| token.strip_prefix(axis).and_then(|v| v.parse::<f64>().ok()))
}

fn cell(point: (f64, f64)) -> (i32, i32) {
    (
        (point.0 / CELL_MM).floor() as i32,
        (point.1 / CELL_MM).floor() as i32,
    )
}

/// Segment-to-segment distance in the plane (0 when they cross).
fn segment_distance(a0: (f64, f64), a1: (f64, f64), b0: (f64, f64), b1: (f64, f64)) -> f64 {
    let cross = |o: (f64, f64), p: (f64, f64), q: (f64, f64)| {
        (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0)
    };
    let d1 = cross(b0, b1, a0);
    let d2 = cross(b0, b1, a1);
    let d3 = cross(a0, a1, b0);
    let d4 = cross(a0, a1, b1);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return 0.0;
    }
    point_segment_distance(a0, b0, b1)
        .min(point_segment_distance(a1, b0, b1))
        .min(point_segment_distance(b0, a0, a1))
        .min(point_segment_distance(b1, a0, a1))
}

fn point_segment_distance(p: (f64, f64), s0: (f64, f64), s1: (f64, f64)) -> f64 {
    let (dx, dy) = (s1.0 - s0.0, s1.1 - s0.1);
    let length2 = dx * dx + dy * dy;
    let t = if length2 < 1e-18 {
        0.0
    } else {
        (((p.0 - s0.0) * dx + (p.1 - s0.1) * dy) / length2).clamp(0.0, 1.0)
    };
    (p.0 - (s0.0 + t * dx)).hypot(p.1 - (s0.1 + t * dy))
}

/// (support object, body object, smallest edge gap) for every touching pair on one layer.
fn layer_collisions(lines: &[Line]) -> Vec<(usize, usize, f64)> {
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    let mut widest = 0.0f64;
    for (index, line) in lines.iter().enumerate().filter(|(_, line)| !line.support) {
        widest = widest.max(line.width);
        let (c0, c1) = (cell(line.from), cell(line.to));
        for cx in c0.0.min(c1.0)..=c0.0.max(c1.0) {
            for cy in c0.1.min(c1.1)..=c0.1.max(c1.1) {
                grid.entry((cx, cy)).or_default().push(index);
            }
        }
    }
    if grid.is_empty() {
        return Vec::new();
    }
    let mut found: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for support in lines.iter().filter(|line| line.support) {
        // Every body line within reach sits in a cell overlapping the support line's bbox
        // grown by the widest possible touching distance.
        let reach = (support.width + widest) * 0.5 + TOUCH_EPSILON_MM;
        let lo = cell((
            support.from.0.min(support.to.0) - reach,
            support.from.1.min(support.to.1) - reach,
        ));
        let hi = cell((
            support.from.0.max(support.to.0) + reach,
            support.from.1.max(support.to.1) + reach,
        ));
        for cx in lo.0..=hi.0 {
            for cy in lo.1..=hi.1 {
                let Some(candidates) = grid.get(&(cx, cy)) else {
                    continue;
                };
                for &index in candidates {
                    let body = &lines[index];
                    if body.object == support.object {
                        continue;
                    }
                    let gap = segment_distance(support.from, support.to, body.from, body.to)
                        - (support.width + body.width) * 0.5;
                    if gap < TOUCH_EPSILON_MM {
                        let entry = found
                            .entry((support.object, body.object))
                            .or_insert(f64::INFINITY);
                        *entry = entry.min(gap);
                    }
                }
            }
        }
    }
    found
        .into_iter()
        .map(|((support, body), gap)| (support, body, gap))
        .collect()
}

/// Parses a sliced plate once: support-vs-foreign-body collisions on every layer plus every
/// object's first-layer lines (for learning the realized support patch).
pub fn analyze_plate(gcode: &[u8]) -> PlateAnalysis {
    let mut parser = Parser {
        width: 0.45,
        ..Parser::default()
    };
    for raw in gcode.split(|&byte| byte == b'\n') {
        // Only comments and moves matter; both are ASCII, so a lossy view is exact for them.
        parser.line(&String::from_utf8_lossy(raw));
    }
    parser.finish_layer();
    PlateAnalysis {
        collisions: parser.collisions.into_values().collect(),
        first_layer: parser.first_layer,
        layers: parser.layer,
        labelled_lines: parser.labelled_lines,
    }
}

/// Farthest an object's first-layer body centre may sit from its placement's expected body
/// centre and still be matched to it. Neighbouring goats are ~40 mm apart.
const MATCH_TOLERANCE_MM: f64 = 5.0;

/// Widens `footprint`'s reserved patch to what a sliced plate really printed: every object's
/// first-layer lines, matched to its placement by the first-layer body centre and moved into
/// the footprint frame, are unioned into `loops` and `bed_loops`. Body, silhouette and landing
/// zones stay as they are — only the support reach grows. Errors when no object can be
/// matched, i.e. the G-code does not describe this layout.
pub fn learn_support_patch(
    footprint: &Footprint,
    placements: &[PackedPlacement],
    first_layer: &[ObjectFirstLayer],
) -> Result<Footprint, String> {
    // Where the first-layer body centre sits in the footprint frame (the landing zone, or the
    // body/patch when the footprint has none).
    let reference = [
        &footprint.support_landing_loops,
        &footprint.body_loops,
        &footprint.loops,
    ]
    .into_iter()
    .find_map(|loops| crate::plate_centre::loops_bounds(loops))
    .ok_or("footprint has no loops")?;
    let offset = (
        (reference.min_x + reference.max_x) * 0.5,
        (reference.min_y + reference.max_y) * 0.5,
    );
    let to_frame = |placement: &PackedPlacement, (x, y): (f64, f64)| {
        let (sin, cos) = (-placement.angle_deg).to_radians().sin_cos();
        let (dx, dy) = (x - placement.x_mm, y - placement.y_mm);
        (dx * cos - dy * sin, dx * sin + dy * cos)
    };
    let mut polygons: Vec<Polygon> = footprint
        .loops
        .iter()
        .chain(footprint.bed_loops.iter().flatten())
        .map(|loop_| crate::polygon_from_loop(loop_))
        .collect();
    let mut matched = 0;
    for object in first_layer {
        let Some(centre) = object.body_center() else {
            continue;
        };
        let Some(placement) = placements
            .iter()
            .map(|placement| {
                let (sin, cos) = placement.angle_deg.to_radians().sin_cos();
                let expected = (
                    placement.x_mm + offset.0 * cos - offset.1 * sin,
                    placement.y_mm + offset.0 * sin + offset.1 * cos,
                );
                (placement, (centre.0 - expected.0).hypot(centre.1 - expected.1))
            })
            .filter(|(_, distance)| *distance <= MATCH_TOLERANCE_MM)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(placement, _)| placement)
        else {
            continue;
        };
        matched += 1;
        for strip in object.strips.iter().filter(|strip| strip.support) {
            let from = to_frame(placement, strip.from);
            let to = to_frame(placement, strip.to);
            polygons.push(crate::strip_polygon(from.0, from.1, to.0, to.1, strip.width));
        }
    }
    if matched == 0 {
        return Err("no sliced object matches a placement; cannot learn the support patch".into());
    }
    let loops = crate::union_exterior_loops(&polygons);
    let bounds = crate::plate_centre::loops_bounds(&loops).ok_or("learned patch is empty")?;
    let mut learned = footprint.clone();
    learned.width = bounds.max_x - bounds.min_x;
    learned.height = bounds.max_y - bounds.min_y;
    learned.bed_loops = Some(loops.clone());
    learned.loops = loops;
    Ok(learned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bambu_plate(support_x: f64) -> String {
        // Object 1: a body wall along x = 0..10 at y = 0 on layers 1 and 2. Object 2: a support
        // line parallel to it at y = support_x on layer 2 only, plus its own body far away.
        format!(
            "; CHANGE_LAYER\n\
             ; start printing object, unique label id: 11\n\
             ; FEATURE: Outer wall\n; LINE_WIDTH: 0.45\n\
             G1 X0 Y0 F600\nG1 X10 Y0 E0.5\n\
             ; stop printing object, unique label id: 11\n\
             ; start printing object, unique label id: 22\n\
             ; FEATURE: Outer wall\nG1 X0 Y20\nG1 X10 Y20 E0.5\n\
             ; stop printing object, unique label id: 22\n\
             ; CHANGE_LAYER\n\
             ; start printing object, unique label id: 11\n\
             ; FEATURE: Outer wall\nG1 X0 Y0\nG1 X10 Y0 E0.5\n\
             ; stop printing object, unique label id: 11\n\
             ; start printing object, unique label id: 22\n\
             ; FEATURE: Support\n; LINE_WIDTH: 0.4\n\
             G1 X0 Y{support_x}\nG1 X10 Y{support_x} E0.5\n\
             ; stop printing object, unique label id: 22\n"
        )
    }

    #[test]
    fn touching_support_on_a_foreign_body_is_reported_with_layer_and_gap() {
        // Centre lines 0.3 apart, half widths 0.225 + 0.2: overlap by 0.125.
        let analysis = analyze_plate(bambu_plate(0.3).as_bytes());
        assert_eq!(analysis.layers, 2);
        assert_eq!(analysis.collisions.len(), 1);
        let collision = &analysis.collisions[0];
        assert_eq!(collision.support_object, "22");
        assert_eq!(collision.body_object, "11");
        assert_eq!((collision.first_layer, collision.last_layer, collision.layers), (2, 2, 1));
        assert!((collision.min_gap_mm + 0.125).abs() < 1e-9, "{}", collision.min_gap_mm);
    }

    #[test]
    fn a_support_clear_of_foreign_bodies_is_not_reported() {
        let analysis = analyze_plate(bambu_plate(0.5).as_bytes());
        assert!(analysis.collisions.is_empty());
        assert!(analysis.labelled_lines > 0);
    }

    #[test]
    fn own_support_touching_own_body_is_fine() {
        let gcode = "; CHANGE_LAYER\n; start printing object, unique label id: 1\n\
                     ; FEATURE: Outer wall\nG1 X0 Y0\nG1 X10 Y0 E0.5\n\
                     ; FEATURE: Support\nG1 X0 Y0.1\nG1 X10 Y0.1 E0.5\n\
                     ; stop printing object, unique label id: 1\n";
        assert!(analyze_plate(gcode.as_bytes()).collisions.is_empty());
    }

    #[test]
    fn orca_labels_types_and_arcs_are_parsed() {
        // A G3 quarter arc of object A's support sweeping across object B's straight wall.
        let gcode = ";LAYER_CHANGE\n\
                     ; printing object b.stl id:1 copy 0\n;TYPE:Outer wall\n;WIDTH:0.42\n\
                     G1 X0 Y5\nG1 X10 Y5 E0.4\n\
                     ; stop printing object b.stl id:1 copy 0\n\
                     ; printing object a.stl id:0 copy 0\n;TYPE:Support\n;WIDTH:0.42\n\
                     G1 X0 Y0\nG3 X5 Y5 I0 J5 E0.3\n\
                     ; stop printing object a.stl id:0 copy 0\n";
        let analysis = analyze_plate(gcode.as_bytes());
        assert_eq!(analysis.collisions.len(), 1);
        assert_eq!(analysis.collisions[0].support_object, "a.stl id:0 copy 0");
        assert_eq!(analysis.collisions[0].body_object, "b.stl id:1 copy 0");
        assert_eq!(analysis.first_layer.len(), 2);
        assert!(analysis.first_layer[1].strips.iter().all(|strip| strip.support));
    }

    #[test]
    fn unretractions_and_unlabelled_lines_are_ignored() {
        let gcode = "; CHANGE_LAYER\nG1 X0 Y0\nG1 X10 Y0 E0.5\n\
                     ; start printing object, unique label id: 1\n; FEATURE: Support\n\
                     G1 X0 Y0.1\nG1 E0.8\n\
                     ; stop printing object, unique label id: 1\n";
        let analysis = analyze_plate(gcode.as_bytes());
        assert!(analysis.collisions.is_empty());
        assert_eq!(analysis.labelled_lines, 0);
    }

    #[test]
    fn first_layer_body_center_ignores_supports() {
        let analysis = analyze_plate(bambu_plate(0.3).as_bytes());
        let object = &analysis.first_layer[0];
        assert_eq!(object.label, "11");
        assert_eq!(object.body_center(), Some((5.0, 0.0)));
    }

    fn square(half: f64) -> Vec<crate::Point> {
        let mut ring: Vec<crate::Point> = [(-half, -half), (half, -half), (half, half), (-half, half)]
            .iter()
            .map(|&(x, y)| crate::Point { x, y })
            .collect();
        ring.push(ring[0]);
        ring
    }

    #[test]
    fn learned_patch_reaches_the_printed_support_of_the_matched_object() {
        let footprint = Footprint {
            loops: vec![square(2.0)],
            body_loops: vec![square(1.5)],
            support_landing_loops: vec![square(1.0)],
            bed_loops: Some(vec![square(2.0)]),
            width: 4.0,
            height: 4.0,
            min_z: 0.0,
            max_z: 1.0,
            triangle_count: 2,
        };
        let placements = [
            PackedPlacement { x_mm: 10.0, y_mm: 10.0, angle_deg: 0.0 },
            PackedPlacement { x_mm: 40.0, y_mm: 10.0, angle_deg: 0.0 },
        ];
        let object = ObjectFirstLayer {
            label: "7".into(),
            strips: vec![
                Strip { from: (9.0, 10.0), to: (11.0, 10.0), width: 0.4, support: false },
                Strip { from: (10.0, 13.0), to: (12.0, 13.0), width: 0.4, support: true },
            ],
        };
        let learned = learn_support_patch(&footprint, &placements, &[object]).unwrap();
        let bounds = crate::plate_centre::loops_bounds(&learned.loops).unwrap();
        assert!((bounds.max_y - 3.2).abs() < 1e-6, "{bounds:?}");
        assert!((bounds.min_y + 2.0).abs() < 1e-6, "{bounds:?}");
        assert_eq!(learned.support_landing_loops, footprint.support_landing_loops);
        assert_eq!(learned.body_loops, footprint.body_loops);
        assert_eq!(learned.bed_loops.as_ref(), Some(&learned.loops));
        // Nothing within reach of a placement: the G-code is not this layout.
        let stray = ObjectFirstLayer {
            label: "8".into(),
            strips: vec![Strip { from: (99.0, 99.0), to: (100.0, 99.0), width: 0.4, support: false }],
        };
        assert!(learn_support_patch(&footprint, &placements, &[stray]).is_err());
    }
}

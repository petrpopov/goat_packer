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

use crate::footprint::{Footprint, LayerZone};
use crate::gcode_text::{
    ArcSweep, DENSITY_CHECK_MIN_LENGTH_MM, MIN_FLOW_PER_MM, axis_value, feature_label,
    is_layer_change, line_width_comment,
};
use crate::geometry::{Rect, polygon_from_loop, strip_polygon, union_exterior_loops};
use crate::options::PackedPlacement;

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
    /// Every object's printed extent over all layers: line centres, arcs sampled along the
    /// sweep — what BambuStudio's G-code viewer measures (`all_paths_inside`: the paths' bbox,
    /// arc interpolation points included, line widths not).
    pub extents: Vec<(String, Rect)>,
}

impl PlateAnalysis {
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
    extents: Vec<Option<Rect>>,
    z: f64,
    /// Every labelled line with its layer height, kept only for `layer_zones_from_sample`.
    keep_lines: bool,
    kept: Vec<KeptLine>,
}

/// A labelled printed line of any layer (`Parser::keep_lines`).
struct KeptLine {
    object: usize,
    z: f64,
    strip: Strip,
    infill: bool,
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
            extent.min_x = extent.min_x.min(x);
            extent.min_y = extent.min_y.min(y);
            extent.max_x = extent.max_x.max(x);
            extent.max_y = extent.max_y.max(y);
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
    let parser = parse(gcode, false);
    let extents = extents_of(&parser);
    PlateAnalysis {
        extents,
        collisions: parser.collisions.into_values().collect(),
        first_layer: parser.first_layer,
        layers: parser.layer,
        labelled_lines: parser.labelled_lines,
    }
}

fn extents_of(parser: &Parser) -> Vec<(String, Rect)> {
    parser
        .labels
        .iter()
        .zip(&parser.extents)
        .filter_map(|(label, extent)| Some((label.clone(), (*extent)?)))
        .collect()
}

fn parse(gcode: &[u8], keep_lines: bool) -> Parser {
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

/// Height of one `Footprint::layer_zones` band. A band unions what its layers print, so a
/// support and a body in the same band but on different layers count as meeting: coarser
/// bands are safer and cost a little density; 2 mm keeps a 45 mm goat at ~23 bands.
pub const LAYER_BAND_MM: f64 = 2.0;

/// `Footprint::layer_zones` from a sliced sample of the plate (a few of its objects at their
/// placements): every matched object's lines of each `LAYER_BAND_MM` band, moved into the
/// footprint frame — supports from all of them (the sample objects print the same supports to
/// ~0.1 mm; the union absorbs the rest), the body from the first one (bodies are identical;
/// infill is left out, the walls around it bound the band's body ring). Also returns the
/// sample's analysis for learning the first-layer patch.
pub fn layer_zones_from_sample(
    gcodes: &[&[u8]],
    footprint: &Footprint,
    placements: &[PackedPlacement],
) -> Result<(Vec<LayerZone>, PlateAnalysis), String> {
    // Several slices of the same sample: tree supports differ between runs (see
    // `crate::first_layer_footprint_of`), so every run's supports join the bands and the
    // analysis carries every run's first layer for `learn_support_patch`.
    let mut runs = Vec::with_capacity(gcodes.len());
    for gcode in gcodes {
        let parser = parse(gcode, true);
        let matched = match_placements(footprint, placements, &parser.first_layer)?;
        if matched.is_empty() {
            return Err("no sample object matches a placement".into());
        }
        let index_of: HashMap<usize, usize> = parser
            .labels
            .iter()
            .enumerate()
            .filter_map(|(object, label)| Some((object, *matched.get(label)?)))
            .collect();
        runs.push((parser, index_of));
    }
    let analysis = PlateAnalysis {
        collisions: runs
            .iter()
            .flat_map(|(parser, _)| parser.collisions.values().cloned())
            .collect(),
        first_layer: runs
            .iter()
            .flat_map(|(parser, _)| parser.first_layer.iter().cloned())
            .collect(),
        layers: runs
            .iter()
            .map(|(parser, _)| parser.layer)
            .max()
            .unwrap_or(0),
        labelled_lines: runs.iter().map(|(parser, _)| parser.labelled_lines).sum(),
        extents: runs
            .iter()
            .flat_map(|(parser, _)| extents_of(parser))
            .collect(),
    };
    let to_frame = |placement: &PackedPlacement, (x, y): (f64, f64)| {
        let (sin, cos) = (-placement.angle_deg).to_radians().sin_cos();
        let (dx, dy) = (x - placement.x_mm, y - placement.y_mm);
        (dx * cos - dy * sin, dx * sin + dy * cos)
    };
    let mut bands: BTreeMap<usize, (Vec<Polygon>, Vec<Polygon>)> = BTreeMap::new();
    for (run, (parser, index_of)) in runs.iter().enumerate() {
        // The body comes from one object of the first run: bodies don't vary.
        let body_object = *index_of.keys().min().expect("matched is not empty");
        for line in &parser.kept {
            let Some(&index) = index_of.get(&line.object) else {
                continue;
            };
            if !line.strip.support && (run != 0 || line.object != body_object || line.infill) {
                continue;
            }
            let placement = &placements[index];
            let (from, to) = (
                to_frame(placement, line.strip.from),
                to_frame(placement, line.strip.to),
            );
            let polygon = strip_polygon(from.0, from.1, to.0, to.1, line.strip.width);
            let band = bands
                .entry((line.z.max(0.0) / LAYER_BAND_MM) as usize)
                .or_default();
            if line.strip.support {
                band.0.push(polygon);
            } else {
                band.1.push(polygon);
            }
        }
    }
    let zones = bands
        .into_values()
        .filter(|(support, body)| !support.is_empty() && !body.is_empty())
        .map(|(support, body)| LayerZone {
            support_loops: union_exterior_loops(&support),
            body_loops: union_exterior_loops(&body),
        })
        .collect();
    Ok((zones, analysis))
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
    let to_frame = |placement: &PackedPlacement, (x, y): (f64, f64)| {
        let (sin, cos) = (-placement.angle_deg).to_radians().sin_cos();
        let (dx, dy) = (x - placement.x_mm, y - placement.y_mm);
        (dx * cos - dy * sin, dx * sin + dy * cos)
    };
    let mut polygons: Vec<Polygon> = footprint
        .loops
        .iter()
        .chain(footprint.bed_loops.iter().flatten())
        .map(|loop_| polygon_from_loop(loop_))
        .collect();
    let matched = match_placements(footprint, placements, first_layer)?;
    if matched.is_empty() {
        return Err("no sliced object matches a placement; cannot learn the support patch".into());
    }
    for object in first_layer {
        let Some(&index) = matched.get(&object.label) else {
            continue;
        };
        let placement = &placements[index];
        for strip in object.strips.iter().filter(|strip| strip.support) {
            let from = to_frame(placement, strip.from);
            let to = to_frame(placement, strip.to);
            polygons.push(strip_polygon(from.0, from.1, to.0, to.1, strip.width));
        }
    }
    let loops = union_exterior_loops(&polygons);
    let bounds = crate::plate_centre::loops_bounds(&loops).ok_or("learned patch is empty")?;
    let mut learned = footprint.clone();
    learned.width = bounds.max_x - bounds.min_x;
    learned.height = bounds.max_y - bounds.min_y;
    learned.bed_loops = Some(loops.clone());
    learned.loops = loops;
    Ok(learned)
}

/// Sliced object label → index of its placement, matched by the first-layer body centre
/// (within `MATCH_TOLERANCE_MM` of where the placement puts the footprint's landing zone, or
/// its body/patch when it has none). Objects without a body on the first layer or without a
/// placement that close are left out.
pub fn match_placements(
    footprint: &Footprint,
    placements: &[PackedPlacement],
    first_layer: &[ObjectFirstLayer],
) -> Result<HashMap<String, usize>, String> {
    // Where the first-layer body centre sits in the footprint frame.
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
    let mut matched = HashMap::new();
    for object in first_layer {
        let Some(centre) = object.body_center() else {
            continue;
        };
        let nearest = placements
            .iter()
            .enumerate()
            .map(|(index, placement)| {
                let (sin, cos) = placement.angle_deg.to_radians().sin_cos();
                let expected = (
                    placement.x_mm + offset.0 * cos - offset.1 * sin,
                    placement.y_mm + offset.0 * sin + offset.1 * cos,
                );
                (index, (centre.0 - expected.0).hypot(centre.1 - expected.1))
            })
            .filter(|(_, distance)| *distance <= MATCH_TOLERANCE_MM)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((index, _)) = nearest {
            matched.insert(object.label.clone(), index);
        }
    }
    Ok(matched)
}

/// Extra distance a nudge adds on top of the measured overlap: the slicer regenerates the
/// supports on the nudged plate, and their reach drifts by a few tenths of a millimetre.
pub const NUDGE_MARGIN_MM: f64 = 0.3;

/// Directions tried for one contact, as offsets from the line between the two objects.
const NUDGE_ANGLES_DEG: [f64; 7] = [0.0, 30.0, -30.0, 60.0, -60.0, 90.0, -90.0];

/// Moves touching objects apart instead of repacking the plate: for each contact (support of
/// `contacts[i].0` touching the body of `.1`, smallest gap `.2` mm, negative = overlap) the pair
/// separates by the overlap plus `NUDGE_MARGIN_MM`, along the line between the two objects or
/// up to 90° off it, by the first move that keeps the whole layout `valid` (the packer's own
/// rule: neighbours, bed, keepouts, tower). Moves, in order: one of the two objects alone, both
/// by half, then everything on one side of the pair (the object and every placement beyond it
/// in that direction) — dense plates touch in chains, where a lone object only runs into the
/// next one. Pairs are handled once (both directions, smallest gap), and a pair an earlier move
/// already separated enough is skipped. `None` when a contact finds no valid move: the caller
/// falls back to a repack.
pub fn nudge_apart(
    placements: &[PackedPlacement],
    contacts: &[(usize, usize, f64)],
    valid: impl Fn(&[PackedPlacement]) -> bool,
) -> Option<Vec<PackedPlacement>> {
    let mut pairs: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for &(a, b, gap) in contacts {
        if a == b || a >= placements.len() || b >= placements.len() {
            return None;
        }
        let entry = pairs.entry((a.min(b), a.max(b))).or_insert(gap);
        *entry = entry.min(gap);
    }
    let offset = |layout: &[PackedPlacement], a: usize, b: usize| {
        (
            layout[b].x_mm - layout[a].x_mm,
            layout[b].y_mm - layout[a].y_mm,
        )
    };
    let mut current = placements.to_vec();
    for ((a, b), gap) in pairs {
        let need = (TOUCH_EPSILON_MM - gap).max(0.0) + NUDGE_MARGIN_MM;
        let (ox, oy) = offset(placements, a, b);
        let length = ox.hypot(oy);
        let (ux, uy) = if length > 1e-9 {
            (ox / length, oy / length)
        } else {
            (1.0, 0.0)
        };
        // Separation already gained along the pair's original direction.
        let (cx, cy) = offset(&current, a, b);
        if (cx - ox) * ux + (cy - oy) * uy >= need - 1e-9 {
            continue;
        }
        let base = uy.atan2(ux);
        let moved = NUDGE_ANGLES_DEG.iter().find_map(|degrees| {
            let angle = base + degrees.to_radians();
            let (dx, dy) = (angle.cos(), angle.sin());
            let side = |layout: &[PackedPlacement], pivot: usize, sign: f64| -> Vec<usize> {
                (0..layout.len())
                    .filter(|&index| {
                        let along = (layout[index].x_mm - layout[pivot].x_mm) * dx
                            + (layout[index].y_mm - layout[pivot].y_mm) * dy;
                        index == pivot || along * sign > 1e-9
                    })
                    .collect()
            };
            // (objects moved forward along d, objects moved back, distance each).
            let moves: [(Vec<usize>, Vec<usize>, f64); 5] = [
                (vec![b], vec![], need),
                (vec![], vec![a], need),
                (vec![b], vec![a], need * 0.5),
                (side(&current, b, 1.0), vec![], need),
                (vec![], side(&current, a, -1.0), need),
            ];
            moves.into_iter().find_map(|(forward, back, distance)| {
                if forward.contains(&a) || back.contains(&b) {
                    return None;
                }
                let mut candidate = current.clone();
                for &index in &forward {
                    candidate[index].x_mm += dx * distance;
                    candidate[index].y_mm += dy * distance;
                }
                for &index in &back {
                    candidate[index].x_mm -= dx * distance;
                    candidate[index].y_mm -= dy * distance;
                }
                valid(&candidate).then_some(candidate)
            })
        })?;
        current = moved;
    }
    Some(current)
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

    fn at(x: f64, y: f64) -> PackedPlacement {
        PackedPlacement {
            x_mm: x,
            y_mm: y,
            angle_deg: 0.0,
        }
    }

    #[test]
    fn nudge_moves_the_body_object_away_along_the_pair_line() {
        let placements = [at(0.0, 0.0), at(40.0, 0.0)];
        let moved = nudge_apart(&placements, &[(0, 1, -0.2)], |_| true).unwrap();
        assert_eq!((moved[0].x_mm, moved[0].y_mm), (0.0, 0.0));
        let expected = 40.0 + 0.2 + TOUCH_EPSILON_MM + NUDGE_MARGIN_MM;
        assert!((moved[1].x_mm - expected).abs() < 1e-9, "{:?}", moved[1]);
        assert!(moved[1].y_mm.abs() < 1e-9);
    }

    #[test]
    fn nudge_moves_the_support_object_when_the_other_cannot_move() {
        // Object 1 sits against the bed edge at x = 40: only object 0 may move.
        let placements = [at(0.0, 0.0), at(40.0, 0.0)];
        let moved = nudge_apart(&placements, &[(0, 1, -0.1)], |layout| {
            layout[1].x_mm <= 40.0
        })
        .unwrap();
        assert_eq!(moved[1].x_mm, 40.0);
        assert!(moved[0].x_mm < -0.1, "{:?}", moved[0]);
    }

    #[test]
    fn nudge_handles_a_pair_once_and_gives_up_without_a_valid_move() {
        let placements = [at(0.0, 0.0), at(40.0, 0.0)];
        let moved = nudge_apart(&placements, &[(0, 1, -0.1), (1, 0, -0.3)], |_| true).unwrap();
        let expected = 40.0 + 0.3 + TOUCH_EPSILON_MM + NUDGE_MARGIN_MM;
        assert!((moved[1].x_mm - expected).abs() < 1e-9, "{:?}", moved[1]);
        assert!(nudge_apart(&placements, &[(0, 1, -0.1)], |_| false).is_none());
    }

    #[test]
    fn nudge_pushes_a_chain_when_a_lone_object_would_hit_the_next() {
        // A row 0 - 1 - 2 along x, 40 mm apart; 0 touches 1, and 1 may not come within 39.9 of
        // anything: only shifting 1 and 2 together keeps the row valid.
        let placements = [at(0.0, 0.0), at(40.0, 0.0), at(80.0, 0.0)];
        let valid = |layout: &[PackedPlacement]| {
            let far_enough = |i: usize, j: usize| {
                (layout[i].x_mm - layout[j].x_mm).hypot(layout[i].y_mm - layout[j].y_mm) >= 39.9
            };
            far_enough(0, 1)
                && far_enough(1, 2)
                && far_enough(0, 2)
                && layout[0].x_mm.abs() < 1e-9
                && layout.iter().all(|p| p.y_mm.abs() < 1e-9)
        };
        let moved = nudge_apart(&placements, &[(0, 1, -0.2)], valid).unwrap();
        let need = 0.2 + TOUCH_EPSILON_MM + NUDGE_MARGIN_MM;
        assert!((moved[1].x_mm - (40.0 + need)).abs() < 1e-9, "{moved:?}");
        assert!((moved[2].x_mm - (80.0 + need)).abs() < 1e-9, "{moved:?}");
    }

    #[test]
    fn nudge_skips_a_pair_an_earlier_move_already_separated() {
        // 0-1 and 0-2 touch; shifting the side beyond 1 also carries 2 away from 0.
        let placements = [at(0.0, 0.0), at(40.0, 0.0), at(80.0, 0.0)];
        let moved = nudge_apart(&placements, &[(0, 1, -0.2), (0, 2, -0.1)], |layout| {
            layout[1].x_mm > 40.0 && (layout[2].x_mm - layout[1].x_mm) >= 39.99
        })
        .unwrap();
        assert!(
            (moved[2].x_mm - moved[1].x_mm - 40.0).abs() < 1e-9,
            "{moved:?}"
        );
    }

    #[test]
    fn layer_zones_come_from_the_sample_in_the_footprint_frame() {
        // Two objects at x = 0 and x = 50 (footprint frame centred on the body). Layer 1
        // (z 0.2) bodies only; layer 2 (z 2.4, band 1) a wall and a support 4..5 mm to the right (strips carry no end caps).
        let object = |id: u32, x: f64, support: bool| {
            let mut text = format!(
                "; start printing object, unique label id: {id}\n; FEATURE: Outer wall\n\
                 ; LINE_WIDTH: 0.4\nG1 X{} Y-1\nG1 X{} Y-1 E1\nG1 X{} Y1 E1\nG1 X{} Y1 E1\n",
                x - 1.0,
                x + 1.0,
                x + 1.0,
                x - 1.0
            );
            if support {
                text += &format!(
                    "; FEATURE: Support interface\nG1 X{} Y0\nG1 X{} Y0 E1\n",
                    x + 4.0,
                    x + 5.0
                );
            }
            text + &format!("; stop printing object, unique label id: {id}\n")
        };
        let gcode = format!(
            "; CHANGE_LAYER\n; Z_HEIGHT: 0.2\n{}{}; CHANGE_LAYER\n; Z_HEIGHT: 2.4\n{}{}",
            object(7, 0.0, false),
            object(8, 50.0, false),
            object(7, 0.0, true),
            object(8, 50.0, true)
        );
        let square = |half: f64| {
            vec![vec![
                crate::Point { x: -half, y: -half },
                crate::Point { x: half, y: -half },
                crate::Point { x: half, y: half },
                crate::Point { x: -half, y: half },
                crate::Point { x: -half, y: -half },
            ]]
        };
        let footprint = Footprint {
            loops: square(1.0),
            body_loops: Vec::new(),
            support_landing_loops: Vec::new(),
            bed_loops: None,
            layer_zones: Vec::new(),
            width: 2.0,
            height: 2.0,
            min_z: 0.0,
            max_z: 3.0,
            triangle_count: 0,
        };
        let placements = [
            PackedPlacement {
                x_mm: 0.0,
                y_mm: 0.0,
                angle_deg: 0.0,
            },
            PackedPlacement {
                x_mm: 50.0,
                y_mm: 0.0,
                angle_deg: 0.0,
            },
        ];
        let (zones, analysis) =
            layer_zones_from_sample(&[gcode.as_bytes()], &footprint, &placements).unwrap();
        assert_eq!(analysis.layers, 2);
        // Band 0 has no support; band 1 has both.
        assert_eq!(zones.len(), 1);
        let support = crate::plate_centre::loops_bounds(&zones[0].support_loops).unwrap();
        assert!(
            (support.min_x - 4.0).abs() < 1e-6 && (support.max_x - 5.0).abs() < 1e-6,
            "{support:?}"
        );
        let body = crate::plate_centre::loops_bounds(&zones[0].body_loops).unwrap();
        // Three sides of the square: bottom, right, top (no closing left side).
        assert!(
            (body.min_x + 1.0).abs() < 1e-6 && (body.max_x - 1.2).abs() < 1e-6,
            "{body:?}"
        );
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
        assert_eq!(
            (
                collision.first_layer,
                collision.last_layer,
                collision.layers
            ),
            (2, 2, 1)
        );
        assert!(
            (collision.min_gap_mm + 0.125).abs() < 1e-9,
            "{}",
            collision.min_gap_mm
        );
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
        assert!(
            analysis.first_layer[1]
                .strips
                .iter()
                .all(|strip| strip.support)
        );
    }

    #[test]
    fn an_arc_bulging_past_the_plate_edge_is_off_plate() {
        // The P1S support base: a G3 whose end points sit at x = 255.6 but whose sweep bulges
        // to x ≈ 256.3 — BambuStudio's viewer samples the arc and refuses the plate.
        let gcode = "; CHANGE_LAYER\n\
                     ; start printing object, unique label id: 7\n; FEATURE: Support\n\
                     G1 X255.6 Y170\nG3 X255.6 Y180 I-17.5 J5 E0.9\n\
                     ; stop printing object, unique label id: 7\n\
                     ; start printing object, unique label id: 8\n; FEATURE: Outer wall\n\
                     G1 X100 Y100\nG1 X150 Y100 E1.0\n\
                     ; stop printing object, unique label id: 8\n";
        let analysis = analyze_plate(gcode.as_bytes());
        let off = analysis.off_plate(Rect::new(0.0, 0.0, 256.0, 256.0));
        assert_eq!(off.len(), 1, "{off:?}");
        assert_eq!(off[0].0, "7");
        assert!(
            off[0].1.max_x > 256.2 && off[0].1.max_x < 256.4,
            "{:?}",
            off[0].1
        );
        assert!(
            analysis
                .off_plate(Rect::new(0.0, 0.0, 257.0, 256.0))
                .is_empty()
        );
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
        let mut ring: Vec<crate::Point> =
            [(-half, -half), (half, -half), (half, half), (-half, half)]
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
            layer_zones: Vec::new(),
            bed_loops: Some(vec![square(2.0)]),
            width: 4.0,
            height: 4.0,
            min_z: 0.0,
            max_z: 1.0,
            triangle_count: 2,
        };
        let placements = [
            PackedPlacement {
                x_mm: 10.0,
                y_mm: 10.0,
                angle_deg: 0.0,
            },
            PackedPlacement {
                x_mm: 40.0,
                y_mm: 10.0,
                angle_deg: 0.0,
            },
        ];
        let object = ObjectFirstLayer {
            label: "7".into(),
            strips: vec![
                Strip {
                    from: (9.0, 10.0),
                    to: (11.0, 10.0),
                    width: 0.4,
                    support: false,
                },
                Strip {
                    from: (10.0, 13.0),
                    to: (12.0, 13.0),
                    width: 0.4,
                    support: true,
                },
            ],
        };
        let learned = learn_support_patch(&footprint, &placements, &[object]).unwrap();
        let bounds = crate::plate_centre::loops_bounds(&learned.loops).unwrap();
        assert!((bounds.max_y - 3.2).abs() < 1e-6, "{bounds:?}");
        assert!((bounds.min_y + 2.0).abs() < 1e-6, "{bounds:?}");
        assert_eq!(
            learned.support_landing_loops,
            footprint.support_landing_loops
        );
        assert_eq!(learned.body_loops, footprint.body_loops);
        assert_eq!(learned.bed_loops.as_ref(), Some(&learned.loops));
        // Nothing within reach of a placement: the G-code is not this layout.
        let stray = ObjectFirstLayer {
            label: "8".into(),
            strips: vec![Strip {
                from: (99.0, 99.0),
                to: (100.0, 99.0),
                width: 0.4,
                support: false,
            }],
        };
        assert!(learn_support_patch(&footprint, &placements, &[stray]).is_err());
    }
}

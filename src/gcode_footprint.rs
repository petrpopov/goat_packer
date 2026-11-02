//! The footprint a probe slice really printed: first-layer walls, supports and brim, and the
//! support interfaces above them, read from G-code as swept strips and unioned into loops.

mod realization;

use crate::footprint::Footprint;
use crate::gcode_text::{
    DENSITY_CHECK_MIN_LENGTH_MM, MIN_FLOW_PER_MM, MoveCommand, arc_of, axis_value, feature_label,
    is_layer_change, line_width_comment, move_command,
};
use crate::geometry::{Point, closed_loop, drop_specks, sort_largest_first, strip_polygon};
use geo::{Polygon, algorithm::bool_ops::unary_union};

pub use realization::{SlicedObject, realization_from_gcode, realizations_from_gcode};

/// Line width assumed until the G-code names one, mm.
const DEFAULT_LINE_WIDTH_MM: f64 = 0.5;

/// Chord length of a sampled arc, mm; well under a line width.
const ARC_CHORD_MM: f64 = 0.5;

/// Builds the true contact patch from a probe slice's first layer: body walls, infill,
/// supports and brim exactly as the slicer lays them down. The loops are centered on the
/// body's bbox center — walls run along the model surface, so that center equals the mesh
/// bbox center the placements are built against, keeping the patch and the placed model in
/// the same coordinate frame.
///
/// Both slicer dialects are parsed. OrcaSlicer marks layers with `;LAYER_CHANGE`, features
/// with `;TYPE:` and widths with `;WIDTH:`. Bambu Studio marks layers with
/// `; CHANGE_LAYER` + `; Z_HEIGHT:` (its `; layer #N` lines number the prime tower's own
/// empty-grid pattern inside the tower section and are NOT layer boundaries), features with
/// `; FEATURE:` and widths with `; LINE_WIDTH:`. Extrusions in `;TYPE:`/`; FEATURE:` blocks
/// mentioning "tower" are skipped (the prime tower is packed around via its own keepout), as
/// is everything between `; WIPE_TOWER_START` and `; WIPE_TOWER_END` — Bambu emits no
/// feature marker for the tower's empty-grid infill, only the section wrapper — and so are
/// tiny-E moves after a travel — those are un-retractions, not laid-down lines, and would
/// draw phantom strips across the plate. Extrusions before the first layer marker (machine
/// start gcode) never touch the patch. Returns `None` when the first layer holds no object
/// extrusions to union.
pub fn first_layer_footprint(gcode: &[u8], model_center: Option<(f64, f64)>) -> Option<Footprint> {
    first_layer_footprint_of(&[gcode], model_center)
}

/// `first_layer_footprint` of several slices of the same project, as the union of their lines.
/// BambuStudio builds tree supports in parallel and they differ between runs: five slices of
/// one P1S goat printed first-layer supports of 43.8x43.4 mm four times and 43.8x39.6 mm once
/// (a whole support foot missing); one core makes them identical, but the user's slicer GUI
/// slices on all cores again. The union covers the observed runs, not every possible future slice.
pub fn first_layer_footprint_of(
    gcodes: &[&[u8]],
    model_center: Option<(f64, f64)>,
) -> Option<Footprint> {
    let mut sink = StripSink::default();
    for gcode in gcodes {
        sink.absorb(parse_strips(gcode, Some(1), |_, feature| {
            Some(!feature.contains("support"))
        }));
    }
    let patch_bounds = sink.patch_bounds?;
    if sink.strips.is_empty() {
        return None;
    }
    let center = patch_origin(&sink, model_center);

    let full_polygons = sink.strips.iter().map(|(polygon, _)| polygon);
    let body_polygons = sink
        .strips
        .iter()
        .filter(|(_, is_body)| *is_body)
        .map(|(polygon, _)| polygon);
    // Only exterior rings: first-layer walls form a frame around the (unprinted) infill
    // interior, and an interior ring would make even-odd collision checks treat the middle of
    // a body as free space a neighbour could occupy.
    let mut loops = exterior_loops(unary_union(full_polygons), center);
    let mut body_loops = exterior_loops(unary_union(body_polygons), center);
    drop_specks(&mut loops);
    drop_specks(&mut body_loops);
    if loops.is_empty() {
        return None;
    }
    sort_largest_first(&mut loops);
    Some(Footprint {
        body_loops,
        support_landing_loops: Vec::new(),
        bed_loops: None, // the caller unions the silhouette in when overhangs demand it
        layer_zones: Vec::new(),
        loops,
        width: patch_bounds.2 - patch_bounds.0,
        height: patch_bounds.3 - patch_bounds.1,
        min_z: 0.0,
        max_z: 0.0,
        triangle_count: 0,
    })
}

/// The point the patch loops are centered on. It must share its origin with the model-bbox
/// center the assembler places clones by; for non-prism models the contact strip's own center
/// sits elsewhere (a lying animal's strip is low in its bbox), which silently shifts the
/// reserved area.
fn patch_origin(sink: &StripSink, model_center: Option<(f64, f64)>) -> (f64, f64) {
    if let Some(center) = model_center {
        return center;
    }
    let center_of =
        |(min_x, min_y, max_x, max_y): Bounds| ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5);
    match sink.body_bounds {
        Some(body) => center_of(body),
        // No body extrusions in layer 1 (a supports-only layer): fall back to the patch's
        // own bbox center and let the caller's clearance absorb the small mismatch.
        None => center_of(
            sink.patch_bounds
                .expect("patch bounds checked by the caller"),
        ),
    }
}

/// Exterior rings of a union, shifted so `origin` becomes (0, 0).
fn exterior_loops(union: geo::MultiPolygon, origin: (f64, f64)) -> Vec<Vec<Point>> {
    union
        .0
        .iter()
        .filter_map(|polygon| closed_loop(polygon.exterior().coords(), origin))
        .collect()
}

/// Top-down projection of every support-interface line above the first layer, as exterior
/// loops in the same frame as `first_layer_footprint(gcode, Some(model_center))`. Interfaces
/// are the tops of the supports, right under the model's overhangs, and the slicer prints them
/// up to ~2 mm past the overhang's outline (P1S goat probe: p99 1.5 mm, max 2.0 mm). Two
/// silhouettes 0.7 mm apart then meet at the overhang's height: on a 43-goat P1S plate four
/// pairs touched a neighbour's outer wall at z ≈ 23 mm with exactly these lines. Empty when the
/// G-code prints no interfaces.
pub fn support_interface_loops(gcode: &[u8], model_center: (f64, f64)) -> Vec<Vec<Point>> {
    support_interface_loops_of(&[gcode], model_center)
}

/// `support_interface_loops` of several slices of the same project (see
/// `first_layer_footprint_of` for why supports need more than one run).
pub fn support_interface_loops_of(gcodes: &[&[u8]], model_center: (f64, f64)) -> Vec<Vec<Point>> {
    let mut sink = StripSink::default();
    for gcode in gcodes {
        sink.absorb(parse_strips(gcode, None, |layer, feature| {
            (layer >= 2 && feature.contains("support interface")).then_some(false)
        }));
    }
    let union = unary_union(sink.strips.iter().map(|(polygon, _)| polygon));
    let mut loops = exterior_loops(union, model_center);
    drop_specks(&mut loops);
    loops
}

/// `(min_x, min_y, max_x, max_y)`.
type Bounds = (f64, f64, f64, f64);

/// Ink collected from layer 1: the strip polygons (`true` = body, `false` = support) plus the
/// patch/body bounds they reach. Straight G1 moves and the sampled points of a G2/G3 arc sweep
/// land here alike.
#[derive(Default)]
struct StripSink {
    strips: Vec<(Polygon, bool)>,
    patch_bounds: Option<Bounds>,
    body_bounds: Option<Bounds>,
}

impl StripSink {
    fn absorb(&mut self, other: StripSink) {
        self.strips.extend(other.strips);
        let merge = |a: Option<Bounds>, b: Option<Bounds>| match (a, b) {
            (Some(a), Some(b)) => Some(grow_bounds(Some(a), b.0, b.1, b.2, b.3)),
            (a, b) => a.or(b),
        };
        self.patch_bounds = merge(self.patch_bounds, other.patch_bounds);
        self.body_bounds = merge(self.body_bounds, other.body_bounds);
    }

    fn record(&mut self, from: (f64, f64), to: (f64, f64), line_width: f64, is_body: bool) {
        let (x0, y0) = from;
        let (x1, y1) = to;
        self.strips
            .push((strip_polygon(x0, y0, x1, y1, line_width), is_body));
        self.patch_bounds = Some(grow_bounds(self.patch_bounds, x0, y0, x1, y1));
        if is_body {
            self.body_bounds = Some(grow_bounds(self.body_bounds, x0, y0, x1, y1));
        }
    }
}

fn grow_bounds(bounds: Option<Bounds>, x0: f64, y0: f64, x1: f64, y1: f64) -> Bounds {
    let (min_x, min_y, max_x, max_y) = bounds.unwrap_or((
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ));
    (
        min_x.min(x0).min(x1),
        min_y.min(y0).min(y1),
        max_x.max(x0).max(x1),
        max_y.max(y0).max(y1),
    )
}

/// Every printed object line of `gcode` that `keep(layer, feature_lowercase)` accepts, as
/// strips (`Some(true)` = body, `Some(false)` = support, `None` = skipped); the parsing rules
/// are `first_layer_footprint`'s (dialects, tower sections, arcs, un-retractions). Stops after
/// `last_layer` when given.
fn parse_strips(
    gcode: &[u8],
    last_layer: Option<usize>,
    keep: impl Fn(usize, &str) -> Option<bool>,
) -> StripSink {
    let text = String::from_utf8_lossy(gcode);
    let mut reader = StripReader::new(last_layer, keep);
    for raw_line in text.lines() {
        if !reader.read_line(raw_line.trim()) {
            break;
        }
    }
    reader.sink
}

/// The G-code reading state: where the head is, which layer and feature it prints.
struct StripReader<Keep> {
    keep: Keep,
    last_layer: Option<usize>,
    x: f64,
    y: f64,
    layer: usize,
    /// Current extrusion type, lowercase.
    feature: String,
    line_width: f64,
    in_tower_section: bool,
    sink: StripSink,
}

impl<Keep: Fn(usize, &str) -> Option<bool>> StripReader<Keep> {
    fn new(last_layer: Option<usize>, keep: Keep) -> Self {
        Self {
            keep,
            last_layer,
            x: 0.0,
            y: 0.0,
            layer: 0,
            feature: String::new(),
            line_width: DEFAULT_LINE_WIDTH_MM,
            in_tower_section: false,
            sink: StripSink::default(),
        }
    }

    /// Reads one trimmed line; `false` once the layers of interest are over.
    fn read_line(&mut self, line: &str) -> bool {
        if is_layer_change(line) {
            self.layer += 1;
            return !self.last_layer.is_some_and(|last| self.layer > last);
        }
        if line.contains("WIPE_TOWER_END") {
            self.in_tower_section = false;
        } else if line.contains("WIPE_TOWER_START") {
            self.in_tower_section = true;
        } else if let Some(kind) = feature_label(line) {
            self.feature = kind.to_lowercase();
        } else if let Some(width) = line_width_comment(line) {
            self.line_width = width.unwrap_or(self.line_width);
        } else {
            match move_command(line) {
                Some(MoveCommand::Arc { clockwise }) => self.read_arc(line, clockwise),
                Some(MoveCommand::Linear) => self.read_straight_move(line),
                None => {}
            }
        }
        true
    }

    /// Whether a move that names X or Y lays down object material in the current state: not
    /// before the first layer marker, not in the prime tower, not machine G-code (`Custom`,
    /// e.g. a purge in a filament-change block).
    fn prints_object_line(&self, line: &str) -> bool {
        self.layer >= 1
            && !self.in_tower_section
            && !self.feature.contains("tower")
            && self.feature != "custom"
            && (axis_value(line, 'X').is_some() || axis_value(line, 'Y').is_some())
    }

    /// The head's new position after `line` (an unnamed axis stays where it was).
    fn target(&self, line: &str) -> (f64, f64) {
        (
            axis_value(line, 'X').unwrap_or(self.x),
            axis_value(line, 'Y').unwrap_or(self.y),
        )
    }

    fn read_straight_move(&mut self, line: &str) {
        let (next_x, next_y) = self.target(line);
        let filament = axis_value(line, 'E').unwrap_or(0.0);
        if filament > 0.0 && self.prints_object_line(line) {
            let length = (next_x - self.x).hypot(next_y - self.y);
            if length > DENSITY_CHECK_MIN_LENGTH_MM
                && filament / length > MIN_FLOW_PER_MM
                && let Some(is_body) = (self.keep)(self.layer, &self.feature)
            {
                self.sink
                    .record((self.x, self.y), (next_x, next_y), self.line_width, is_body);
            }
        }
        self.x = next_x;
        self.y = next_y;
    }

    /// Arc-fitted gcode (Bambu Studio's `enable_arc_fitting`) draws whole walls as G2/G3
    /// sweeps with an I/J centre. Skipping the line like any other comment froze the tracked
    /// position at the last G1, so the next G1 inked a phantom chord across whatever the arcs
    /// went around — on the magnet goat one 16.7 mm chord over the head put a single stray
    /// pixel past the mesh silhouette, `patch_within_silhouette` failed, and a bed that packs
    /// 18 rotated objects locked to one orientation (15). Follow the arc: track its endpoint
    /// and ink the sweep.
    fn read_arc(&mut self, line: &str, clockwise: bool) {
        let next = self.target(line);
        let filament = axis_value(line, 'E').unwrap_or(0.0);
        if filament > 0.0
            && self.prints_object_line(line)
            && let Some(is_body) = (self.keep)(self.layer, &self.feature)
            // An arc without a centre (I/J) or a radius (R) cannot be rebuilt; slicer output
            // always names one, so such a line only moves the head.
            && let Some(arc) = arc_of(line, (self.x, self.y), next, clockwise)
        {
            let length = arc.length();
            // Same density cut as straight moves, judged over the whole sweep: Arachne
            // thin-tip arcs print far under a normal line's flow but an order of magnitude
            // above a travel-smeared un-retraction.
            if length > DENSITY_CHECK_MIN_LENGTH_MM && filament / length > MIN_FLOW_PER_MM {
                let steps = ((length / ARC_CHORD_MM).ceil() as usize).max(2);
                let mut from = (self.x, self.y);
                for to in arc.points(steps) {
                    self.sink.record(from, to, self.line_width, is_body);
                    from = to;
                }
            }
        }
        (self.x, self.y) = next;
    }
}

//! Walking a sliced plate's G-code move by move: layers, print heights, features, line widths,
//! object labels, the tower sections and the head position, turned into one `Extrusion` per
//! printed move. The plate checks (`parser`, `gui_checks`) only look at extrusions; how the
//! slicer dialects spell things stays here.
//!
//! Object labels follow the plate's label scheme (`labels`). An `; OBJECT_ID:` label has no
//! stop marker, so it ends where the tower or machine G-code (`Custom`) starts. When such a
//! plate names exactly one object, every extrusion that is not tower, machine G-code or skirt
//! is that object's, labelled or not. Explicit labels mark every object line, so with them a
//! line outside a label is not an object's (skirt, purge).

use std::collections::BTreeMap;

use super::labels::{LabelLine, LabelScheme, label_line, label_scheme};
use crate::gcode_text::{
    DENSITY_CHECK_MIN_LENGTH_MM, MIN_FLOW_PER_MM, MoveCommand, arc_of, axis_value, feature_label,
    is_layer_change, line_width_comment, move_command,
};
use crate::geometry::{Point, Rect};

/// Chord length of a followed arc; well under a line width.
pub(super) const ARC_STEP_MM: f64 = 0.3;

/// Line width assumed until the G-code names one, mm.
const DEFAULT_LINE_WIDTH_MM: f64 = 0.45;

/// What a printed move is, from its feature and the section it sits in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Role {
    /// Machine G-code (start, filament change): no object's, and no slicer check counts it.
    Custom,
    Tower,
    /// The tower brim where the slicer marks it (Orca `; WIPE_TOWER_BRIM_START`); BambuStudio
    /// prints its brim and chamfer unmarked inside the tower section.
    TowerBrim,
    Skirt,
    Brim,
    Support,
    SupportInterface,
    Body {
        infill: bool,
    },
}

impl Role {
    fn of(feature: &str, tower_section: bool, tower_brim_section: bool) -> Self {
        if tower_brim_section {
            return Role::TowerBrim;
        }
        if tower_section || feature.contains("tower") {
            return Role::Tower;
        }
        if feature == "custom" {
            return Role::Custom;
        }
        if feature.contains("support") {
            return if feature.contains("interface") {
                Role::SupportInterface
            } else {
                Role::Support
            };
        }
        if feature.contains("skirt") {
            return Role::Skirt;
        }
        if feature.contains("brim") {
            return Role::Brim;
        }
        Role::Body {
            infill: feature.contains("infill"),
        }
    }

    pub(super) fn is_support(self) -> bool {
        matches!(self, Role::Support | Role::SupportInterface)
    }

    pub(super) fn is_tower(self) -> bool {
        matches!(self, Role::Tower | Role::TowerBrim)
    }

    /// Material of the plate rather than of an object: the tower, machine G-code, the skirt.
    fn belongs_to_plate(self) -> bool {
        self.is_tower() || matches!(self, Role::Custom | Role::Skirt)
    }
}

/// One printed move (filament fed while the head moves in X/Y).
pub(super) struct Extrusion<'a> {
    /// 1-based layer; 0 = before the first layer marker (machine start G-code).
    pub(super) layer: usize,
    /// The layer's print height (`; Z_HEIGHT:` / `;Z:`), 0 until the first one.
    pub(super) z: f64,
    pub(super) role: Role,
    /// Index into the plate's labels of the object printing it; `None` for plate material and
    /// unlabelled lines.
    pub(super) object: Option<usize>,
    pub(super) width: f64,
    /// The centre line: the start, then every chord end (arcs cut at `ARC_STEP_MM`).
    pub(super) path: &'a [(f64, f64)],
    /// Exact extent of the centre line, arc bulges included.
    pub(super) extent: Rect,
    /// Filament fed, mm of E.
    pub(super) filament: f64,
    pub(super) arc: bool,
}

impl Extrusion<'_> {
    pub(super) fn end(&self) -> (f64, f64) {
        *self.path.last().expect("a path has at least two points")
    }

    /// A straight move whose flow is far below any real line: an un-retraction smeared over a
    /// travel (`MIN_FLOW_PER_MM`), which the material checks ignore.
    pub(super) fn is_smear(&self) -> bool {
        let (from, to) = (self.path[0], self.end());
        let length = (to.0 - from.0).hypot(to.1 - from.1);
        !self.arc
            && length > DENSITY_CHECK_MIN_LENGTH_MM
            && self.filament / length <= MIN_FLOW_PER_MM
    }
}

/// What a plate check does with the moves of one read.
pub(super) trait ExtrusionSink {
    fn extrusion(&mut self, extrusion: &Extrusion);
    /// After the last move of `layer` (0 = the start G-code), before the next layer begins and
    /// once more at the end of the file.
    fn layer_done(&mut self, _layer: usize) {}
}

/// Reads `gcode` once, handing every printed move to `sink`. Returns the plate's object
/// labels; `Extrusion::object` indexes them.
pub(super) fn read(gcode: &[u8], sink: &mut impl ExtrusionSink) -> Vec<String> {
    let (scheme, object_ids) = label_scheme(gcode);
    let mut walk = Walk::new(scheme);
    if scheme == LabelScheme::BambuObjectId && object_ids.len() == 1 {
        let only = object_ids.into_iter().next().expect("one id");
        walk.sole = Some(walk.object(&only));
    }
    for raw in gcode.split(|&byte| byte == b'\n') {
        // Only comments and moves matter; both are ASCII, so a lossy view is exact for them.
        walk.line(String::from_utf8_lossy(raw).trim(), sink);
    }
    sink.layer_done(walk.layer);
    walk.labels
}

/// The reading state.
struct Walk {
    scheme: LabelScheme,
    labels: Vec<String>,
    index_of: BTreeMap<String, usize>,
    /// The object being printed under the plate's label scheme.
    label: Option<usize>,
    /// The only object of a `; OBJECT_ID:` plate: owns every unlabelled object line.
    sole: Option<usize>,
    layer: usize,
    z: f64,
    /// Current feature, lowercase.
    feature: String,
    width: f64,
    x: f64,
    y: f64,
    absolute_xy: bool,
    absolute_e: bool,
    last_e: f64,
    tower_section: bool,
    tower_brim_section: bool,
    path: Vec<(f64, f64)>,
}

impl Walk {
    fn new(scheme: LabelScheme) -> Self {
        Self {
            scheme,
            labels: Vec::new(),
            index_of: BTreeMap::new(),
            label: None,
            sole: None,
            layer: 0,
            z: 0.0,
            feature: String::new(),
            width: DEFAULT_LINE_WIDTH_MM,
            x: 0.0,
            y: 0.0,
            absolute_xy: true,
            absolute_e: false,
            last_e: 0.0,
            tower_section: false,
            tower_brim_section: false,
            path: Vec::new(),
        }
    }

    fn object(&mut self, label: &str) -> usize {
        if let Some(&index) = self.index_of.get(label) {
            return index;
        }
        let index = self.labels.len();
        self.labels.push(label.to_owned());
        self.index_of.insert(label.to_owned(), index);
        index
    }

    fn line(&mut self, line: &str, sink: &mut impl ExtrusionSink) {
        if is_layer_change(line) {
            sink.layer_done(self.layer);
            self.layer += 1;
        } else if let Some(z) = layer_height(line) {
            self.z = z;
        } else if line.starts_with(';') || line.starts_with("EXCLUDE_OBJECT") {
            self.comment(line);
        } else if let Some(command) = move_command(line) {
            self.read_move(line, command, sink);
        } else {
            self.mode(line);
        }
    }

    fn comment(&mut self, line: &str) {
        if self.label_line(line) {
            return;
        }
        let body = line.trim_start_matches(';').trim();
        if body.contains("WIPE_TOWER_BRIM_START") {
            self.tower_brim_section = true;
        } else if body.contains("WIPE_TOWER_BRIM_END") {
            self.tower_brim_section = false;
        } else if body.contains("WIPE_TOWER_START") {
            self.tower_section = true;
            self.end_weak_label();
        } else if body.contains("WIPE_TOWER_END") {
            self.tower_section = false;
        } else if let Some(feature) = feature_label(line) {
            self.feature = feature.to_lowercase();
            if self.feature == "custom" {
                self.end_weak_label();
            }
        } else if let Some(width) = line_width_comment(line) {
            self.width = width.unwrap_or(self.width);
        }
    }

    /// Reads an object label of the plate's scheme; `true` when `line` was one.
    fn label_line(&mut self, line: &str) -> bool {
        match label_line(self.scheme, line) {
            Some(LabelLine::Start(label)) => self.label = Some(self.object(label)),
            Some(LabelLine::Stop) => self.label = None,
            None => return false,
        }
        true
    }

    /// `; OBJECT_ID:` has no stop marker: the label ends where the tower or machine G-code
    /// starts.
    fn end_weak_label(&mut self) {
        if self.scheme == LabelScheme::BambuObjectId {
            self.label = None;
        }
    }

    fn mode(&mut self, line: &str) {
        match line.split_whitespace().next() {
            Some("G90") => self.absolute_xy = true,
            Some("G91") => self.absolute_xy = false,
            Some("M82") => self.absolute_e = true,
            Some("M83") => self.absolute_e = false,
            Some("G92") => {
                if let Some(e) = axis_value(line, 'E') {
                    self.last_e = e;
                }
            }
            _ => {}
        }
    }

    /// Filament fed by a move: relative E as written, absolute E as the rise over the last
    /// value (Bambu/Orca switch E with M82/M83 only, never with G90/G91).
    fn fed(&mut self, line: &str) -> f64 {
        let Some(e) = axis_value(line, 'E') else {
            return 0.0;
        };
        if !self.absolute_e {
            return e;
        }
        let fed = e - self.last_e;
        self.last_e = e;
        fed
    }

    fn read_move(&mut self, line: &str, command: MoveCommand, sink: &mut impl ExtrusionSink) {
        let (x, y) = (axis_value(line, 'X'), axis_value(line, 'Y'));
        let from = (self.x, self.y);
        let to = if self.absolute_xy {
            (x.unwrap_or(self.x), y.unwrap_or(self.y))
        } else {
            (self.x + x.unwrap_or(0.0), self.y + y.unwrap_or(0.0))
        };
        (self.x, self.y) = to;
        let filament = self.fed(line);
        // Un-retractions carry no X/Y; a move without filament is travel.
        if filament <= 0.0 || (x.is_none() && y.is_none()) || self.width <= 0.0 {
            return;
        }
        self.path.clear();
        self.path.push(from);
        let mut extent = Rect::of_points([point(from), point(to)].iter());
        let mut arc = false;
        if let MoveCommand::Arc { clockwise } = command
            && let Some(sweep) = arc_of(line, from, to, clockwise)
        {
            self.path.extend(sweep.chord_points(ARC_STEP_MM));
            extent = sweep.extent();
            arc = true;
        } else {
            self.path.push(to);
        }
        let role = Role::of(&self.feature, self.tower_section, self.tower_brim_section);
        let object = if role.belongs_to_plate() {
            None
        } else {
            self.label.or(self.sole)
        };
        sink.extrusion(&Extrusion {
            layer: self.layer,
            z: self.z,
            role,
            object,
            width: self.width,
            path: &self.path,
            extent,
            filament,
            arc,
        });
    }
}

fn point((x, y): (f64, f64)) -> Point {
    Point { x, y }
}

/// The print height named by `; Z_HEIGHT:` (Bambu) or `;Z:` (Orca).
fn layer_height(line: &str) -> Option<f64> {
    line.strip_prefix("; Z_HEIGHT:")
        .or_else(|| line.strip_prefix(";Z:"))
        .and_then(|value| value.trim().parse::<f64>().ok())
}

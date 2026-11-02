//! The G-code vocabulary shared by the readers of sliced plates (`gcode_footprint` for the
//! probe's first layer, `gcode_collisions` for a whole plate). Both slicer dialects are
//! understood: OrcaSlicer marks layers with `;LAYER_CHANGE`, features with `;TYPE:` and widths
//! with `;WIDTH:`; Bambu Studio uses `; CHANGE_LAYER`, `; FEATURE:` and `; LINE_WIDTH:`.

use crate::geometry::{Point, Rect};

/// Moves shorter than this are judged by their flow density (`MIN_FLOW_PER_MM`) against
/// un-retractions smeared over a travel; below it a move cannot be told apart from noise.
pub(crate) const DENSITY_CHECK_MIN_LENGTH_MM: f64 = 0.3;

/// Least filament (mm of E) per mm of travel that still counts as a printed line. A 0.5 x 0.25
/// line spends ~0.05 mm of filament per mm of travel, but Arachne thins real walls far below
/// that: the long tip lines of a P1S goat print at 0.001-0.004 filament-mm per mm — exactly
/// the segments that stick out past the mesh silhouette. Real Bambu/Orca un-retractions carry
/// no X/Y and are filtered separately; only a travel-smeared smear (measured ~0.0002/mm in the
/// fixture) stays below this cut, an order of magnitude under the real lines.
pub(crate) const MIN_FLOW_PER_MM: f64 = 0.001;

pub(crate) fn is_layer_change(line: &str) -> bool {
    line == ";LAYER_CHANGE" || line == "; CHANGE_LAYER"
}

/// The feature (extrusion type) named by a `;TYPE:` / `; FEATURE:` comment.
pub(crate) fn feature_label(line: &str) -> Option<&str> {
    line.strip_prefix(";TYPE:")
        .or_else(|| line.strip_prefix("; FEATURE:"))
        .or_else(|| line.strip_prefix(";FEATURE:"))
        .map(str::trim)
}

/// The line width in mm named by a `;WIDTH:` / `; LINE_WIDTH:` comment; `Some(None)` is a
/// width comment whose number does not parse (the caller keeps its previous width).
pub(crate) fn line_width_comment(line: &str) -> Option<Option<f64>> {
    line.strip_prefix(";WIDTH:")
        .or_else(|| line.strip_prefix("; LINE_WIDTH:"))
        .or_else(|| line.strip_prefix(";LINE_WIDTH:"))
        .map(|value| value.trim().parse::<f64>().ok())
}

/// Numeric value of an `X`/`Y`/`E`/`I`/`J` word of a move line (no sign restrictions); the
/// trailing `;` comment is ignored.
pub(crate) fn axis_value(line: &str, axis: char) -> Option<f64> {
    let code = line.split(';').next().unwrap_or("");
    code.split_whitespace()
        .skip(1)
        .find_map(|token| token.strip_prefix(axis).and_then(|v| v.parse::<f64>().ok()))
}

/// The kind of move a G-code line commands, judged by its first word (so `G17` or `G28` never
/// pass for `G1`/`G2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MoveCommand {
    Linear,
    /// G2 (`clockwise`) or G3.
    Arc {
        clockwise: bool,
    },
}

pub(crate) fn move_command(line: &str) -> Option<MoveCommand> {
    match line.split_whitespace().next()? {
        "G0" | "G1" | "G00" | "G01" => Some(MoveCommand::Linear),
        "G2" | "G02" => Some(MoveCommand::Arc { clockwise: true }),
        "G3" | "G03" => Some(MoveCommand::Arc { clockwise: false }),
        _ => None,
    }
}

/// The arc a G2/G3 line sweeps from `from` to `to`: centre form (`I`/`J` offsets from the start;
/// a missing one is 0) or radius form (`R`). `None` when the line names neither or the arc is
/// degenerate; callers then treat the move as its chord.
pub(crate) fn arc_of(
    line: &str,
    from: (f64, f64),
    to: (f64, f64),
    clockwise: bool,
) -> Option<ArcSweep> {
    let (i, j) = (axis_value(line, 'I'), axis_value(line, 'J'));
    if i.is_some() || j.is_some() {
        let offset = (i.unwrap_or(0.0), j.unwrap_or(0.0));
        return ArcSweep::new(from, to, offset, clockwise);
    }
    ArcSweep::from_radius(from, to, axis_value(line, 'R')?, clockwise)
}

/// Shortest radius of an arc worth following, mm.
const MIN_ARC_RADIUS_MM: f64 = 1e-9;

/// A G2/G3 arc rebuilt from its start, end and I/J centre offset.
pub(crate) struct ArcSweep {
    centre: (f64, f64),
    radius: f64,
    start_angle: f64,
    /// Signed angle swept, radians; negative = clockwise (G2).
    sweep: f64,
}

impl ArcSweep {
    /// `None` for a degenerate (zero-radius) arc. Slicers never emit full-turn arcs, so the
    /// sweep is unwrapped to |sweep| <= 2π: G2 sweeps clockwise (angle decreasing), G3
    /// counter-clockwise.
    pub(crate) fn new(
        from: (f64, f64),
        to: (f64, f64),
        centre_offset: (f64, f64),
        clockwise: bool,
    ) -> Option<Self> {
        let centre = (from.0 + centre_offset.0, from.1 + centre_offset.1);
        let radius = centre_offset.0.hypot(centre_offset.1);
        if radius <= MIN_ARC_RADIUS_MM {
            return None;
        }
        let start_angle = (from.1 - centre.1).atan2(from.0 - centre.0);
        let end_angle = (to.1 - centre.1).atan2(to.0 - centre.0);
        let mut sweep = end_angle - start_angle;
        if clockwise {
            if sweep >= 0.0 {
                sweep -= std::f64::consts::TAU;
            }
        } else if sweep <= 0.0 {
            sweep += std::f64::consts::TAU;
        }
        Some(Self {
            centre,
            radius,
            start_angle,
            sweep,
        })
    }

    /// Radius form: the circle of radius |`radius`| through both ends. A positive radius takes
    /// the arc of at most half a turn, a negative one the longer arc (the usual firmware
    /// reading). Moving clockwise along the shorter arc keeps the centre on the right of the
    /// chord, so the centre sits on that side for G2 with R > 0 and on the other side for the
    /// three other combinations. A radius shorter than half the chord cannot reach both ends;
    /// it is read as a half circle, the closest arc that does.
    pub(crate) fn from_radius(
        from: (f64, f64),
        to: (f64, f64),
        radius: f64,
        clockwise: bool,
    ) -> Option<Self> {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let chord = dx.hypot(dy);
        if chord <= MIN_ARC_RADIUS_MM || radius.abs() <= MIN_ARC_RADIUS_MM {
            return None;
        }
        let half = chord * 0.5;
        let rise = (radius * radius - half * half).max(0.0).sqrt();
        // Unit normal to the right of the chord direction.
        let right = (dy / chord, -dx / chord);
        let side = if clockwise == (radius > 0.0) {
            1.0
        } else {
            -1.0
        };
        let centre = (
            from.0 + dx * 0.5 + side * rise * right.0,
            from.1 + dy * 0.5 + side * rise * right.1,
        );
        Self::new(from, to, (centre.0 - from.0, centre.1 - from.1), clockwise)
    }

    pub(crate) fn length(&self) -> f64 {
        self.radius * self.sweep.abs()
    }

    /// The exact bounding box of the swept centre line: both ends plus every axis extreme
    /// (a multiple of a quarter turn) the sweep passes. Chord samples cut the bulge short by up
    /// to the sagitta; the extremes do not.
    pub(crate) fn extent(&self) -> Rect {
        let end_angle = self.start_angle + self.sweep;
        let point = |angle: f64| Point {
            x: self.centre.0 + self.radius * angle.cos(),
            y: self.centre.1 + self.radius * angle.sin(),
        };
        let (low, high) = if self.sweep < 0.0 {
            (end_angle, self.start_angle)
        } else {
            (self.start_angle, end_angle)
        };
        let quarter = std::f64::consts::FRAC_PI_2;
        let first = (low / quarter).ceil() as i64;
        let last = (high / quarter).floor() as i64;
        let extremes = (first..=last).map(|k| point(k as f64 * quarter));
        let points: Vec<Point> = [point(self.start_angle), point(end_angle)]
            .into_iter()
            .chain(extremes)
            .collect();
        Rect::of_points(points.iter())
    }

    /// The arc as chords no longer than `step_mm` (at least one): the chord end points after
    /// the start, in order.
    pub(crate) fn chord_points(&self, step_mm: f64) -> impl Iterator<Item = (f64, f64)> + '_ {
        let steps = ((self.length() / step_mm).ceil() as usize).max(1);
        self.points(steps)
    }

    /// The arc cut into `steps` chords: the chord end points after the start, in order.
    pub(crate) fn points(&self, steps: usize) -> impl Iterator<Item = (f64, f64)> + '_ {
        (1..=steps).map(move |step| {
            let angle = self.start_angle + self.sweep * (step as f64) / (steps as f64);
            (
                self.centre.0 + self.radius * angle.cos(),
                self.centre.1 + self.radius * angle.sin(),
            )
        })
    }
}

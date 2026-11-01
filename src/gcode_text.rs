//! The G-code vocabulary shared by the two readers of sliced plates (`gcode_footprint` for the
//! probe's first layer, `gcode_collisions` for a whole plate). Both slicer dialects are
//! understood: OrcaSlicer marks layers with `;LAYER_CHANGE`, features with `;TYPE:` and widths
//! with `;WIDTH:`; Bambu Studio uses `; CHANGE_LAYER`, `; FEATURE:` and `; LINE_WIDTH:`.

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

    pub(crate) fn length(&self) -> f64 {
        self.radius * self.sweep.abs()
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

//! Which points can be tips and what a tip is (spec 01 §5.4 table, §5.6 point status).

use super::Context;
use crate::polygon_ops::{IntPoint, inside_even_odd};
use crate::support_model::node::Node;
use crate::support_model::settings::Derived;

/// A line point's standing at its layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Status {
    /// Outside AvoidSafe: a branch from here reaches the bed even through holes.
    Safe,
    /// Outside AvoidFast only.
    Bed,
}

/// The status of `point` at `layer`; `None` = inside both avoidances (the point is dropped).
pub(super) fn status(context: &Context, point: IntPoint, layer: usize) -> Option<Status> {
    if !inside_even_odd(&context.safe_avoidance(layer), point) {
        Some(Status::Safe)
    } else if !inside_even_odd(&context.fast_avoidance(layer), point) {
        Some(Status::Bed)
    } else {
        None
    }
}

/// The point is still valid one layer down: outside the avoidance of the kind its status
/// stands for.
pub(super) fn still_valid(
    context: &Context,
    point: IntPoint,
    status: Status,
    below: usize,
) -> bool {
    match status {
        Status::Safe => !inside_even_odd(&context.safe_avoidance(below), point),
        Status::Bed => !inside_even_odd(&context.fast_avoidance(below), point),
    }
}

/// What a roof tip serves: `roof_depth` roof layers lie between it and the contact layer.
pub(super) struct TipKind {
    pub status: Status,
    pub roof_depth: usize,
    /// More than five points on the line it came from (decides `keep_round`).
    pub long_line: bool,
}

/// A tip on `layer` at `point`: a 10 µm circle of influence, depth 0, to-bed.
pub(super) fn tip(derived: &Derived, point: IntPoint, layer: usize, kind: &TipKind) -> Node {
    let under_roof = kind.roof_depth > 0;
    Node {
        layer,
        depth: 0,
        rdepth: 0,
        foot: 0.0,
        origin_layer: layer,
        origin_xy: point,
        hint: point,
        centre: Some(point),
        to_bed: true,
        graceful: false,
        tight_xy: derived.tight_xy,
        under_roof,
        past_holes: kind.status == Status::Safe,
        keep_round: derived.tip_radius < 3 * derived.line_width && !under_roof && kind.long_line,
        hold_until: derived.roof_layers.saturating_sub(kind.roof_depth) as i64,
        model_growth: 0,
        parents: Vec::new(),
        area: vec![circle(derived, point)],
        deleted: false,
        marked: false,
    }
}

/// The 10 µm circle: a regular polygon whose vertex count comes from a chord error of 25
/// units, so 45 vertices on the 1 nm grid and 15 on B's (spec 02 §1.3); coordinates
/// truncate.
fn circle(derived: &Derived, centre: IntPoint) -> Vec<IntPoint> {
    let radius = derived.grid.length(0.01);
    let count = Derived::circle_vertices(radius as f64, 25.0).max(3);
    (0..count)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / count as f64;
            IntPoint::new(
                centre.x + (radius as f64 * libm::cos(angle)) as i32,
                centre.y + (radius as f64 * libm::sin(angle)) as i32,
            )
        })
        .collect()
}

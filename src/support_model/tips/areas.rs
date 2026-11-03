//! Which parts of one overhang may carry tips (spec 01 §5.3). Tips cannot stand inside the
//! forbidden region F_m (branches from there could not reach the bed), so the overhang is
//! split into the part outside it (`roofs`, tips and roofs go here) and a fringe that grows
//! outward in small rounds so tips can stand at the edge of overhang that lies just inside
//! F_m.

use super::Context;
use crate::polygon_ops::{
    Island, Join, Rings, difference, intersection, islands, offset, simplify, union,
};

/// One area to sample: a connected part, and whether it is a roof candidate.
pub(super) struct Candidate {
    pub island: Island,
    pub roof: bool,
}

/// Round-grow tolerance of the fringe rounds (mm) and the simplification after it (mm).
const GROW_ARC_TOLERANCE_MM: f64 = 0.01;
const GROW_SIMPLIFY_MM: f64 = 0.015;

/// The safety offset of the fringe against the roofs (grid units, spec 01 §5.3).
const FRINGE_SAFETY_UNITS: f64 = 10.0;

/// The candidates of the overhang `overhang` at insertion layer `layer`: roof parts first,
/// then fringe parts, each in the boolean library's component order.
pub(super) fn task_areas(context: &Context, layer: usize, overhang: &Rings) -> Vec<Candidate> {
    let derived = &context.stage.derived;
    let forbidden = context.forbidden(layer);
    let clearance = &context.stage.clearance;
    let bare = clearance.collision(0, layer, true);
    let region = difference(overhang, &forbidden);
    let half_line = derived.line_width as f64 / 2.0;
    let near = offset(&region, half_line, Join::Miter(1.2));
    let mut rest = intersection(&difference(overhang, &near), &forbidden);
    let mut standing = region.clone();
    let step = derived.outset_step;
    let mut round = 1;
    while step > 0 && round * step < derived.extra_outset && !rest.is_empty() {
        let reach = (round * step) as f64;
        let covered = grown(context, &standing, 1.5 * reach, &bare);
        rest = difference(&rest, &covered);
        let edge = difference(&grown(context, &rest, reach, &bare), &forbidden);
        standing = union(&standing, &edge);
        round += 1;
    }
    let roofs = if derived.roof_layers > 0 {
        region.clone()
    } else {
        Rings::new()
    };
    let fringe = difference(
        &standing,
        &offset(&roofs, FRINGE_SAFETY_UNITS, Join::Miter(1.2)),
    );
    islands(&roofs)
        .into_iter()
        .map(|island| Candidate { island, roof: true })
        .chain(islands(&fringe).into_iter().map(|island| Candidate {
            island,
            roof: false,
        }))
        .collect()
}

/// G(X, r): X grown by r with round joins, simplified, minus the bare collision.
fn grown(context: &Context, region: &Rings, radius: f64, bare: &Rings) -> Rings {
    let grid = context.stage.derived.grid;
    let rounded = offset(
        region,
        radius,
        Join::Round(grid.scaled(GROW_ARC_TOLERANCE_MM)),
    );
    let simple = simplify(&rounded, grid.scaled(GROW_SIMPLIFY_MM), false);
    difference(&simple, bare)
}

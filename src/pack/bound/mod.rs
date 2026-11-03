//! An upper bound on the copies a plate holds (Oler / Folkman-Graham, in the normed form of
//! Bezdek and Langi 2020): for a convex centrally symmetric `C` such that every integer point
//! strictly inside it collides (`C` inscribed in the symmetric no-fit region), a set of
//! origins in a convex `D` whose differences are never inside `C` has at most
//! `area(D) / Δ(C) + perimeter_C(D) / 2 + 1` points, `Δ` the critical determinant of `C`.
//!
//! `D` is the hull of the feasible origins before the tower (a relaxation: ignoring the tower
//! can only raise the bound). The bound holds for the raster problem of one resolution, with
//! a small margin on `Δ` (sampled, see `critical`): a stop rule and a report figure, not a
//! proof about the continuous plate. Measured on the goat bench (0.25 mm): U1 66, P1S 56,
//! A2L 91 against 58 / 47 / 74 locked and 59 / 47 / 78 finer; the boundary term is loose
//! because the densest goat lattices are diagonal, not edge-aligned.
//!
//! A single class: the bound holds for layouts of that class only. Layouts mixing classes
//! need a `C` inside the no-fit regions of every pair (`UpperBounds::overall`), which is much
//! looser (research 2026-11-03: 0/180 goat pair 77 / 68 / 115).

mod ball;
mod critical;
mod region;

use super::context::Ctx;
use crate::geometry::Point;
use crate::mesh_section::hull::convex_hull;
use ball::{Ball, inscribed_ball};
use critical::critical_determinant;
use region::symmetric_region;
use serde::Serialize;

/// Slack on the final floor: the bound is an integer only up to float noise.
const FLOOR_EPS: f64 = 1e-6;

/// The bounds of a run at its resolution.
#[derive(Clone, Debug, Default, Serialize)]
pub struct UpperBounds {
    /// Per class: the most copies of that class alone (`None` when no ball fits).
    pub per_class: Vec<Option<usize>>,
    /// The most copies of any mix of the classes (`None` when no ball fits).
    pub overall: Option<usize>,
}

/// The bounds of every class and of their mix.
pub fn upper_bounds(ctx: &Ctx) -> UpperBounds {
    let per_class: Vec<Option<usize>> = (0..ctx.classes())
        .map(|class| bound_of(ctx, &[class]))
        .collect();
    let overall = match per_class.len() {
        0 => None,
        1 => per_class[0],
        _ => bound_of(ctx, &(0..ctx.classes()).collect::<Vec<_>>()),
    };
    UpperBounds { per_class, overall }
}

fn bound_of(ctx: &Ctx, classes: &[usize]) -> Option<usize> {
    let ball = inscribed_ball(&symmetric_region(ctx, classes))?;
    let delta = critical_determinant(&ball)?;
    let (area, perimeter) = hull_measures(&origin_points(ctx, classes), &ball)?;
    Some((area / delta + perimeter / 2.0 + 1.0 + FLOOR_EPS).floor() as usize)
}

/// The leftmost and rightmost feasible origin of every row, which hold the hull's vertices.
fn origin_points(ctx: &Ctx, classes: &[usize]) -> Vec<Point> {
    let mut points = Vec::new();
    for &class in classes {
        let allowed = &ctx.origins(class).allowed;
        for (index, row) in allowed.rows.iter().enumerate() {
            let (Some(first), Some(last)) = (row.first(), row.last()) else {
                continue;
            };
            let y = f64::from(allowed.min_y + index as i32);
            points.push(Point {
                x: f64::from(first.0),
                y,
            });
            points.push(Point {
                x: f64::from(last.1 - 1),
                y,
            });
        }
    }
    points
}

/// Area of the hull of `points` and its perimeter in the norm of `ball`; a hull with no
/// area (one point, a row of points) has area 0 and twice its length. `None` without points.
fn hull_measures(points: &[Point], ball: &Ball) -> Option<(f64, f64)> {
    if let Some(hull) = convex_hull(points) {
        let n = hull.len();
        let edges = (0..n).map(|i| (hull[i], hull[(i + 1) % n]));
        let area = edges.clone().map(|(p, q)| p.x * q.y - q.x * p.y).sum::<f64>() / 2.0;
        let perimeter = edges.map(|(p, q)| ball.norm((q.x - p.x, q.y - p.y))).sum();
        return Some((area.abs(), perimeter));
    }
    let low = points
        .iter()
        .copied()
        .min_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)))?;
    let high = points
        .iter()
        .copied()
        .max_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)))?;
    Some((0.0, 2.0 * ball.norm((high.x - low.x, high.y - low.y))))
}

#[cfg(test)]
pub(crate) mod tests;

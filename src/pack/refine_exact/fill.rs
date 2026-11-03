//! Re-seats the copies the raster fill added around a lattice, in exact geometry. Each keeps
//! its offset from its lattice site, so it follows the site when the lattice shrinks; when
//! that spot is taken it takes the nearest free spot on a fine grid around it.

use super::lattice::{Params, offset, point};
use super::oracle::Oracle;
use super::seed::{Extra, slot_base};
use crate::geometry::Point;

/// Search radius around an extra's followed position, mm, and the grid step inside it.
const RADIUS_MM: f64 = 2.0;
const STEP_MM: f64 = 0.125;

/// Offsets of the search grid inside the radius, nearest first (ties: grid order).
fn search_offsets() -> Vec<Point> {
    let n = (RADIUS_MM / STEP_MM).round() as i32;
    let mut grid: Vec<(i32, i32)> = Vec::new();
    for j in -n..=n {
        for i in -n..=n {
            if i * i + j * j <= n * n {
                grid.push((i, j));
            }
        }
    }
    grid.sort_by_key(|&(i, j)| i * i + j * j);
    grid.iter()
        .map(|&(i, j)| point(f64::from(i) * STEP_MM, f64::from(j) * STEP_MM))
        .collect()
}

/// A copy of `class` at `at` breaks no zone rule and no pair rule with `placed`.
pub(super) fn fits(oracle: &Oracle, placed: &[(usize, Point)], class: usize, at: Point) -> bool {
    oracle.zones_clear(class, at)
        && placed
            .iter()
            .all(|&(other, p)| oracle.pair_clear(other, class, point(at.x - p.x, at.y - p.y)))
}

/// `placed` (lattice sites) plus the extras that find a spot, in order, stopping at `cap`
/// copies when set. Returns the copies as `(class, position)`.
pub(super) fn seat_extras(
    oracle: &Oracle,
    classes: &[usize],
    x: &Params,
    sites: &[(usize, Point)],
    extras: &[Extra],
    cap: usize,
) -> Vec<(usize, Point)> {
    let offsets = search_offsets();
    let mut placed: Vec<(usize, Point)> = sites.iter().map(|&(s, p)| (classes[s], p)).collect();
    for extra in extras {
        if cap > 0 && placed.len() >= cap {
            break;
        }
        let origin = slot_base(x, extra.slot);
        let site = offset(x, extra.i, extra.j, false);
        let home = point(
            origin.x + site.x + extra.delta.x,
            origin.y + site.y + extra.delta.y,
        );
        let class = classes[extra.slot];
        let spot = offsets
            .iter()
            .map(|o| point(home.x + o.x, home.y + o.y))
            .find(|&at| fits(oracle, &placed, class, at));
        if let Some(at) = spot {
            placed.push((class, at));
        }
    }
    placed
}

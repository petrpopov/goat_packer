//! A convex, centrally symmetric polygon `C` as the unit ball of a norm, inscribed in the
//! no-fit region: every integer point strictly inside it collides, so two copies whose origins
//! differ by an integer vector are at `C`-distance at least 1 (Oler / Folkman-Graham).

use super::region::first_missing;
use crate::geometry::Point;
use crate::mesh_section::hull::convex_hull;
use crate::span_raster::SpanRaster;
use std::f64::consts::PI;

/// Directions over a half turn; the polygon has up to twice as many vertices.
const DIRECTIONS: usize = 48;
/// Ray-march step, pixels.
const STEP: f64 = 0.5;
/// The least factor one shrink step applies (a point near the centre would zero the ball).
const MIN_FACTOR: f64 = 0.5;
/// Shrink rounds before the ball is given up (a region this ragged has no useful ball).
const MAX_ROUNDS: usize = 2000;
/// Growth tried per direction and pass after the ball is valid.
const GROW_FACTOR: f64 = 1.01;
const GROW_PASSES: usize = 3;
/// Integer points this close to the boundary count as on it.
const EDGE_EPS: f64 = 1e-7;

pub(super) struct Ball {
    /// Counter-clockwise vertices.
    pub(super) vertices: Vec<Point>,
    /// `C = { x : a . x <= 1 }` for each row `a`, one per edge.
    planes: Vec<(f64, f64)>,
}

impl Ball {
    pub(super) fn from_vertices(vertices: Vec<Point>) -> Ball {
        let n = vertices.len();
        let planes = (0..n)
            .map(|i| {
                let (p, q) = (vertices[i], vertices[(i + 1) % n]);
                let (nx, ny) = (q.y - p.y, p.x - q.x);
                let offset = nx * p.x + ny * p.y;
                (nx / offset, ny / offset)
            })
            .collect();
        Ball { vertices, planes }
    }

    /// `||e||_C`: the factor by which `C` scales to reach `e`.
    pub(super) fn norm(&self, e: (f64, f64)) -> f64 {
        self.planes
            .iter()
            .map(|a| a.0 * e.0 + a.1 * e.1)
            .fold(0.0, f64::max)
    }

    /// The edge whose plane the point is closest to, with the point's value on that plane.
    fn closest_edge(&self, x: f64, y: f64) -> (usize, f64) {
        self.planes
            .iter()
            .map(|a| a.0 * x + a.1 * y)
            .enumerate()
            .fold((0, f64::NEG_INFINITY), |best, (i, v)| {
                if v > best.1 { (i, v) } else { best }
            })
    }

    /// The `x` extent of the polygon on the horizontal line `y`.
    fn x_range(&self, y: f64) -> Option<(f64, f64)> {
        let n = self.vertices.len();
        let mut range: Option<(f64, f64)> = None;
        for i in 0..n {
            let (p, q) = (self.vertices[i], self.vertices[(i + 1) % n]);
            if (p.y > y) == (q.y > y) {
                continue;
            }
            let x = p.x + (q.x - p.x) * (y - p.y) / (q.y - p.y);
            range = Some(range.map_or((x, x), |(lo, hi)| (lo.min(x), hi.max(x))));
        }
        range
    }

    fn y_extent(&self) -> (f64, f64) {
        let ys = self.vertices.iter().map(|p| p.y);
        (
            ys.clone().fold(f64::INFINITY, f64::min),
            ys.fold(f64::NEG_INFINITY, f64::max),
        )
    }

    /// Per row, the first integer point strictly inside the polygon that `region` lacks.
    fn violations(&self, region: &SpanRaster) -> Vec<(i32, i32)> {
        let (low, high) = self.y_extent();
        // Rows on the top or bottom extreme lie on the boundary (a flat edge there).
        let rows = (low + EDGE_EPS).floor() as i32 + 1..=(high - EDGE_EPS).ceil() as i32 - 1;
        rows.filter_map(|y| {
            let (left, right) = self.x_range(f64::from(y))?;
            let first = (left + EDGE_EPS).floor() as i32 + 1;
            let last = (right - EDGE_EPS).ceil() as i32 - 1;
            first_missing(region, y, first, last).map(|x| (x, y))
        })
        .collect()
    }
}

/// A ball inscribed in `region` (symmetric about the origin), or `None` when none with an
/// interior exists. Radii start at the first non-region point along each ray, then shrink at
/// the directions of violating integer points until the exact row check passes.
pub(super) fn inscribed_ball(region: &SpanRaster) -> Option<Ball> {
    let mut radii: Vec<f64> = (0..DIRECTIONS)
        .map(|k| ray_radius(region, direction(k)))
        .collect();
    for _ in 0..MAX_ROUNDS {
        let ball = ball_of(&radii)?;
        let violations = ball.violations(region);
        if violations.is_empty() {
            return grown(region, radii).or(Some(ball));
        }
        shrink_edges(&mut radii, &ball, &violations);
    }
    None
}

/// The shrink steps overshoot (they move both ends of an edge): push each radius out again
/// while the ball stays valid. `None` only if the grown ball degenerates.
fn grown(region: &SpanRaster, mut radii: Vec<f64>) -> Option<Ball> {
    for _ in 0..GROW_PASSES {
        for k in 0..DIRECTIONS {
            let kept = radii[k];
            radii[k] = kept * GROW_FACTOR;
            let valid = ball_of(&radii).is_some_and(|b| b.violations(region).is_empty());
            if !valid {
                radii[k] = kept;
            }
        }
    }
    ball_of(&radii)
}

fn direction(k: usize) -> (f64, f64) {
    let theta = PI * k as f64 / DIRECTIONS as f64;
    (theta.cos(), theta.sin())
}

/// The distance of the first integer point missing from the region along `dir` (sampled at
/// the nearest integer point): a ball may reach it, not contain it.
fn ray_radius(region: &SpanRaster, dir: (f64, f64)) -> f64 {
    let mut t = 0.0;
    loop {
        t += STEP;
        let (x, y) = ((t * dir.0).round() as i32, (t * dir.1).round() as i32);
        if !region.contains(x, y) {
            return f64::from(x).hypot(f64::from(y));
        }
    }
}

fn ball_of(radii: &[f64]) -> Option<Ball> {
    let corner = |k: usize, sign: f64| {
        let (dx, dy) = direction(k);
        Point {
            x: sign * radii[k] * dx,
            y: sign * radii[k] * dy,
        }
    };
    let points: Vec<Point> = (0..DIRECTIONS)
        .flat_map(|k| [corner(k, 1.0), corner(k, -1.0)])
        .collect();
    convex_hull(&points).map(Ball::from_vertices)
}

/// Pulls the edge closest to each violating point inward until the point lies on it: both
/// end vertices are scaled by the point's plane value (below 1 inside). All violations of a
/// round are applied at once, each against the ball as it was at the start of the round.
fn shrink_edges(radii: &mut [f64], ball: &Ball, violations: &[(i32, i32)]) {
    let before = radii.to_vec();
    let n = ball.vertices.len();
    for &(x, y) in violations {
        let (edge, value) = ball.closest_edge(f64::from(x), f64::from(y));
        let factor = (value * (1.0 - 1e-6)).clamp(MIN_FACTOR, 1.0 - 1e-6);
        for vertex in [ball.vertices[edge], ball.vertices[(edge + 1) % n]] {
            let k = direction_index(vertex);
            radii[k] = radii[k].min(before[k] * factor);
        }
    }
}

/// The direction (over a half turn) a ball vertex was made from.
fn direction_index(vertex: Point) -> usize {
    let turn = vertex.y.atan2(vertex.x).rem_euclid(PI) / PI * DIRECTIONS as f64;
    turn.round() as usize % DIRECTIONS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_region(half: i32) -> SpanRaster {
        let side = 2 * half + 1;
        SpanRaster {
            min_y: -half,
            rows: vec![vec![(-half, half + 1)]; side as usize],
        }
    }

    #[test]
    fn the_ball_of_a_square_region_is_its_square() {
        // Offsets |d| <= 9 collide, so the open square (-10, 10)^2 is the largest ball.
        let ball = inscribed_ball(&square_region(9)).expect("ball");
        for corner in [(10.0, 10.0), (-10.0, 10.0), (10.0, -10.0)] {
            assert!((ball.norm(corner) - 1.0).abs() < 0.03, "{corner:?} {}", ball.norm(corner));
        }
        assert!((ball.norm((10.0, 0.0)) - 1.0).abs() < 0.03);
        assert!(ball.violations(&square_region(9)).is_empty());
    }
}

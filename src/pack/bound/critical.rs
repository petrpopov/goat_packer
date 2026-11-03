//! The critical determinant `Δ(C)` of a convex centrally symmetric polygon: the least cell
//! area of a lattice whose nonzero points all lie outside the open ball (the densest lattice
//! packing of `C/2`). For each boundary point `p` the hexagon `p, q, q - p` with `q` on the
//! boundary of `C` and of `C + p` is the tightest lattice through `p`; `Δ` is the least
//! `|p x q|` over `p`. Sampled `p` can only overshoot, so the result is cut by a small margin.

use super::ball::Ball;
use crate::geometry::Point;

/// Boundary points tried per edge of the half boundary (vertices included).
const SAMPLES_PER_EDGE: usize = 6;
/// Sampling can miss the exact minimum by a little: report this fraction less.
const MARGIN: f64 = 0.995;
const TINY: f64 = 1e-9;

/// `Δ` of the ball in pixel units squared; `None` for a degenerate ball.
pub(super) fn critical_determinant(ball: &Ball) -> Option<f64> {
    let v = &ball.vertices;
    let n = v.len();
    let mut best = f64::INFINITY;
    for i in 0..n / 2 {
        let (a, b) = (v[i], v[(i + 1) % n]);
        for s in 0..SAMPLES_PER_EDGE {
            let t = s as f64 / SAMPLES_PER_EDGE as f64;
            let p = Point {
                x: a.x + (b.x - a.x) * t,
                y: a.y + (b.y - a.y) * t,
            };
            best = best.min(least_cell(v, p));
        }
    }
    (best.is_finite() && best > 0.0).then_some(best * MARGIN)
}

/// The least `|p x q|` over the boundary points `q` of `C` that are also on `C + p`.
fn least_cell(v: &[Point], p: Point) -> f64 {
    let n = v.len();
    let mut least = f64::INFINITY;
    for i in 0..n {
        let (a0, a1) = (v[i], v[(i + 1) % n]);
        for j in 0..n {
            let b0 = Point {
                x: v[j].x + p.x,
                y: v[j].y + p.y,
            };
            let w = v[(j + 1) % n];
            let b1 = Point {
                x: w.x + p.x,
                y: w.y + p.y,
            };
            for q in touch_points(a0, a1, b0, b1) {
                let cell = (p.x * q.y - p.y * q.x).abs();
                if cell > TINY {
                    least = least.min(cell);
                }
            }
        }
    }
    least
}

/// Where segment `a0 a1` meets segment `b0 b1`: the crossing, or both ends of a shared run
/// when they overlap along one line.
fn touch_points(a0: Point, a1: Point, b0: Point, b1: Point) -> Vec<Point> {
    let (d, e) = ((a1.x - a0.x, a1.y - a0.y), (b1.x - b0.x, b1.y - b0.y));
    let cross = d.0 * e.1 - d.1 * e.0;
    let offset = (b0.x - a0.x, b0.y - a0.y);
    let scale = d.0.hypot(d.1) * e.0.hypot(e.1);
    if cross.abs() > TINY * scale.max(1.0) {
        let t = (offset.0 * e.1 - offset.1 * e.0) / cross;
        let u = (offset.0 * d.1 - offset.1 * d.0) / cross;
        let range = -TINY..=1.0 + TINY;
        if range.contains(&t) && range.contains(&u) {
            return vec![Point {
                x: a0.x + d.0 * t,
                y: a0.y + d.1 * t,
            }];
        }
        return Vec::new();
    }
    overlap_ends(a0, d, b0, b1)
}

/// The ends of the part of `b0 b1` on the segment `a0 + t d` (`t` in `[0, 1]`) when the two
/// are collinear; empty when they are not or do not overlap.
fn overlap_ends(a0: Point, d: (f64, f64), b0: Point, b1: Point) -> Vec<Point> {
    let len2 = d.0 * d.0 + d.1 * d.1;
    if len2 < TINY {
        return Vec::new();
    }
    let along = |p: Point| ((p.x - a0.x) * d.0 + (p.y - a0.y) * d.1) / len2;
    let off_line = |p: Point| ((p.x - a0.x) * d.1 - (p.y - a0.y) * d.0).abs() / len2.sqrt();
    if off_line(b0) > 1e-7 || off_line(b1) > 1e-7 {
        return Vec::new();
    }
    let (t0, t1) = (along(b0).min(along(b1)).max(0.0), along(b0).max(along(b1)).min(1.0));
    if t0 > t1 {
        return Vec::new();
    }
    [t0, t1]
        .map(|t| Point {
            x: a0.x + d.0 * t,
            y: a0.y + d.1 * t,
        })
        .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::bound::ball::Ball;

    fn polygon(points: Vec<Point>) -> Ball {
        Ball::from_vertices(points)
    }

    fn regular(sides: usize, radius: f64) -> Ball {
        polygon(
            (0..sides)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / sides as f64;
                    Point {
                        x: radius * a.cos(),
                        y: radius * a.sin(),
                    }
                })
                .collect(),
        )
    }

    #[test]
    fn a_square_ball_has_determinant_one() {
        // Four vertices at radius sqrt(2) on the axes: the square [-1, 1]^2 turned 45 degrees.
        let square = regular(4, std::f64::consts::SQRT_2);
        let delta = critical_determinant(&square).expect("determinant");
        assert!((delta / MARGIN - 1.0).abs() < 1e-6, "{delta}");
    }

    #[test]
    fn a_fine_polygon_approaches_the_disk() {
        // Disk of radius 1: Δ = sqrt(3) / 2 (the hexagonal lattice).
        let delta = critical_determinant(&regular(96, 1.0)).expect("determinant");
        assert!((delta / MARGIN - 0.866).abs() < 0.01, "{delta}");
    }
}

//! Resampling a polyline into tip positions (spec 01 §5.6): points at least `spacing` apart
//! (straight-line distance, not arc length), starting at the polyline's start, the output
//! order being the tip order that reaches the merge stage.

use crate::polygon_ops::IntPoint;

/// The walk may enlarge its distance this many times when it is stuck.
const MAX_ENLARGEMENTS: usize = 2;

/// Spacing shrinks by this factor while fewer than `need` points result.
const RELAX: f64 = 0.9;

fn distance(a: IntPoint, b: IntPoint) -> f64 {
    let (dx, dy) = (
        f64::from(a.x) - f64::from(b.x),
        f64::from(a.y) - f64::from(b.y),
    );
    libm::sqrt(dx * dx + dy * dy)
}

fn length(path: &[IntPoint]) -> f64 {
    path.windows(2).map(|pair| distance(pair[0], pair[1])).sum()
}

/// Points along `path` (closed: the last vertex connects back to the first) at least
/// `spacing` units apart, at least `need` of them where the polyline allows. `smallest` is
/// the 0.1 mm floor in grid units.
pub fn resample(
    path: &[IntPoint],
    closed: bool,
    spacing: f64,
    need: usize,
    smallest: f64,
) -> Vec<IntPoint> {
    if path.len() < 2 {
        return path.to_vec();
    }
    let mut spacing = spacing.max(smallest);
    let mut walked = path.to_vec();
    if closed {
        walked.push(path[0]);
    }
    if length(&walked) < 2.0 * spacing && need <= 1 {
        return vec![point_at(&walked, 0.5 * length(&walked))];
    }
    let (walked, far) = if closed {
        rotate_to_farthest(path)
    } else {
        (walked, None)
    };
    loop {
        let points = walk(&walked, far, spacing, need);
        if points.len() >= need || spacing * RELAX < smallest {
            return points;
        }
        spacing *= RELAX;
    }
}

/// The point at arc length `at` from the first vertex.
fn point_at(path: &[IntPoint], at: f64) -> IntPoint {
    let mut left = at;
    for pair in path.windows(2) {
        let segment = distance(pair[0], pair[1]);
        if left <= segment && segment > 0.0 {
            let t = left / segment;
            return IntPoint::new(
                (f64::from(pair[0].x) + (f64::from(pair[1].x) - f64::from(pair[0].x)) * t) as i32,
                (f64::from(pair[0].y) + (f64::from(pair[1].y) - f64::from(pair[0].y)) * t) as i32,
            );
        }
        left -= segment;
    }
    path[path.len() - 1]
}

/// A closed ring rotated to start at the first vertex of its farthest vertex pair (first
/// strict maximum), closed again; the other vertex of the pair is returned.
fn rotate_to_farthest(ring: &[IntPoint]) -> (Vec<IntPoint>, Option<IntPoint>) {
    let mut best = (0.0, 0, 0);
    for i in 0..ring.len() {
        for j in i + 1..ring.len() {
            let d = distance(ring[i], ring[j]);
            if d > best.0 {
                best = (d, i, j);
            }
        }
    }
    let mut rotated: Vec<IntPoint> = ring[best.1..]
        .iter()
        .chain(&ring[..best.1])
        .copied()
        .collect();
    rotated.push(rotated[0]);
    (rotated, Some(ring[best.2]))
}

fn walk(path: &[IntPoint], far: Option<IntPoint>, spacing: f64, need: usize) -> Vec<IntPoint> {
    let mut chosen = vec![path[0]];
    let last = path[path.len() - 1];
    let extra = far.unwrap_or(last);
    if need > 1 || distance(extra, path[0]) > spacing {
        chosen.push(extra);
    }
    let mut anchor = path[0];
    let mut previous = path[0];
    let mut index = 1;
    let mut reach = spacing;
    let mut enlargements = 0;
    let mut steps = 0;
    while index < path.len() && steps < 100_000 {
        steps += 1;
        let Some(found) = (index..path.len()).find(|&j| distance(anchor, path[j]) >= reach) else {
            break;
        };
        let from = if found == index {
            previous
        } else {
            path[found - 1]
        };
        let Some(candidate) = circle_hit(anchor, reach, from, path[found]) else {
            index = found + 1;
            previous = path[found];
            continue;
        };
        let nearest = chosen
            .iter()
            .map(|&p| distance(p, candidate))
            .fold(f64::MAX, f64::min);
        if nearest >= spacing - 1.0 {
            chosen.push(candidate);
            reach = spacing;
            enlargements = 0;
        } else if candidate == anchor {
            if enlargements >= MAX_ENLARGEMENTS {
                break;
            }
            enlargements += 1;
            reach += spacing;
        } else {
            reach = (spacing - nearest).max(spacing * 0.05);
        }
        anchor = candidate;
        previous = candidate;
        index = found;
    }
    chosen
}

/// Where the circle of `radius` around `centre` is crossed going out along `a` → `b`.
fn circle_hit(centre: IntPoint, radius: f64, a: IntPoint, b: IntPoint) -> Option<IntPoint> {
    let (ax, ay) = (
        f64::from(a.x) - f64::from(centre.x),
        f64::from(a.y) - f64::from(centre.y),
    );
    let (dx, dy) = (
        f64::from(b.x) - f64::from(a.x),
        f64::from(b.y) - f64::from(a.y),
    );
    let quad = dx * dx + dy * dy;
    if quad == 0.0 {
        return None;
    }
    let half = ax * dx + ay * dy;
    let discriminant = half * half - quad * (ax * ax + ay * ay - radius * radius);
    if discriminant < 0.0 {
        return None;
    }
    let t = (-half + libm::sqrt(discriminant)) / quad;
    if !(0.0..=1.0 + 1e-9).contains(&t) {
        return None;
    }
    let t = t.min(1.0);
    Some(IntPoint::new(
        (f64::from(a.x) + dx * t) as i32,
        (f64::from(a.y) + dy * t) as i32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(length: i32) -> Vec<IntPoint> {
        vec![IntPoint::new(0, 0), IntPoint::new(length, 0)]
    }

    #[test]
    fn a_long_line_gets_points_a_spacing_apart() {
        let points = resample(&line(1000), false, 100.0, 1, 10.0);
        assert!(points.len() >= 10, "{points:?}");
        for pair in points.windows(2).skip(1) {
            assert!(distance(pair[0], pair[1]) >= 99.0);
        }
        assert_eq!(points[0], IntPoint::new(0, 0));
    }

    #[test]
    fn a_short_line_gets_its_middle() {
        let points = resample(&line(150), false, 100.0, 1, 10.0);
        assert_eq!(points, vec![IntPoint::new(75, 0)]);
    }

    #[test]
    fn a_small_ring_is_sampled_at_least_need_times() {
        let ring = vec![
            IntPoint::new(0, 0),
            IntPoint::new(60, 0),
            IntPoint::new(60, 60),
            IntPoint::new(0, 60),
        ];
        let points = resample(&ring, true, 100.0, 3, 10.0);
        assert!(points.len() >= 3, "{points:?}");
    }
}

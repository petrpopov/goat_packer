//! The polygons a tube slice is made of (spec 03 §3.2, §3.9): an inscribed ellipse for a tilted
//! tube, a disc for a ball, both with the slicers' chord tolerance of 0.015 mm.

use crate::polygon_ops::{IntPoint, Ring};

/// Chord tolerance of the tube discretisation, mm (spec 03 §3.2).
pub const CHORD_TOLERANCE_MM: f64 = 0.015;

/// Segments of a circle of `radius` units with chord error `tolerance` units; at least 6.
pub fn segments(radius: f64, tolerance: f64) -> usize {
    if radius <= tolerance {
        return 6;
    }
    let step = 2.0 * libm::acos(1.0 - tolerance / radius);
    (libm::ceil(std::f64::consts::TAU / step) as usize).max(6)
}

/// A counter-clockwise ellipse with semi-axes `major` along `direction` and `minor` across,
/// vertices on the curve.
pub fn ellipse(
    centre: [f64; 2],
    major: f64,
    minor: f64,
    direction: [f64; 2],
    tolerance: f64,
) -> Ring {
    let count = segments(major, tolerance);
    let across = [-direction[1], direction[0]];
    (0..count)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / count as f64;
            let (along, side) = (major * libm::cos(angle), minor * libm::sin(angle));
            IntPoint::new(
                libm::round(centre[0] + direction[0] * along + across[0] * side) as i32,
                libm::round(centre[1] + direction[1] * along + across[1] * side) as i32,
            )
        })
        .collect()
}

/// A straight tube piece (a right circular cylinder of `radius` between two planes across its
/// axis) from `from` at height `z_from` to `to` at `z_to`, cut by the horizontal plane at
/// `plane`: an ellipse clipped along its major axis to the part of the plane between the end
/// planes. `None` when the plane misses the piece. A flat piece (a branch that jumps sideways
/// between layers) is a short stadium, not the metres-long ellipse of the endless cylinder.
pub fn slice_cylinder(
    from: [f64; 2],
    z_from: f64,
    to: [f64; 2],
    z_to: f64,
    radius: f64,
    plane: f64,
    tolerance: f64,
) -> Option<Ring> {
    let shift = [to[0] - from[0], to[1] - from[1]];
    let sideways = libm::hypot(shift[0], shift[1]);
    let height = z_to - z_from;
    let length = libm::hypot(height, sideways);
    if length <= 0.0 {
        return None;
    }
    if sideways < 1e-9 * length {
        let inside = (z_from.min(z_to)..=z_from.max(z_to)).contains(&plane);
        return inside.then(|| disc(from, radius, tolerance));
    }
    let direction = [shift[0] / sideways, shift[1] / sideways];
    let sine = sideways / length;
    let cosine = height / length;
    // Where the axis meets the plane (extended past the ends if need be), and how far along
    // the axis that is.
    let along_axis = (plane - z_from) / cosine;
    let crossing = [
        from[0] + direction[0] * (plane - z_from) * sideways / height,
        from[1] + direction[1] * (plane - z_from) * sideways / height,
    ];
    let low = -along_axis / sine;
    let high = (length - along_axis) / sine;
    let major = radius / cosine;
    let (low, high) = (low.max(-major), high.min(major));
    if low >= high {
        return None;
    }
    let count = segments(major.min(high - low).max(radius), tolerance);
    let across = [-direction[1], direction[0]];
    let mut ring: Ring = (0..count)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / count as f64;
            let along = (major * libm::cos(angle)).clamp(low, high);
            let side = radius * libm::sin(angle);
            IntPoint::new(
                libm::round(crossing[0] + direction[0] * along + across[0] * side) as i32,
                libm::round(crossing[1] + direction[1] * along + across[1] * side) as i32,
            )
        })
        .collect();
    ring.dedup();
    Some(ring)
}

/// A counter-clockwise disc.
pub fn disc(centre: [f64; 2], radius: f64, tolerance: f64) -> Ring {
    ellipse(centre, radius, radius, [1.0, 0.0], tolerance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::polygon_ops::area;

    #[test]
    fn a_disc_is_inscribed_within_the_chord_tolerance() {
        let ring = disc([0.0, 0.0], 1_000_000.0, 15_000.0);
        let measured = area(&[ring]);
        let circle = std::f64::consts::PI * 1e12;
        assert!(measured < circle && measured > circle * 0.97, "{measured}");
    }

    #[test]
    fn an_ellipse_has_the_ratio_of_its_axes() {
        let ring = ellipse([0.0, 0.0], 2_000_000.0, 1_000_000.0, [0.0, 1.0], 15_000.0);
        let (min_x, max_x) = ring.iter().fold((i32::MAX, i32::MIN), |(lo, hi), p| {
            (lo.min(p.x), hi.max(p.x))
        });
        let (min_y, max_y) = ring.iter().fold((i32::MAX, i32::MIN), |(lo, hi), p| {
            (lo.min(p.y), hi.max(p.y))
        });
        assert!((max_y - min_y) > 3_900_000 && (max_x - min_x) < 2_100_000);
    }

    #[test]
    fn a_small_circle_keeps_a_minimum_of_vertices() {
        assert!(segments(10.0, 15_000.0) >= 6);
        assert_eq!(segments(1_000_000.0, 15_000.0), 19);
        assert_eq!(segments(400_000.0, 15_000.0), 12);
        assert_eq!(segments(3_750_000.0, 15_000.0), 36);
    }
}

//! Euclidean offsets of even-odd loops: the clearance between two kinds is a disc, not a
//! square, so masks are grown as vectors and only then rasterized (design 2026-11-02, §8.1).

use crate::geometry::Point;
use crate::object_print::Loops;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineJoin, OutlineStyle};

/// Angle step of the round joins, radians: i_overlay's finest (it clamps to [π/100, π/4]).
/// The arc's vertices lie on the circle at most twice this step apart, so a chord cuts into
/// the true disc by less than `radius * (1 - cos(step))`: 0.25 µm at a 0.5 mm radius.
const JOIN_STEP_RAD: f64 = std::f64::consts::PI / 100.0;

/// How far below `gap` the true distance of two shapes may be while their masks, each grown
/// by half of `gap`, stay apart: the chord sag of both round joins. Growing the radius to
/// cover it would push every grid-aligned straight edge across its grid line and cost a
/// whole pixel there, so the sag is a tolerance of the exact check instead.
pub(crate) fn join_sag_mm(gap: f64) -> f64 {
    gap * (1.0 - JOIN_STEP_RAD.cos())
}

/// Everything within `radius` of `loops` (even-odd rings), as even-odd rings: the exact
/// Minkowski sum with the disc, its round joins drawn as inscribed chords (`join_sag_mm`).
/// i_overlay snaps to a power-of-two grid, so coordinates that are binary fractions
/// (every multiple of a 0.25 mm pixel) come back exact and aligned edges stay aligned.
pub(crate) fn grown(loops: &[Vec<Point>], radius: f64) -> Loops {
    assert!(
        radius.is_finite() && radius >= 0.0,
        "buffer radius {radius}"
    );
    if radius == 0.0 || loops.is_empty() {
        return loops.to_vec();
    }
    offset(loops, radius)
}

/// The points at least `radius` inside `loops`. The chords of the joins around reflex
/// corners leave slivers of the removed discs in, so the result contains the exact
/// erosion: a support core is never smaller than the overlap rule allows.
pub(crate) fn shrunk(loops: &[Vec<Point>], radius: f64) -> Loops {
    assert!(
        radius.is_finite() && radius >= 0.0,
        "erosion radius {radius}"
    );
    if radius == 0.0 || loops.is_empty() {
        return loops.to_vec();
    }
    offset(loops, -radius)
}

fn offset(loops: &[Vec<Point>], distance: f64) -> Loops {
    let paths: Vec<Vec<[f64; 2]>> = loops.iter().map(|ring| open_path(ring)).collect();
    // Even-odd rings may cross or nest arbitrarily; the outline needs proper shapes
    // (outer rings counter-clockwise, holes clockwise), which simplify produces.
    let shapes = paths.simplify_shape(FillRule::EvenOdd);
    let style = OutlineStyle::new(distance).line_join(LineJoin::Round(JOIN_STEP_RAD));
    shapes
        .outline(&style)
        .into_iter()
        .flatten()
        .map(|path| closed_ring(&path))
        .collect()
}

/// A closed ring as i_overlay's implicitly closed path.
fn open_path(ring: &[Point]) -> Vec<[f64; 2]> {
    let open = match (ring.first(), ring.last()) {
        (Some(first), Some(last)) if ring.len() > 1 && first == last => &ring[..ring.len() - 1],
        _ => ring,
    };
    open.iter().map(|p| [p.x, p.y]).collect()
}

fn closed_ring(path: &[[f64; 2]]) -> Vec<Point> {
    let mut ring: Vec<Point> = path.iter().map(|&[x, y]| Point { x, y }).collect();
    if let Some(&first) = ring.first() {
        ring.push(first);
    }
    ring
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::polygon_area;

    fn square(half: f64) -> Vec<Point> {
        let mut ring: Vec<Point> = [(-half, -half), (half, -half), (half, half), (-half, half)]
            .iter()
            .map(|&(x, y)| Point { x, y })
            .collect();
        ring.push(ring[0]);
        ring
    }

    fn area(loops: &[Vec<Point>]) -> f64 {
        // Holes come back clockwise: the signed sum is the area of the even-odd set.
        loops
            .iter()
            .map(|ring| polygon_area(ring))
            .sum::<f64>()
            .abs()
    }

    #[test]
    fn a_grown_square_is_the_rounded_square_within_the_join_sag() {
        let grown = grown(&[square(1.0)], 0.5);
        let exact = 4.0 + 4.0 * 2.0 * 0.5 + std::f64::consts::PI * 0.25;
        let got = area(&grown);
        // The four corner arcs lose at most their sag along their length.
        let sag_area = join_sag_mm(1.0) * std::f64::consts::PI * 0.5;
        assert!(
            got <= exact && got >= exact - sag_area,
            "area {got}, exact {exact}"
        );
        // Straight edges stay exactly on their offset line (here a grid line).
        let bounds = crate::geometry::Rect::of_points(grown.iter().flatten());
        assert_eq!(bounds.max_x, 1.5);
    }

    #[test]
    fn a_ring_shrinks_on_both_sides() {
        let ring = vec![square(4.0), square(2.0)];
        let core = shrunk(&ring, 0.5);
        let bounds = crate::geometry::Rect::of_points(core.iter().flatten());
        assert_eq!(bounds.max_x, 3.5);
        // Width 2 - 2 * 0.5 = 1 on every side, rounded hole corners keep a little more.
        let got = area(&core);
        assert!(got >= 7.0 * 7.0 - 5.0 * 5.0 - 1e-6, "area {got}");
        assert!(got < 7.0 * 7.0 - 5.0 * 5.0 + 0.3, "area {got}");
        assert!(shrunk(&ring, 1.5).is_empty());
    }

    #[test]
    fn overlapping_even_odd_rings_cancel_before_growing() {
        // Two coincident squares are no material under the even-odd rule.
        assert!(grown(&[square(1.0), square(1.0)], 0.5).is_empty());
    }
}

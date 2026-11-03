//! Round growth of an area (`A ⊕ r`, the slicer's round-join offset) as an exact union on the
//! nm grid: the area itself, a rectangle along every edge and a disc at every vertex. The
//! discs are circumscribed polygons, so the result always contains the true round offset;
//! that is the safe side for every kind built here (landing, brim). A union of pieces also
//! closes holes and gaps narrower than 2r correctly, where a moved-ring offset would leave
//! inverted rings behind.
//!
//! S2's `polygon_ops` facade is meant to own every offset; until it lands this is the one the
//! first-layer kinds use (single-threaded integer booleans via `mesh_section::area`).

use i_overlay::core::fill_rule::FillRule;

use crate::geometry::Point;
use crate::mesh_section::area::{
    IntRing, IntRings, NM_PER_MM, counter_clockwise, difference, int_point, loops_from_rings,
    resolve, rings_from_loops,
};
use crate::object_print::Loops;

/// Largest distance a disc polygon may stick out past its circle. Small next to every margin
/// the kinds carry (landing ε 0.05 mm, tower tolerance 0.1 mm), and it only ever adds.
const ARC_TOLERANCE_MM: f64 = 0.002;

/// Even-odd `loops` grown by `distance_mm` (> 0) with round joins; closed loops back.
pub fn grow_round(loops: &[Vec<Point>], distance_mm: f64) -> Loops {
    loops_from_rings(&grown_rings(&area_rings(loops), distance_mm))
}

/// `outer ∖ inner` of one area grown twice: the ring between two round offsets.
pub fn ring_between(loops: &[Vec<Point>], inner_mm: f64, outer_mm: f64) -> Loops {
    let area = area_rings(loops);
    let outer = grown_rings(&area, outer_mm);
    let inner = if inner_mm > 0.0 {
        grown_rings(&area, inner_mm)
    } else {
        area
    };
    loops_from_rings(&difference(&outer, &inner))
}

/// The points within `distance_mm` of the boundary of `loops` (both sides).
pub fn boundary_band(loops: &[Vec<Point>], distance_mm: f64) -> Loops {
    let pieces = edge_pieces(&area_rings(loops), distance_mm);
    loops_from_rings(&resolve(&pieces, FillRule::NonZero))
}

/// `subject ∩ clip` of two even-odd loop sets.
pub fn intersect(subject: &[Vec<Point>], clip: &[Vec<Point>]) -> Loops {
    let subject = area_rings(subject);
    let outside = difference(&subject, &area_rings(clip));
    loops_from_rings(&difference(&subject, &outside))
}

/// `subject ∖ clip` of two even-odd loop sets.
pub fn subtract(subject: &[Vec<Point>], clip: &[Vec<Point>]) -> Loops {
    loops_from_rings(&difference(&area_rings(subject), &area_rings(clip)))
}

/// Even-odd loops (closed or open, any orientation) as one resolved area.
fn area_rings(loops: &[Vec<Point>]) -> IntRings {
    resolve(&rings_from_loops(loops), FillRule::EvenOdd)
}

fn grown_rings(area: &IntRings, distance_mm: f64) -> IntRings {
    if distance_mm <= 0.0 || area.is_empty() {
        return area.clone();
    }
    // Resolved outer rings run counter-clockwise like every piece, so the non-zero union
    // never cancels a piece against the area it overlaps.
    let mut rings = area.clone();
    rings.extend(edge_pieces(area, distance_mm));
    resolve(&rings, FillRule::NonZero)
}

/// A counter-clockwise rectangle along every edge and a disc at every vertex of `area`.
fn edge_pieces(area: &IntRings, distance_mm: f64) -> IntRings {
    let disc = disc_offsets(distance_mm);
    let mut pieces = IntRings::new();
    for ring in area {
        let points: Vec<[f64; 2]> = ring
            .iter()
            .map(|p| [f64::from(p.x) / NM_PER_MM, f64::from(p.y) / NM_PER_MM])
            .collect();
        for (index, &a) in points.iter().enumerate() {
            let b = points[(index + 1) % points.len()];
            pieces.extend(counter_clockwise(polygon_at(a, &disc)));
            pieces.extend(edge_rectangle(a, b, distance_mm));
        }
    }
    pieces
}

/// Vertex offsets of a polygon that circumscribes a circle of `radius_mm`.
fn disc_offsets(radius_mm: f64) -> Vec<[f64; 2]> {
    let ratio = (radius_mm / (radius_mm + ARC_TOLERANCE_MM)).clamp(-1.0, 1.0);
    let segments = (std::f64::consts::PI / libm::acos(ratio)).ceil().max(8.0) as usize;
    let half_step = std::f64::consts::PI / segments as f64;
    let reach = radius_mm / libm::cos(half_step);
    (0..segments)
        .map(|index| {
            let angle = 2.0 * half_step * index as f64;
            [reach * libm::cos(angle), reach * libm::sin(angle)]
        })
        .collect()
}

fn polygon_at(centre: [f64; 2], offsets: &[[f64; 2]]) -> IntRing {
    offsets
        .iter()
        .map(|o| int_point(centre[0] + o[0], centre[1] + o[1]))
        .collect()
}

fn edge_rectangle(a: [f64; 2], b: [f64; 2], distance_mm: f64) -> Option<IntRing> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = libm::hypot(dx, dy);
    if length == 0.0 {
        return None;
    }
    let (nx, ny) = (dy / length * distance_mm, -dx / length * distance_mm);
    counter_clockwise(vec![
        int_point(a[0] + nx, a[1] + ny),
        int_point(b[0] + nx, b[1] + ny),
        int_point(b[0] - nx, b[1] - ny),
        int_point(a[0] - nx, a[1] - ny),
    ])
}

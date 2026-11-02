//! Exact cross-sections at one height (spec 01 §1.3-1.4): per volume the plane segments,
//! chained into loops, resolved with the non-zero rule (`slicing_mode = regular`; other
//! modes are rejected by the support settings), closed by the slice closing radius and
//! simplified; per object the normal parts united and negative parts subtracted in volume
//! order.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::i_float::int::point::IntPoint;

use super::area::{IntRing, IntRings, difference, int_point, loops_from_rings, resolve};
use super::chain::chain_loops;
use super::offset::close_area;
use super::plane::plane_segments;
use crate::embedded_mesh::{ObjectMesh, Part, PartKind};
use crate::object_print::Loops;

/// Douglas-Peucker tolerance the slicer applies to every section ring whenever `resolution`
/// exceeds 0.001 mm (it does not use the resolution value itself; spec 01 §1.3 step 5).
pub const SECTION_SIMPLIFY_MM: f64 = 0.0025;

/// How a section is finished after the loops are resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionOptions {
    /// `slice_closing_radius` from the project (0.049 mm in every owner project); 0 skips it.
    pub closing_radius_mm: f64,
    /// Ring simplification tolerance, `SECTION_SIMPLIFY_MM` or 0 to keep every vertex.
    pub simplify_mm: f64,
}

impl SectionOptions {
    /// The exact resolved section: no closing, no simplification.
    pub const EXACT: SectionOptions = SectionOptions {
        closing_radius_mm: 0.0,
        simplify_mm: 0.0,
    };

    /// The slicer's finishing for a project with `slice_closing_radius` = `closing_radius_mm`
    /// and `resolution` > 0.001 mm.
    pub fn slicer(closing_radius_mm: f64) -> SectionOptions {
        SectionOptions {
            closing_radius_mm,
            simplify_mm: SECTION_SIMPLIFY_MM,
        }
    }
}

/// The chained loops of one volume at `z`, oriented with the solid on the left, before any
/// fill rule: closed mm loops (first point repeated).
pub fn raw_section_loops(part: &Part, z: f64) -> Loops {
    loops_from_rings(&raw_rings(part, z))
}

/// One volume's section at `z`.
pub fn part_section(part: &Part, z: f64, options: &SectionOptions) -> Loops {
    loops_from_rings(&part_rings(part, z, options))
}

/// The object area at `z`: normal parts united, each negative part removing what the parts
/// before it put there; modifiers, enforcers and blockers never change it (spec 01 §1.4).
pub fn object_section(mesh: &ObjectMesh, z: f64, options: &SectionOptions) -> Loops {
    let mut area = IntRings::new();
    for part in &mesh.parts {
        match part.kind {
            PartKind::Normal => {
                area.extend(part_rings(part, z, options));
                area = resolve(&area, FillRule::NonZero);
            }
            PartKind::Negative => area = difference(&area, &part_rings(part, z, options)),
            _ => {}
        }
    }
    loops_from_rings(&area)
}

pub(crate) fn raw_rings(part: &Part, z: f64) -> IntRings {
    chain_loops(&plane_segments(part, z))
        .into_iter()
        .map(|ring| {
            ring.iter()
                .map(|point| int_point(point[0], point[1]))
                .collect()
        })
        .collect()
}

pub(crate) fn part_rings(part: &Part, z: f64, options: &SectionOptions) -> IntRings {
    let resolved = resolve(&raw_rings(part, z), FillRule::NonZero);
    let closed = close_area(&resolved, options.closing_radius_mm);
    if options.simplify_mm <= 0.0 {
        return closed;
    }
    let tolerance = options.simplify_mm * super::area::NM_PER_MM;
    closed
        .iter()
        .map(|ring| simplify_ring(ring, tolerance))
        .filter(|ring| ring.len() >= 3)
        .collect()
}

/// Douglas-Peucker on a closed ring, run as the open path from the first vertex back to it
/// (so, like the slicer's, the result depends on the start vertex by at most the tolerance).
fn simplify_ring(ring: &IntRing, tolerance: f64) -> IntRing {
    let mut path = ring.clone();
    path.push(ring[0]);
    let mut keep = vec![false; path.len()];
    keep[0] = true;
    keep[path.len() - 1] = true;
    let mut pending = vec![(0usize, path.len() - 1)];
    while let Some((first, last)) = pending.pop() {
        if let Some((index, distance)) = farthest(&path, first, last)
            && distance > tolerance
        {
            keep[index] = true;
            pending.push((first, index));
            pending.push((index, last));
        }
    }
    let mut simplified: IntRing = path
        .iter()
        .zip(&keep)
        .filter_map(|(point, &kept)| kept.then_some(*point))
        .collect();
    simplified.pop();
    simplified
}

/// The vertex strictly between `first` and `last` farthest from their chord.
fn farthest(path: &[IntPoint], first: usize, last: usize) -> Option<(usize, f64)> {
    let (a, b) = (path[first], path[last]);
    let (ax, ay) = (f64::from(a.x), f64::from(a.y));
    let (dx, dy) = (f64::from(b.x) - ax, f64::from(b.y) - ay);
    let length = libm::hypot(dx, dy);
    let mut best: Option<(usize, f64)> = None;
    for (index, point) in path.iter().enumerate().take(last).skip(first + 1) {
        let (px, py) = (f64::from(point.x) - ax, f64::from(point.y) - ay);
        let distance = if length == 0.0 {
            libm::hypot(px, py)
        } else {
            (px * dy - py * dx).abs() / length
        };
        if best.is_none_or(|(_, best_distance)| distance > best_distance) {
            best = Some((index, distance));
        }
    }
    best
}

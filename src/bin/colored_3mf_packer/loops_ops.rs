//! Booleans, areas and round growth of the exterior-only loops the probes produce, on the
//! nanometre grid of `polygon_ops` (the repo's one polygon facade). Inputs are outer rings
//! without holes: the gaps between support lines are no room for a neighbour, so a ring is
//! always read as filled, whatever its winding.

use goat_packer::{
    object_print::Loops,
    polygon_ops::{
        Grid, Join, Rings, offset, ring_doubled_area, simplify, union_all,
        area as rings_area,
    },
};

const GRID: Grid = Grid::Nanometre;

/// Largest distance the round joins of `grown` may fall inside the true offset, mm.
const ARC_TOLERANCE_MM: f64 = 0.005;
/// How far `grown` may move a boundary when it thins out the vertices, mm. Probe unions carry
/// a vertex per support line end (tens of thousands per band), which the masks would pay for
/// at every growth and raster.
const SIMPLIFY_TOLERANCE_MM: f64 = 0.02;
/// What `grown` adds beyond the asked distance so that arcs and thinning never undershoot it.
const COVER_MM: f64 = ARC_TOLERANCE_MM + 2.0 * SIMPLIFY_TOLERANCE_MM;

/// Every ring counter-clockwise, so overlapping rings add up instead of cancelling.
fn filled(loops: &[Vec<goat_packer::Point>]) -> Rings {
    let mut rings = GRID.rings(loops);
    for ring in &mut rings {
        if ring_doubled_area(ring) < 0 {
            ring.reverse();
        }
    }
    rings
}

/// The union of `parts`.
pub fn union(parts: &[&Loops]) -> Loops {
    let regions: Vec<Rings> = parts.iter().map(|loops| filled(loops)).collect();
    GRID.loops(&union_all(&regions))
}

/// Area of the union of the (filled) rings, mm².
pub fn area_mm2(loops: &Loops) -> f64 {
    let region = union_all(&[filled(loops)]);
    GRID.mm2(rings_area(&region))
}

/// `loops` grown by at least `distance_mm` (round joins), thinned to a few vertices.
pub fn grown(loops: &Loops, distance_mm: f64) -> Loops {
    let tolerance = GRID.scaled(SIMPLIFY_TOLERANCE_MM);
    let thin = simplify(&union_all(&[filled(loops)]), tolerance, false);
    let delta = GRID.scaled(distance_mm + COVER_MM);
    let arcs = GRID.scaled(ARC_TOLERANCE_MM);
    let grown = offset(&thin, delta, Join::Round(arcs));
    GRID.loops(&simplify(&grown, tolerance, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use goat_packer::Point;

    fn square(x: f64, y: f64, side: f64) -> Vec<Point> {
        let p = |dx: f64, dy: f64| Point { x: x + dx, y: y + dy };
        vec![p(0.0, 0.0), p(side, 0.0), p(side, side), p(0.0, side), p(0.0, 0.0)]
    }

    #[test]
    fn overlapping_rings_of_either_winding_union_instead_of_cancelling() {
        let mut clockwise = square(5.0, 0.0, 10.0);
        clockwise.reverse();
        let both = union(&[&vec![square(0.0, 0.0, 10.0)], &vec![clockwise]]);
        assert!((area_mm2(&both) - 150.0).abs() < 1e-6, "{}", area_mm2(&both));
    }

    #[test]
    fn growing_never_undershoots_the_asked_distance() {
        let grown = grown(&vec![square(0.0, 0.0, 10.0)], 1.0);
        let xs = grown.iter().flatten().map(|p| p.x);
        let (min, max) = xs.fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
        assert!(min <= -1.0 && max >= 11.0, "{min} {max}");
        assert!(min >= -1.1 && max <= 11.1, "{min} {max}");
    }
}

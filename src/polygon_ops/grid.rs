//! The slicer's integer grid, a flavour property (spec 02 §1.1, design §5.2): Bambu Studio
//! stores coordinates in 10 nm units (32-bit), every Orca build in 1 nm units (64-bit).
//!
//! Lengths become units by truncating the double quotient `mm / scaling factor` toward zero,
//! as the slicers' `scaled` does (spec 01 §0). Because 10⁻⁵ and 10⁻⁶ are not exact in binary,
//! some round values lose a unit (2.0 mm is 199 999 units in B); computing the same double
//! quotient reproduces that exactly. Coordinates (section points, mesh vertices) are rounded
//! half up instead (spec 01 §1.3 step 1).

use serde::{Deserialize, Serialize};

use super::{IntPoint, Ring, Rings};
use crate::geometry::Point;
use crate::object_print::Loops;

/// Largest coordinate magnitude in grid units: `i_overlay` computes crossings exactly only
/// below 2^30 (`split/cross_solver.rs`), i.e. ±1073 mm on the 1 nm grid, ±10.7 m on 10 nm.
pub const MAX_COORDINATE: i32 = (1 << 30) - 1;

/// The integer grid one slicer flavour computes supports on.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Grid {
    /// Bambu Studio: 1 unit = 10 nm, scaling factor 10⁻⁵ (B `libslic3r.h:40,58`).
    TenNanometre,
    /// OrcaSlicer and Snapmaker Orca: 1 unit = 1 nm, scaling factor 10⁻⁶ (O `libslic3r.h:60`).
    Nanometre,
}

impl Grid {
    /// mm per unit, the exact double the slicers divide by.
    pub const fn scaling_factor(self) -> f64 {
        match self {
            Grid::TenNanometre => 0.00001,
            Grid::Nanometre => 0.000001,
        }
    }

    /// A length in units, truncated toward zero (`scaled<coord_t>`).
    pub fn length(self, mm: f64) -> i64 {
        (mm / self.scaling_factor()) as i64
    }

    /// A length in units without truncation (`scale_`, a double).
    pub fn scaled(self, mm: f64) -> f64 {
        mm / self.scaling_factor()
    }

    /// Units back to mm (`unscale`).
    pub fn mm(self, units: f64) -> f64 {
        units * self.scaling_factor()
    }

    /// Area in units² back to mm².
    pub fn mm2(self, units2: f64) -> f64 {
        units2 * self.scaling_factor() * self.scaling_factor()
    }

    /// A coordinate in units, rounded half up and kept inside `MAX_COORDINATE`.
    pub fn coordinate(self, mm: f64) -> i32 {
        let units = libm::floor(mm / self.scaling_factor() + 0.5);
        units.clamp(-f64::from(MAX_COORDINATE), f64::from(MAX_COORDINATE)) as i32
    }

    pub fn point(self, x: f64, y: f64) -> IntPoint {
        IntPoint::new(self.coordinate(x), self.coordinate(y))
    }

    /// A ring from mm points; a repeated closing point is dropped.
    pub fn ring(self, points: &[Point]) -> Ring {
        let mut ring: Ring = points
            .iter()
            .map(|point| self.point(point.x, point.y))
            .collect();
        while ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        ring
    }

    /// Rings from closed mm loops, each kept as given (orientation included).
    pub fn rings(self, loops: &[Vec<Point>]) -> Rings {
        loops.iter().map(|ring| self.ring(ring)).collect()
    }

    /// Closed mm loops (first point repeated), the `object_print::Loops` form.
    pub fn loops(self, rings: &[Ring]) -> Loops {
        rings
            .iter()
            .filter(|ring| ring.len() >= 3)
            .map(|ring| {
                let mut points: Vec<Point> = ring
                    .iter()
                    .map(|point| Point {
                        x: self.mm(f64::from(point.x)),
                        y: self.mm(f64::from(point.y)),
                    })
                    .collect();
                points.push(points[0]);
                points
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Grid;

    #[test]
    fn truncation_loses_a_unit_where_the_slicer_does() {
        // Spec 01 §0 / 02 §1.1: 2.0 mm -> 199 999, 0.03 mm -> 2 999, 7.5 mm -> 749 999 in B.
        assert_eq!(Grid::TenNanometre.length(2.0), 199_999);
        assert_eq!(Grid::TenNanometre.length(0.03), 2_999);
        assert_eq!(Grid::TenNanometre.length(7.5), 749_999);
        assert_eq!(Grid::Nanometre.length(2.0), 2_000_000);
        assert_eq!(Grid::Nanometre.length(0.28), 280_000);
    }

    #[test]
    fn coordinates_round_half_up() {
        assert_eq!(Grid::Nanometre.coordinate(0.0000005), 1);
        assert_eq!(Grid::Nanometre.coordinate(-0.0000005), 0);
        assert_eq!(Grid::TenNanometre.coordinate(1.0), 100_000);
    }
}

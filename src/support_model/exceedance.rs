//! How far an observed support layer lies outside a predicted one (design §5.5 calibration
//! rule): the area outside and the largest distance of any observed boundary point from the
//! prediction. Measured on outer boundaries, so toolpath details inside a footprint do not
//! count.

use crate::object_print::Loops;
use crate::polygon_ops::{Grid, Rings, area, difference, distance_to_boundary};

/// Observed material outside a prediction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Exceedance {
    pub outside_mm2: f64,
    pub max_mm: f64,
}

impl Exceedance {
    /// The worse of two measurements, per field.
    pub fn worst(self, other: Exceedance) -> Exceedance {
        Exceedance {
            outside_mm2: self.outside_mm2.max(other.outside_mm2),
            max_mm: self.max_mm.max(other.max_mm),
        }
    }
}

/// The measurement grid: nanometres, exact for any plate.
const UNITS: Grid = Grid::Nanometre;

fn rings_of(loops: &Loops) -> Rings {
    UNITS.rings(loops)
}

/// What of `observed` lies outside `predicted`.
pub fn exceedance(observed: &Loops, predicted: &Loops) -> Exceedance {
    let (seen, expected) = (rings_of(observed), rings_of(predicted));
    let outside = difference(&seen, &expected);
    if outside.is_empty() {
        return Exceedance::default();
    }
    let far = outside
        .iter()
        .flatten()
        .map(|&point| distance_to_boundary(&expected, point))
        .fold(0.0, f64::max);
    Exceedance {
        outside_mm2: UNITS.mm2(area(&outside)),
        max_mm: UNITS.mm(far),
    }
}

/// The area of loops in mm² under the non-zero rule.
pub fn area_mm2(loops: &Loops) -> f64 {
    UNITS.mm2(area(&crate::polygon_ops::resolve(
        &rings_of(loops),
        crate::polygon_ops::FillRule::NonZero,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point;

    fn square(x0: f64, x1: f64) -> Loops {
        let p = |x: f64, y: f64| Point { x, y };
        vec![vec![
            p(x0, 0.0),
            p(x1, 0.0),
            p(x1, 10.0),
            p(x0, 10.0),
            p(x0, 0.0),
        ]]
    }

    #[test]
    fn material_beyond_the_prediction_is_measured() {
        let measured = exceedance(&square(0.0, 12.0), &square(0.0, 10.0));
        assert!((measured.outside_mm2 - 20.0).abs() < 1e-6);
        assert!((measured.max_mm - 2.0).abs() < 1e-6);
        assert_eq!(
            exceedance(&square(1.0, 9.0), &square(0.0, 10.0)),
            Exceedance::default()
        );
    }
}

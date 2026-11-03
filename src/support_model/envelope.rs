//! The support envelope (design §5.5): the union of the ensemble's realizations per kind and
//! band, grown by the margin δ that covers what the model does not reproduce. δ describes the
//! slicer, not the printer, and is not the pair gap between neighbours.

use crate::object_print::{Loops, Margins, Realization};
use crate::polygon_ops::{Grid, Join, Rings, islands, offset, union_all};

/// The working grid of loop operations: nanometres cover any plate exactly.
const UNITS: Grid = Grid::Nanometre;

/// Arc tolerance of the margin growth, mm.
const ARC_TOLERANCE_MM: f64 = 0.01;

/// The union of `members` per kind and band with every kind grown by its margin: the first
/// layer's base by δ_L1, the bands (support, above the base, interface) by δ_band.
pub fn envelope(members: &[&Realization], margins: Margins) -> Realization {
    let bands = members
        .iter()
        .map(|member| member.support.len())
        .max()
        .unwrap_or(0);
    let band_union = |pick: &dyn Fn(&Realization) -> &[Loops]| -> Vec<Loops> {
        (0..bands)
            .map(|band| {
                let parts: Vec<Rings> = members
                    .iter()
                    .filter_map(|member| pick(member).get(band))
                    .map(rings_of)
                    .collect();
                loops_of(&grown(union_all(parts.iter()), margins.band_mm))
            })
            .collect()
    };
    let base: Vec<Rings> = members
        .iter()
        .map(|member| rings_of(&member.base))
        .collect();
    let landing: Vec<Rings> = members
        .iter()
        .map(|member| rings_of(&member.landing))
        .collect();
    Realization {
        member: 0,
        landing: loops_of(&union_all(landing.iter())),
        base: loops_of(&grown(union_all(base.iter()), margins.first_layer_mm)),
        above_base: band_union(&|member| &member.above_base),
        support: band_union(&|member| &member.support),
        interface: band_union(&|member| &member.interface),
    }
}

/// Holes filled: the gaps inside a footprint are no room for a neighbour.
fn rings_of(loops: &Loops) -> Rings {
    exterior(UNITS.rings(loops))
}

fn exterior(rings: Rings) -> Rings {
    islands(&rings)
        .into_iter()
        .filter_map(|island| island.into_iter().next())
        .collect()
}

fn grown(rings: Rings, margin_mm: f64) -> Rings {
    if margin_mm <= 0.0 || rings.is_empty() {
        return exterior(rings);
    }
    exterior(offset(
        &exterior(rings),
        UNITS.scaled(margin_mm),
        Join::Round(UNITS.scaled(ARC_TOLERANCE_MM)),
    ))
}

fn loops_of(rings: &Rings) -> Loops {
    UNITS.loops(&exterior(rings.clone()))
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

    fn member(base: Loops) -> Realization {
        Realization {
            base: base.clone(),
            above_base: vec![Loops::new(), base.clone()],
            support: vec![base.clone(), base.clone()],
            interface: vec![Loops::new(), Loops::new()],
            ..Realization::default()
        }
    }

    #[test]
    fn the_envelope_unions_the_members_and_grows_by_the_margins() {
        let (a, b) = (member(square(0.0, 10.0)), member(square(20.0, 30.0)));
        let margins = Margins {
            first_layer_mm: 1.0,
            band_mm: 2.0,
        };
        let merged = envelope(&[&a, &b], margins);
        let width = |loops: &Loops| {
            let xs = loops.iter().flatten().map(|p| p.x);
            xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min)
        };
        assert!(
            (width(&merged.base) - 32.0).abs() < 0.05,
            "{}",
            width(&merged.base)
        );
        assert!((width(&merged.support[1]) - 34.0).abs() < 0.05);
        assert!(merged.interface[1].is_empty());
    }
}

//! Lattice sites on the plate: which sites of a lattice a copy may take (exact zone test) and
//! how far the sites that cannot are from being able to.

use super::lattice::{Params, offset, point, site_range};
use super::oracle::Oracle;
use super::seed::slot_base;
use crate::geometry::{Point, Rect};

/// Sites this far (mm) beyond the box of feasible origins still count towards the search's
/// pull; farther ones are hopeless for a small move. The pull of a site falls to zero at
/// this distance, so a site entering the range never makes the sum jump.
const PULL_RANGE_MM: f64 = 12.0;

/// What a layout may use: the lattice classes and the box the origins of each can lie in.
pub(super) struct Field<'a> {
    pub(super) oracle: &'a Oracle<'a>,
    pub(super) classes: &'a [usize],
    /// Per class slot: box around every feasible origin, plate mm.
    pub(super) regions: &'a [Rect],
}

/// The sites of a lattice: those a copy may take, and the pull of those it may not.
pub(super) struct Sites {
    /// `(slot, position)`, rows from the bottom.
    pub(super) usable: Vec<(usize, Point)>,
    /// Minus the sum over unusable sites within `PULL_RANGE_MM` of the box of their
    /// closeness to it (1 on the box, 0 at the range, squared): a smooth stand-in for "how
    /// many sites a small move would bring in". Lower is better.
    pub(super) pull: f64,
}

impl Field<'_> {
    /// The sites for parameters `x`. A site inside its class's box is tested exactly; one
    /// outside cannot be usable (the box holds every feasible origin) and only pulls.
    pub(super) fn sites(&self, x: &Params) -> Sites {
        let mut usable: Vec<(usize, Point)> = Vec::new();
        let mut pull = 0.0;
        for (slot, &class) in self.classes.iter().enumerate() {
            let origin = slot_base(x, slot);
            let region = self.regions[slot];
            let Some((i0, i1, j0, j1)) = site_range(x, origin, region.grown(PULL_RANGE_MM)) else {
                continue;
            };
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let d = offset(x, i, j, false);
                    let at = point(origin.x + d.x, origin.y + d.y);
                    let miss = distance_outside(region, at);
                    if miss > 0.0 {
                        pull -= closeness(miss);
                    } else if self.oracle.zones_clear(class, at) {
                        usable.push((slot, at));
                    }
                }
            }
        }
        usable.sort_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.1.x.total_cmp(&b.1.x)));
        Sites { usable, pull }
    }
}

/// How far `p` lies outside `region`, summed over both axes, mm.
fn distance_outside(region: Rect, p: Point) -> f64 {
    let over = |v: f64, lo: f64, hi: f64| (lo - v).max(v - hi).max(0.0);
    over(p.x, region.min_x, region.max_x) + over(p.y, region.min_y, region.max_y)
}

/// 1 at the box, 0 from `PULL_RANGE_MM` on; squared so the nearest sites pull hardest.
fn closeness(miss: f64) -> f64 {
    (1.0 - miss / PULL_RANGE_MM).max(0.0).powi(2)
}

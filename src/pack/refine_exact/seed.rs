//! A raster lattice candidate read back as continuous parameters: the lattice the member
//! built (basis, class offset, phase) and the copies the edge fill added around it.

use super::lattice::{PHASE, Params, coordinates, offset, phase, point};
use crate::geometry::Point;
use crate::kind_masks::OriginGrid;
use crate::pack::lattice::hnf::Hnf;
use crate::pack::select::{Candidate, Member};

/// A copy that is not on the lattice, anchored to the lattice site nearest to it so it can
/// follow the site when the lattice moves.
#[derive(Clone, Copy, Debug)]
pub(super) struct Extra {
    pub(super) slot: usize,
    pub(super) i: i32,
    pub(super) j: i32,
    /// From that site to the copy, mm.
    pub(super) delta: Point,
}

#[derive(Clone, Debug)]
pub(crate) struct Seed {
    pub(super) anchor: Option<usize>,
    /// Rotation classes of the lattice: one, or two for a double lattice.
    pub(super) classes: Vec<usize>,
    /// The raster lattice in mm, phase included.
    pub(super) params: Params,
    pub(super) extras: Vec<Extra>,
    /// Copies of the candidate (lattice and fill).
    pub(super) count: usize,
}

impl Seed {
    /// The lattice that built `candidate`, or `None` for a member without one.
    pub(super) fn decode(candidate: &Candidate, grid: &OriginGrid) -> Option<Seed> {
        let detail = &candidate.key.tie.detail;
        let member = candidate.key.tie.member;
        let (classes, hnf, cosets) = match (member, detail.as_slice()) {
            (Member::Lattice, &[c, s, t, h, coset, _]) => (vec![c], (s, t, h), vec![coset]),
            (Member::DoubleLattice, &[a, b, s, t, h, ca, cb, _]) => {
                (vec![a, b], (s, t, h), vec![ca, cb])
            }
            _ => return None,
        };
        let hnf = Hnf {
            s: i32::try_from(hnf.0).ok()?,
            t: i32::try_from(hnf.1).ok()?,
            h: i32::try_from(hnf.2).ok()?,
        };
        let classes: Vec<usize> = classes
            .iter()
            .map(|&c| usize::try_from(c).ok())
            .collect::<Option<_>>()?;
        let cosets: Vec<usize> = cosets
            .iter()
            .map(|&c| usize::try_from(c).ok())
            .collect::<Option<_>>()?;
        let res = grid.resolution;
        let [u, v] = hnf.reduced();
        let mut params: Params = [0.0; 8];
        params[..4].copy_from_slice(&[
            f64::from(u.0) * res,
            f64::from(u.1) * res,
            f64::from(v.0) * res,
            f64::from(v.1) * res,
        ]);
        // (slot, position) of every copy; a copy is on the lattice when it sits in its
        // class's coset.
        let mut sites: Vec<Vec<Point>> = vec![Vec::new(); classes.len()];
        let mut loose: Vec<(usize, Point)> = Vec::new();
        for p in &candidate.placements {
            let (x, y) = p.mm(grid);
            let at = point(x, y);
            let slot = classes.iter().position(|&c| c == p.class);
            match slot {
                Some(s) if hnf.coset_of(p.x, p.y) == cosets[s] => sites[s].push(at),
                Some(s) => loose.push((s, at)),
                None => return None,
            }
        }
        if sites[0].is_empty() {
            return None;
        }
        let centroid = centroid(sites.iter().flatten());
        let base = nearest(&sites[0], centroid)?;
        if classes.len() == 2 {
            let second = nearest(&sites[1], base)?;
            params[4] = second.x - base.x;
            params[5] = second.y - base.y;
        }
        params[PHASE] = base.x;
        params[PHASE + 1] = base.y;
        let extras = loose
            .iter()
            .map(|&(slot, at)| anchor_extra(&params, slot, at))
            .collect();
        Some(Seed {
            anchor: candidate.anchor,
            classes,
            params,
            extras,
            count: candidate.placements.len(),
        })
    }

    pub(super) fn dof(&self) -> usize {
        if self.classes.len() == 2 { 6 } else { 4 }
    }
}

/// Where the copies of `slot` sit: the first class at the phase, the second shifted by `w`.
pub(super) fn slot_base(x: &Params, slot: usize) -> Point {
    let base = phase(x);
    if slot == 0 {
        base
    } else {
        point(base.x + x[4], base.y + x[5])
    }
}

fn anchor_extra(x: &Params, slot: usize, at: Point) -> Extra {
    let origin = slot_base(x, slot);
    let (i, j) = coordinates(x, origin, at);
    let (i, j) = (i.round() as i32, j.round() as i32);
    let site = offset(x, i, j, false);
    Extra {
        slot,
        i,
        j,
        delta: point(at.x - origin.x - site.x, at.y - origin.y - site.y),
    }
}

fn centroid<'a>(points: impl Iterator<Item = &'a Point>) -> Point {
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    for p in points {
        sx += p.x;
        sy += p.y;
        n += 1.0;
    }
    point(sx / n, sy / n)
}

/// The first of `points` nearest to `to` (ties: the earlier one).
fn nearest(points: &[Point], to: Point) -> Option<Point> {
    let d = |p: &Point| (p.x - to.x).powi(2) + (p.y - to.y).powi(2);
    points.iter().copied().min_by(|a, b| d(a).total_cmp(&d(b)))
}

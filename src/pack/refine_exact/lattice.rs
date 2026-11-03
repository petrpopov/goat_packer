//! A lattice with continuous parameters and its exact admissibility.
//!
//! The parameters are `x = [ux, uy, vx, vy, wx, wy, ox, oy]`: the basis `u`, `v`, for a
//! double lattice the offset `w` of the second class's copies from the first's, and the
//! phase `o`. Copies of the first class sit at `o + i u + j v`, of the second at
//! `o + w + i u + j v`. The phase does not enter the pair rules (a translation of the whole
//! layout), only the plate zones, so admissibility is a finite list of pair shifts that
//! depends on the first six parameters.

use super::oracle::Oracle;
use crate::geometry::{Point, Rect};

/// Lattice steps a pair of copies can still be in contact over. A shape reaching further than
/// three cells is not a lattice problem; the final gate would catch the pair anyway.
const REACH_STEPS: i32 = 3;

pub(super) type Params = [f64; 8];

/// Parameter indices of the phase.
pub(super) const PHASE: usize = 6;

pub(super) fn phase(x: &Params) -> Point {
    point(x[PHASE], x[PHASE + 1])
}

pub(super) fn point(x: f64, y: f64) -> Point {
    Point { x, y }
}

/// Cell area, mm².
pub(super) fn det(x: &Params) -> f64 {
    (x[0] * x[3] - x[1] * x[2]).abs()
}

/// `i u + j v` (plus `w` when asked) for the parameters `x`.
pub(super) fn offset(x: &Params, i: i32, j: i32, with_w: bool) -> Point {
    let (i, j) = (f64::from(i), f64::from(j));
    let (wx, wy) = if with_w { (x[4], x[5]) } else { (0.0, 0.0) };
    point(i * x[0] + j * x[2] + wx, i * x[1] + j * x[3] + wy)
}

/// One pair shift of the lattice that must stay clear.
#[derive(Clone, Copy)]
struct Contact {
    a: usize,
    b: usize,
    i: i32,
    j: i32,
    with_w: bool,
}

/// The pair shifts of a lattice of one or two classes. A violated contact moves to the front
/// (the next trial near it tends to break the same one), so `admissible` usually fails fast;
/// the order never changes the answer.
pub(super) struct Contacts {
    list: Vec<Contact>,
}

impl Contacts {
    /// `classes` has one entry (single lattice) or two (double lattice).
    pub(super) fn new(classes: &[usize]) -> Contacts {
        let steps = REACH_STEPS;
        let mut list = Vec::new();
        for &class in classes {
            for i in 0..=steps {
                for j in -steps..=steps {
                    // One of each +-d: the rules are symmetric under swapping the copies.
                    if i > 0 || j > 0 {
                        list.push(Contact {
                            a: class,
                            b: class,
                            i,
                            j,
                            with_w: false,
                        });
                    }
                }
            }
        }
        if let [a, b] = *classes {
            for i in -steps..=steps {
                for j in -steps..=steps {
                    list.push(Contact {
                        a,
                        b,
                        i,
                        j,
                        with_w: true,
                    });
                }
            }
        }
        Contacts { list }
    }

    /// Every contact clear for the parameters `x`.
    pub(super) fn admissible(&mut self, oracle: &Oracle, x: &Params) -> bool {
        for k in 0..self.list.len() {
            let c = self.list[k];
            if !oracle.pair_clear(c.a, c.b, offset(x, c.i, c.j, c.with_w)) {
                self.list[..=k].rotate_right(1);
                return false;
            }
        }
        true
    }
}

/// The integer ranges `(i0..=i1, j0..=j1)` of lattice sites `base + i u + j v` that can lie in
/// `region`, from the region's corners in lattice coordinates.
pub(super) fn site_range(x: &Params, base: Point, region: Rect) -> Option<(i32, i32, i32, i32)> {
    let d = x[0] * x[3] - x[1] * x[2];
    if d.abs() < 1e-9 {
        return None;
    }
    let corners = [
        (region.min_x, region.min_y),
        (region.max_x, region.min_y),
        (region.min_x, region.max_y),
        (region.max_x, region.max_y),
    ];
    let mut range = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for (cx, cy) in corners {
        let (px, py) = (cx - base.x, cy - base.y);
        let i = (px * x[3] - py * x[2]) / d;
        let j = (x[0] * py - x[1] * px) / d;
        range = (
            range.0.min(i),
            range.1.max(i),
            range.2.min(j),
            range.3.max(j),
        );
    }
    Some((
        range.0.floor() as i32,
        range.1.ceil() as i32,
        range.2.floor() as i32,
        range.3.ceil() as i32,
    ))
}

/// Lattice coordinates `(i, j)` of `p` relative to `base`, real valued.
pub(super) fn coordinates(x: &Params, base: Point, p: Point) -> (f64, f64) {
    let d = x[0] * x[3] - x[1] * x[2];
    let (px, py) = (p.x - base.x, p.y - base.y);
    ((px * x[3] - py * x[2]) / d, (x[0] * py - x[1] * px) / d)
}

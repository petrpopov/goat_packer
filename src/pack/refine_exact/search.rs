//! The count-driven search over a lattice's continuous parameters.
//!
//! Shrinking the cell alone loses copies: a move that does not keep the rows fitted to the
//! plate edges pushes the outer sites off it (measured on the goat plates: the minimum-area
//! lattice holds 53 of the raster lattice's 58 on U1). So the objective is lexicographic:
//! more usable sites; then less pull of the sites just outside the plate (a smooth measure
//! of how close the next site is to fitting); then a smaller cell. A move is taken only if it
//! keeps every lattice contact clear and improves that tuple.
//!
//! A pattern search over the basis, the offset and the phase: directions `+-e_k` and
//! `+-e_k +-e_l`, each repeated with a doubling step while it keeps improving. Sequential
//! and in fixed order, so the result is a function of the start, the oracle and the budget.

use super::lattice::{Contacts, PHASE, Params, det};
use super::sites::Field;

/// Step sizes of the passes, mm.
const STEPS: [f64; 5] = [0.5, 0.125, 0.03125, 0.0078125, 0.001953125];
/// Evaluations (admissibility test plus site count) per seed. A fixed budget, so effort never
/// changes what a seed finds.
const EVALS: usize = 2500;
/// Largest doubling of a step along one direction.
const DOUBLINGS: usize = 6;
/// A pull or area change below this is not progress.
const EPS: f64 = 1e-9;

/// The tuple the search improves.
#[derive(Clone, Copy, Debug)]
pub(super) struct Score {
    pub(super) count: usize,
    pub(super) pull: f64,
    area: f64,
}

impl Score {
    fn beats(&self, other: &Score) -> bool {
        if self.count != other.count {
            return self.count > other.count;
        }
        if (self.pull - other.pull).abs() > EPS {
            return self.pull < other.pull;
        }
        self.area < other.area - EPS
    }
}

/// The score of `x`, or `None` when a lattice contact is violated.
fn score(field: &Field, contacts: &mut Contacts, x: &Params) -> Option<Score> {
    if !contacts.admissible(field.oracle, x) {
        return None;
    }
    let sites = field.sites(x);
    Some(Score {
        count: sites.usable.len(),
        pull: sites.pull,
        area: det(x),
    })
}

/// Parameter indices the search moves: basis, phase, and the offset of a double lattice.
fn free_indices(dof: usize) -> Vec<usize> {
    let mut free: Vec<usize> = vec![0, 1, 2, 3, PHASE, PHASE + 1];
    if dof == 6 {
        free.extend([4, 5]);
    }
    free
}

fn directions(free: &[usize]) -> Vec<Params> {
    let mut dirs: Vec<Params> = Vec::new();
    for &k in free {
        for s in [1.0, -1.0] {
            let mut d = [0.0; 8];
            d[k] = s;
            dirs.push(d);
        }
    }
    for (a, &k) in free.iter().enumerate() {
        for &l in &free[a + 1..] {
            for (s, t) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                let mut d = [0.0; 8];
                d[k] = s;
                d[l] = t;
                dirs.push(d);
            }
        }
    }
    dirs
}

fn moved(x: &Params, d: &Params, t: f64) -> Params {
    let mut y = *x;
    for k in 0..8 {
        y[k] += t * d[k];
    }
    y
}

/// The search from `start` (admissible) and from the same lattice with a site on each corner
/// of the first class's box: the pull is a local guide, and a lattice fitted to a corner
/// reaches fits the centred phase cannot (two sides must both gain at once there). The best
/// tuple wins, the earlier start on a tie, so the result is never worse than `start`'s.
/// `dof` is 4 for a single lattice, 6 for a double one.
pub(super) fn search(
    field: &Field,
    contacts: &mut Contacts,
    start: Params,
    dof: usize,
) -> (Params, Score) {
    let region = field.regions[0];
    let corners = [
        (region.min_x, region.min_y),
        (region.max_x, region.max_y),
        (region.min_x, region.max_y),
        (region.max_x, region.min_y),
    ];
    let mut best = descend(field, contacts, start, dof);
    for (x, y) in corners {
        let mut from = start;
        from[PHASE] = x;
        from[PHASE + 1] = y;
        let found = descend(field, contacts, from, dof);
        if found.1.beats(&best.1) {
            best = found;
        }
    }
    best
}

fn descend(field: &Field, contacts: &mut Contacts, start: Params, dof: usize) -> (Params, Score) {
    let mut x = start;
    let mut best = score(field, contacts, &x).expect("the start is admissible");
    let dirs = directions(&free_indices(dof));
    let mut evals = 0;
    for step in STEPS {
        loop {
            let mut improved = false;
            for d in &dirs {
                let mut t = step;
                for _ in 0..=DOUBLINGS {
                    if evals >= EVALS {
                        return (x, best);
                    }
                    evals += 1;
                    let y = moved(&x, d, t);
                    match score(field, contacts, &y) {
                        Some(s) if s.beats(&best) => {
                            x = y;
                            best = s;
                            improved = true;
                            t *= 2.0;
                        }
                        _ => break,
                    }
                }
            }
            if !improved {
                break;
            }
        }
    }
    (x, best)
}

//! Booleans on the grid: every operand is read with the non-zero rule, as the slicers'
//! `union_`, `intersection`, `diff` do (spec 02 §1.1), and the result comes back resolved
//! (outer rings counter-clockwise, holes clockwise, grouped per island).

use i_overlay::core::{
    fill_rule::FillRule,
    overlay::{IntOverlayOptions, Overlay, ShapeType},
    overlay_rule::OverlayRule,
    solver::{Precision, Solver, Strategy},
};

use std::panic::AssertUnwindSafe;

use super::query::{IntBox, bbox, ring_doubled_area};
use super::{Island, Ring, Rings, flatten};

/// No parallel sorting inside i_overlay: one support-model member runs on one thread and
/// must give the same bits on every machine (design §5.2).
const SOLVER: Solver = Solver {
    strategy: Strategy::Auto,
    precision: Precision::HIGH,
    multithreading: None,
};

/// `rings` resolved under `fill`.
pub fn resolve(rings: &[Ring], fill: FillRule) -> Rings {
    flatten(overlay(rings, &[], OverlayRule::Subject, fill))
}

/// `rings` resolved under the non-zero rule into polygons with holes (`union_ex`).
pub fn islands(rings: &[Ring]) -> Vec<Island> {
    overlay(rings, &[], OverlayRule::Subject, FillRule::NonZero)
}

pub fn union(a: &[Ring], b: &[Ring]) -> Rings {
    flatten(union_islands(a, b))
}

pub fn union_islands(a: &[Ring], b: &[Ring]) -> Vec<Island> {
    overlay(a, b, OverlayRule::Union, FillRule::NonZero)
}

/// Rings per leaf of `union_many`: below it one sweep is cheapest.
const UNION_LEAF: usize = 16;

/// The sweep of `union_many`: the list solver was the fastest on piles of tube slices
/// (P1S layers: list 11.2 s, auto 13.1 s, fragment 13.1 s, tree 23 s).
const UNION_SOLVER: Solver = Solver {
    strategy: Strategy::List,
    ..SOLVER
};

/// The non-zero union of many rings as a balanced tree of unions (each half resolved first,
/// in parallel). One sweep over a pile of nearly coincident polygons (the same tube slice of
/// many ensemble graphs, a few units apart) computes every crossing inside the pile: 22 k
/// rings of one P1S layer took 170 s, the tree 1.6 s. Splitting the rings by position
/// instead of by input order was no faster: nearly coincident rings each keep a piece of the
/// union's scalloped outline, so the outlines do not shrink up the tree either way.
pub fn union_many(rings: &[Ring]) -> Rings {
    let run = |rings: &[Ring]| {
        flatten(overlay_with(
            rings,
            &[],
            OverlayRule::Subject,
            FillRule::NonZero,
            UNION_SOLVER,
        ))
    };
    if rings.len() <= UNION_LEAF {
        return run(rings);
    }
    let (low, high) = rings.split_at(rings.len() / 2);
    let (low, high) = rayon::join(|| union_many(low), || union_many(high));
    run(&[low, high].concat())
}

/// The union of several regions in one pass.
pub fn union_all<'a>(regions: impl IntoIterator<Item = &'a Rings>) -> Rings {
    let rings: Rings = regions.into_iter().flatten().cloned().collect();
    resolve(&rings, FillRule::NonZero)
}

pub fn intersection(a: &[Ring], b: &[Ring]) -> Rings {
    flatten(intersection_islands(a, b))
}

pub fn intersection_islands(a: &[Ring], b: &[Ring]) -> Vec<Island> {
    if disjoint_boxes(a, b) {
        return Vec::new();
    }
    overlay(a, b, OverlayRule::Intersect, FillRule::NonZero)
}

/// `a \ b`.
pub fn difference(a: &[Ring], b: &[Ring]) -> Rings {
    flatten(difference_islands(a, b))
}

pub fn difference_islands(a: &[Ring], b: &[Ring]) -> Vec<Island> {
    if disjoint_boxes(a, b) {
        return overlay(a, &[], OverlayRule::Subject, FillRule::NonZero);
    }
    overlay(a, b, OverlayRule::Difference, FillRule::NonZero)
}

pub fn xor(a: &[Ring], b: &[Ring]) -> Rings {
    flatten(overlay(a, b, OverlayRule::Xor, FillRule::NonZero))
}

/// The two regions share area. The slicers' `overlaps` also counts a boundary of one
/// crossing the other; that differs only for regions that merely touch, where no area is
/// shared (documented backend difference, spec 02 §6 item 2).
pub fn overlaps(a: &[Ring], b: &[Ring]) -> bool {
    !intersection_islands(a, b).is_empty()
}

fn disjoint_boxes(a: &[Ring], b: &[Ring]) -> bool {
    match (bbox(a), bbox(b)) {
        (Some(first), Some(second)) => !IntBox::overlap(&first, &second),
        _ => true,
    }
}

fn overlay(subject: &[Ring], clip: &[Ring], rule: OverlayRule, fill: FillRule) -> Vec<Island> {
    overlay_with(subject, clip, rule, fill, SOLVER)
}

/// i_overlay 4.5 panics (`bind/solver.rs`: a hole whose anchor finds no enclosing contour,
/// index `usize::MAX`) on some piles of degenerate sliver rings: triangles a tenth of a
/// millimetre long and under a nanometre wide, which `union_many` meets in the drawn
/// supports (first seen on a U1 goat at 30 degrees: 47 of a leaf's 68 rings were such
/// slivers; 12 of them alone reproduce it under every strategy; i_overlay 9 does not
/// panic). Inputs that do not panic keep their exact result (no filtering up front, so
/// cached unions stay byte-identical); a panicking one is retried once without the
/// slivers, which hold no area, and the retry is reported on stderr.
fn overlay_with(subject: &[Ring], clip: &[Ring], rule: OverlayRule, fill: FillRule, solver: Solver) -> Vec<Island> {
    let run = |subject: &[Ring], clip: &[Ring]| {
        std::panic::catch_unwind(AssertUnwindSafe(|| run_overlay(subject, clip, rule, fill, solver)))
    };
    match run(subject, clip) {
        Ok(islands) => islands,
        Err(payload) => {
            let subject_kept = without_slivers(subject);
            let clip_kept = without_slivers(clip);
            let dropped = subject.len() + clip.len() - subject_kept.len() - clip_kept.len();
            eprintln!(
                "polygon overlay: i_overlay panicked; retrying without {dropped} degenerate \
                 sliver ring(s) narrower than {SLIVER_WIDTH_UNITS} grid unit"
            );
            if dropped == 0 {
                std::panic::resume_unwind(payload);
            }
            match run(&subject_kept, &clip_kept) {
                Ok(islands) => islands,
                Err(_) => std::panic::resume_unwind(payload),
            }
        }
    }
}

fn run_overlay(subject: &[Ring], clip: &[Ring], rule: OverlayRule, fill: FillRule, solver: Solver) -> Vec<Island> {
    let edges: usize = subject.iter().chain(clip).map(Vec::len).sum();
    if edges == 0 {
        return Vec::new();
    }
    let mut overlay = Overlay::new_custom(edges, IntOverlayOptions::default(), solver);
    for ring in subject.iter().filter(|ring| ring.len() >= 3) {
        overlay.add_contour(ring, ShapeType::Subject);
    }
    for ring in clip.iter().filter(|ring| ring.len() >= 3) {
        overlay.add_contour(ring, ShapeType::Clip);
    }
    overlay.overlay(rule, fill)
}

/// A ring narrower than this on average (twice its area over its perimeter, in grid units)
/// is a sliver: one unit is 1 nm (Orca) or 10 nm (Bambu).
const SLIVER_WIDTH_UNITS: f64 = 1.0;

/// `rings` without the slivers (and without rings of fewer than three points).
fn without_slivers(rings: &[Ring]) -> Vec<Ring> {
    rings
        .iter()
        .filter(|ring| ring.len() >= 3)
        .filter(|ring| {
            let perimeter: f64 = ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .map(|(a, b)| (f64::from(b.x) - f64::from(a.x)).hypot(f64::from(b.y) - f64::from(a.y)))
                .sum();
            (ring_doubled_area(ring) as f64).abs() >= SLIVER_WIDTH_UNITS * perimeter
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::polygon_ops::{IntPoint, area};

    fn square(x: i32, y: i32, size: i32) -> Ring {
        vec![
            IntPoint::new(x, y),
            IntPoint::new(x + size, y),
            IntPoint::new(x + size, y + size),
            IntPoint::new(x, y + size),
        ]
    }

    #[test]
    fn set_operations_have_the_expected_areas() {
        let a = vec![square(0, 0, 10)];
        let b = vec![square(5, 5, 10)];
        assert_eq!(area(&union(&a, &b)), 175.0);
        assert_eq!(area(&intersection(&a, &b)), 25.0);
        assert_eq!(area(&difference(&a, &b)), 75.0);
        assert_eq!(area(&xor(&a, &b)), 150.0);
        assert!(overlaps(&a, &b));
        assert!(!overlaps(&a, &[square(20, 0, 5)]));
    }

    #[test]
    fn a_clockwise_ring_alone_is_still_an_area_under_non_zero() {
        let mut ring = square(0, 0, 10);
        ring.reverse();
        let resolved = resolve(&[ring], FillRule::NonZero);
        assert_eq!(area(&resolved), 100.0);
    }

    #[test]
    fn islands_keep_holes_with_their_outer_ring() {
        let mut hole = square(2, 2, 6);
        hole.reverse();
        let found = islands(&[square(0, 0, 10), hole, square(20, 0, 4)]);
        assert_eq!(found.len(), 2);
        let with_hole = found.iter().find(|island| island.len() == 2).unwrap();
        assert_eq!(area(with_hole), 64.0);
    }

    /// The 12 rings a ring-by-ring minimisation kept from the `union_many` leaf that made
    /// i_overlay 4.5 panic (U1 goat, `--rotations 30`, Nanometre grid): ten slivers under a
    /// nanometre wide and two thin triangles that hold all the area.
    const PANICKING_RINGS: [&[(i32, i32)]; 12] = [
        &[(-15626051, -9501468), (-15650796, -9486160), (-15621531, -9504265)],
        &[(-15411554, -9634164), (-15289963, -9686971), (-15493881, -9555247)],
        &[(-15621531, -9504265), (-15505079, -9576306), (-15411554, -9634164)],
        &[(-9724065, -7208837), (-9708249, -7193021), (-10312967, -5296385)],
        &[(-14756686, -6947407), (-14722609, -6780039), (-14712240, -6729111)],
        &[(-10119432, -5248963), (-10113325, -5277839), (-10089022, -5392740)],
        &[(-9701675, -7290857), (-9712525, -7238168), (-9704243, -7278387)],
        &[(-9707441, -7196844), (-9677732, -7337308), (-9671624, -7366184)],
        &[(-9686547, -7364323), (-9701675, -7290857), (-9692500, -7335415)],
        &[(10621481, 6035950), (10633930, 5955261), (10570636, 6137176)],
        &[(10164538, 7771200), (10153912, 7798021), (10182042, 7727014)],
        &[(-9704244, -7278386), (-9686547, -7364322), (-9692500, -7335414), (-9721457, -7194793)],
    ];

    fn panicking_rings() -> Rings {
        PANICKING_RINGS
            .iter()
            .map(|ring| ring.iter().map(|&(x, y)| IntPoint::new(x, y)).collect())
            .collect()
    }

    #[test]
    fn the_raw_overlay_still_panics_on_the_recorded_slivers() {
        // Guards the regression test below: if an i_overlay upgrade stops panicking here,
        // the retry path is no longer exercised (and may be dropped).
        let rings = panicking_rings();
        let raw = std::panic::catch_unwind(|| {
            run_overlay(&rings, &[], OverlayRule::Subject, FillRule::NonZero, UNION_SOLVER)
        });
        assert!(raw.is_err());
    }

    #[test]
    fn a_union_of_panicking_slivers_keeps_the_area_of_the_rest() {
        let rings = panicking_rings();
        let kept = without_slivers(&rings);
        assert_eq!(kept.len(), 3, "the two thin triangles and the 2.6e-3 mm² one stay");
        let expected = area(&resolve(&kept, FillRule::NonZero));
        assert!(expected > 0.0);
        assert_eq!(area(&union_many(&rings)), expected);
        assert_eq!(area(&resolve(&rings, FillRule::NonZero)), expected);
    }

    #[test]
    fn slivers_are_rings_narrower_than_one_unit() {
        let sliver = vec![IntPoint::new(0, 0), IntPoint::new(1_000_000, 0), IntPoint::new(0, 1)];
        let thin = vec![IntPoint::new(0, 0), IntPoint::new(1_000_000, 0), IntPoint::new(0, 4)];
        assert!(without_slivers(&[sliver]).is_empty());
        assert_eq!(without_slivers(&[thin]).len(), 1);
    }
}

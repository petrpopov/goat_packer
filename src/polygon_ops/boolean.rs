//! Booleans on the grid: every operand is read with the non-zero rule, as the slicers'
//! `union_`, `intersection`, `diff` do (spec 02 §1.1), and the result comes back resolved
//! (outer rings counter-clockwise, holes clockwise, grouped per island).

use i_overlay::core::{
    fill_rule::FillRule,
    overlay::{IntOverlayOptions, Overlay, ShapeType},
    overlay_rule::OverlayRule,
    solver::{Precision, Solver, Strategy},
};

use super::query::{IntBox, bbox};
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
    let edges: usize = subject.iter().chain(clip).map(Vec::len).sum();
    if edges == 0 {
        return Vec::new();
    }
    let mut overlay = Overlay::new_custom(edges, IntOverlayOptions::default(), SOLVER);
    for ring in subject.iter().filter(|ring| ring.len() >= 3) {
        overlay.add_contour(ring, ShapeType::Subject);
    }
    for ring in clip.iter().filter(|ring| ring.len() >= 3) {
        overlay.add_contour(ring, ShapeType::Clip);
    }
    overlay.overlay(rule, fill)
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
}

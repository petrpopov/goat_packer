//! The bound on generated shapes: exact on squares in a rectangle (the case where the
//! inequality is tight) and never below what the members place.

use super::*;
use crate::geometry::Rect;
use crate::keepout::{BodyRule, PlateRules};
use crate::kind_masks::{KindShapes, MaskRules, OriginGrid, plate_zones};
use crate::object_print::ObjectPrint;
use crate::options::PackOptions;
use crate::pack::{Member, pack_members};

const RESOLUTION: f64 = 1.0;

fn ring(points: &[(f64, f64)]) -> Vec<Point> {
    let mut ring: Vec<Point> = points.iter().map(|&(x, y)| Point { x, y }).collect();
    ring.push(ring[0]);
    ring
}

fn print_of(points: &[(f64, f64)], rotation_deg: f64) -> ObjectPrint {
    let shape = ring(points);
    ObjectPrint {
        rotation_deg,
        silhouette: vec![shape.clone()],
        part_hulls: vec![shape.clone()],
        part_boxes: vec![Rect::of_points(shape.iter())],
        landing: vec![shape.clone()],
        support_base: vec![shape.clone()],
        body: vec![vec![shape]],
        support: vec![vec![]],
        interface: vec![vec![]],
        ..ObjectPrint::default()
    }
}

/// A square of `side` mm centred on the origin.
fn square(side: f64) -> ObjectPrint {
    let h = side / 2.0;
    print_of(&[(-h, -h), (h, -h), (h, h), (-h, h)], 0.0)
}

/// A right triangle, turned a half turn when `flipped`.
fn triangle(flipped: bool) -> ObjectPrint {
    let sign = if flipped { -1.0 } else { 1.0 };
    let corners = [(-6.67, -3.33), (13.33, -3.33), (-6.67, 6.67)];
    let turned: Vec<(f64, f64)> = corners.iter().map(|&(x, y)| (sign * x, sign * y)).collect();
    print_of(&turned, if flipped { 180.0 } else { 0.0 })
}

pub(crate) fn plate(width: f64, height: f64) -> PlateRules {
    PlateRules {
        printable_area: Rect::new(0.0, 0.0, width, height),
        body_edge_band_mm: 0.0,
        print_edge_reserve_mm: 0.0,
        support_clip_mm: 0.0,
        support_clip_sides: [true; 4],
        keepouts: Vec::new(),
        body_rule: BodyRule::Silhouette,
    }
}

pub(crate) fn options(rules: &PlateRules) -> PackOptions {
    PackOptions {
        bed: rules.printable_area,
        body_bed: None,
        exclusions: Vec::new(),
        body_exclusions: Vec::new(),
        resolution: RESOLUTION,
        resolutions: Vec::new(),
        clearance: 0.0,
        angle_step_deg: 2.0,
        attempts: 16,
        candidates_per_step: 0,
        stall_attempts: 0,
        max_objects: 0,
        lock_orientation: true,
        seed: 0,
        support_overlap_mm: None,
        foreign_gaps: Default::default(),
        plate_rules: None,
    }
}

fn bounds_of(prints: &[ObjectPrint], rules: &PlateRules) -> UpperBounds {
    let zones = plate_zones(rules, None).expect("zones");
    let grid = OriginGrid {
        origin: Point { x: 0.0, y: 0.0 },
        resolution: RESOLUTION,
    };
    let mask_rules = MaskRules {
        clearance_mm: 0.0,
        foreign: Default::default(),
        body_rule: rules.body_rule,
        support_overlap_mm: None,
    };
    let masks = prints
        .iter()
        .map(|p| KindShapes::new(p, mask_rules).rasterize(RESOLUTION))
        .collect();
    let ctx = Ctx::new(masks, (0..prints.len()).collect(), &zones, grid);
    upper_bounds(&ctx)
}

fn packed(prints: &[ObjectPrint], rules: &PlateRules) -> usize {
    let members = [Member::Lattice, Member::Sweep, Member::Rows, Member::Improve];
    pack_members(prints, rules, None, &options(rules), &members)
        .expect("pack")
        .placements
        .len()
}

#[test]
fn squares_in_a_rectangle_meet_the_bound_exactly() {
    // 10 mm squares on a 100 x 60 plate: 10 x 6 fit, and (a/s + 1)(b/s + 1) is 60 too.
    let prints = [square(10.0)];
    let rules = plate(100.0, 60.0);
    assert_eq!(bounds_of(&prints, &rules).overall, Some(60));
    assert_eq!(packed(&prints, &rules), 60);
}

#[test]
fn squares_that_do_not_divide_the_plate_are_bounded_above_the_count() {
    // 10 mm squares on a 95 x 55 plate: 9 x 5 = 45 fit; the bound may be looser, never lower.
    let prints = [square(10.0)];
    let rules = plate(95.0, 55.0);
    let bound = bounds_of(&prints, &rules).overall.expect("bound");
    let count = packed(&prints, &rules);
    assert_eq!(count, 45);
    assert!(bound >= count, "bound {bound} below {count}");
}

#[test]
fn the_bound_never_falls_below_the_members_on_triangles() {
    let rules = plate(60.0, 40.0);
    for prints in [
        vec![triangle(false)],
        vec![triangle(false), triangle(true)],
    ] {
        let bounds = bounds_of(&prints, &rules);
        let count = packed(&prints, &rules);
        let overall = bounds.overall.expect("bound");
        assert!(overall >= count, "bound {overall} below {count}");
    }
}

#[test]
fn a_mix_of_classes_is_bounded_no_lower_than_each_class_alone() {
    let rules = plate(60.0, 40.0);
    let bounds = bounds_of(&[triangle(false), triangle(true)], &rules);
    let overall = bounds.overall.expect("overall");
    for single in bounds.per_class.iter().flatten() {
        assert!(overall >= *single, "{overall} below {single}");
    }
}

#[test]
fn the_bound_is_cheap() {
    let rules = plate(250.0, 250.0);
    let started = std::time::Instant::now();
    let bounds = bounds_of(&[triangle(false)], &rules);
    assert!(bounds.overall.is_some());
    // 0.25 s is far above the 100 ms goal; the test only catches a quadratic blow-up.
    assert!(started.elapsed().as_secs_f64() < 5.0);
}

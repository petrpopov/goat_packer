//! The portfolio's lattice member on generated prints (no real data): it nests a shaped
//! print, runs identically on any thread count, and a larger effort never loses a copy.

use goat_packer::keepout::{BodyRule, Keepout, KeepoutClass, KeepoutFrame, KeepoutShape};
use goat_packer::pack::{Member, PackOutcome, TowerSpec, pack_members};
use goat_packer::{ObjectPrint, PackOptions, PlateRules, Point, Rect};

fn ring(points: &[(f64, f64)]) -> Vec<Point> {
    let mut ring: Vec<Point> = points.iter().map(|&(x, y)| Point { x, y }).collect();
    ring.push(ring[0]);
    ring
}

/// A rhombus 20 x 10 mm with a slightly larger support base: it only nests in sheared rows.
fn rhombus() -> ObjectPrint {
    let shape = ring(&[(-10., 0.), (0., -5.), (10., 0.), (0., 5.)]);
    let base = ring(&[(-10.5, 0.), (0., -5.5), (10.5, 0.), (0., 5.5)]);
    ObjectPrint {
        silhouette: vec![shape.clone()],
        part_hulls: vec![shape.clone()],
        part_boxes: vec![Rect::of_points(shape.iter())],
        landing: vec![shape.clone()],
        support_base: vec![base],
        body: vec![vec![shape]],
        support: vec![vec![]],
        interface: vec![vec![]],
        ..ObjectPrint::default()
    }
}

/// The same shape turned half a turn, with its support base jutting out on one side: a second
/// rotation class that is not a copy of the first.
fn lopsided(half_turns: u8) -> ObjectPrint {
    let sign = if half_turns.is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    let turned = |points: &[(f64, f64)]| {
        let points: Vec<(f64, f64)> = points.iter().map(|&(x, y)| (sign * x, sign * y)).collect();
        ring(&points)
    };
    let shape = turned(&[(-10., 0.), (0., -5.), (10., 0.), (0., 5.)]);
    ObjectPrint {
        rotation_deg: 180.0 * f64::from(half_turns),
        silhouette: vec![shape.clone()],
        part_hulls: vec![shape.clone()],
        part_boxes: vec![Rect::of_points(shape.iter())],
        landing: vec![shape.clone()],
        support_base: vec![turned(&[(-10., -1.), (12., -1.), (12., 1.), (-10., 1.)])],
        body: vec![vec![shape]],
        support: vec![vec![]],
        interface: vec![vec![]],
        ..ObjectPrint::default()
    }
}

fn rules() -> PlateRules {
    PlateRules {
        printable_area: Rect::new(0.0, 0.0, 100.0, 100.0),
        body_edge_band_mm: 1.0,
        print_edge_reserve_mm: 0.0,
        keepouts: vec![Keepout {
            id: "purge".to_owned(),
            class: KeepoutClass::Soft,
            shape: KeepoutShape::Strips(vec![[20.0, 1.0, 80.0, 1.0, 1.0]]),
            z_range_mm: Some([0.0, 0.2]),
            frame: KeepoutFrame::Plate,
        }],
        body_rule: BodyRule::Silhouette,
    }
}

fn tower() -> TowerSpec {
    TowerSpec {
        body: Rect::new(-0.25, -0.25, 10.0, 8.0),
        brim_l0_mm: 2.0,
        chamfer: vec![[0.4, 1.5]],
        top_z_mm: 4.0,
        anchor_range: Rect::new(3.0, 3.0, 85.0, 88.0),
        source_anchor: Point { x: 60.0, y: 60.0 },
    }
}

fn options(attempts: usize) -> PackOptions {
    PackOptions {
        bed: Rect::new(0.0, 0.0, 100.0, 100.0),
        body_bed: None,
        exclusions: Vec::new(),
        body_exclusions: Vec::new(),
        resolution: 0.5,
        clearance: 0.5,
        angle_step_deg: 90.0,
        attempts,
        candidates_per_step: 0,
        stall_attempts: 0,
        max_objects: 0,
        lock_orientation: true,
        seed: 0,
        support_overlap_mm: None,
        plate_rules: None,
    }
}

fn run(
    prints: &[ObjectPrint],
    tower: Option<&TowerSpec>,
    attempts: usize,
    members: &[Member],
) -> PackOutcome {
    pack_members(prints, &rules(), tower, &options(attempts), members).expect("a valid layout")
}

#[test]
fn the_lattice_nests_shaped_prints_and_passes_both_checks() {
    // Rectangular cells would fit floor(100/21) x floor(100/11) = 36 of these; sheared rows give 56.
    let outcome = run(&[rhombus()], None, 64, &[Member::Lattice]);
    assert_eq!(outcome.member, Member::Lattice);
    assert!(
        outcome.placements.len() >= 54,
        "{}",
        outcome.placements.len()
    );
    // `pack_members` checks the winner twice; the report says what happened.
    assert!(outcome.validation.raster_passed >= 1);
    assert!(outcome.validation.vector_failures.is_empty());
    assert!(outcome.validation.raster_failures.is_empty());
    let rows = run(&[rhombus()], None, 64, &[Member::Rows]);
    assert!(outcome.placements.len() >= rows.placements.len());
}

#[test]
fn the_lattice_moves_the_tower_to_where_it_costs_least() {
    let spec = tower();
    let outcome = run(&[rhombus()], Some(&spec), 64, &[Member::Lattice]);
    let anchor = outcome.tower_anchor.expect("a tower anchor");
    let range = spec.anchor_range;
    assert!(anchor.x >= range.min_x && anchor.x <= range.max_x);
    assert!(anchor.y >= range.min_y && anchor.y <= range.max_y);
    let without = run(&[rhombus()], None, 64, &[Member::Lattice]);
    // The tower covers about 14 x 11 mm of a 100 mm plate: it may cost copies, but flush with
    // an edge it must not cost more than a few.
    assert!(outcome.placements.len() + 6 >= without.placements.len());
    assert!(outcome.validation.vector_failures.is_empty());
}

#[test]
fn the_lattice_run_is_identical_on_one_and_eight_threads() {
    let prints = [lopsided(0), lopsided(1)];
    let spec = tower();
    let result = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let outcome =
            pool.install(|| run(&prints, Some(&spec), 64, &[Member::Lattice, Member::Sweep]));
        serde_json::to_string(&outcome).unwrap()
    };
    let reference = result(1);
    for threads in [8, 1, 8] {
        assert_eq!(result(threads), reference, "{threads} threads");
    }
}

#[test]
fn more_effort_never_loses_copies() {
    let spec = tower();
    let prints = [lopsided(0), lopsided(1)];
    let mut previous = 0;
    for attempts in [1, 2, 4, 8, 16, 32, 64] {
        let outcome = run(&prints, Some(&spec), attempts, &[Member::Lattice]);
        let count = outcome.placements.len();
        assert!(
            count >= previous,
            "effort {attempts}: {count} after {previous}"
        );
        previous = count;
    }
}

//! The double-lattice and two-block members on generated prints (no real data): they pass
//! both checks, a shape's half-turn copy lets the double lattice beat the single class, and
//! runs are identical on any thread count.

use goat_packer::keepout::{BodyRule, PlateRules};
use goat_packer::pack::{Member, PackOutcome, TowerSpec, pack_members};
use goat_packer::{ObjectPrint, PackOptions, Point, Rect};

fn ring(points: &[(f64, f64)]) -> Vec<Point> {
    let mut ring: Vec<Point> = points.iter().map(|&(x, y)| Point { x, y }).collect();
    ring.push(ring[0]);
    ring
}

/// A right triangle (legs 20 and 10 mm) turned `half_turns` half turns.
fn triangle(half_turns: u8) -> ObjectPrint {
    let sign = if half_turns.is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    let turned = |scale: f64| {
        let corners = [(-6.67, -3.33), (13.33, -3.33), (-6.67, 6.67)];
        ring(&corners.map(|(x, y)| (sign * scale * x, sign * scale * y)))
    };
    ObjectPrint {
        rotation_deg: 180.0 * f64::from(half_turns),
        silhouette: vec![turned(1.0)],
        part_hulls: vec![turned(1.0)],
        part_boxes: vec![Rect::of_points(turned(1.0).iter())],
        landing: vec![turned(1.0)],
        support_base: vec![turned(1.05)],
        body: vec![vec![turned(1.0)]],
        support: vec![vec![]],
        interface: vec![vec![]],
        ..ObjectPrint::default()
    }
}

fn rules() -> PlateRules {
    PlateRules {
        printable_area: Rect::new(0.0, 0.0, 100.0, 80.0),
        body_edge_band_mm: 0.0,
        print_edge_reserve_mm: 0.0,
        keepouts: Vec::new(),
        body_rule: BodyRule::Silhouette,
    }
}

fn tower() -> TowerSpec {
    TowerSpec {
        body: Rect::new(-0.25, -0.25, 10.0, 8.0),
        brim_l0_mm: 2.0,
        chamfer: vec![[0.4, 1.5]],
        brim_conflicts: false,
        top_z_mm: 4.0,
        anchor_range: Rect::new(3.0, 3.0, 85.0, 68.0),
        source_anchor: Point { x: 60.0, y: 50.0 },
    }
}

fn options(attempts: usize) -> PackOptions {
    PackOptions {
        bed: Rect::new(0.0, 0.0, 100.0, 80.0),
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
        foreign_gaps: Default::default(),
        plate_rules: None,
    }
}

fn run(prints: &[ObjectPrint], attempts: usize, members: &[Member]) -> PackOutcome {
    pack_members(
        prints,
        &rules(),
        Some(&tower()),
        &options(attempts),
        members,
    )
    .expect("a valid layout")
}

#[test]
fn the_double_lattice_beats_the_single_class_and_passes_both_checks() {
    let both = [triangle(0), triangle(1)];
    let double = run(&both, 64, &[Member::DoubleLattice]);
    assert_eq!(double.member, Member::DoubleLattice);
    assert!(double.validation.raster_failures.is_empty());
    assert!(double.validation.vector_failures.is_empty());
    let single = run(&both[..1], 64, &[Member::Lattice]);
    let (d, s) = (double.placements.len(), single.placements.len());
    assert!(d * 10 >= s * 13, "double {d} against single {s}");
    assert!(double.placements.iter().any(|p| p.print == 1));
}

#[test]
fn the_two_block_layout_passes_both_checks() {
    let outcome = run(&[triangle(0), triangle(1)], 64, &[Member::TwoBlock]);
    assert_eq!(outcome.member, Member::TwoBlock);
    assert!(outcome.validation.raster_failures.is_empty());
    assert!(outcome.validation.vector_failures.is_empty());
}

#[test]
fn the_new_members_are_identical_on_one_and_eight_threads() {
    let prints = [triangle(0), triangle(1)];
    let members = [Member::DoubleLattice, Member::TwoBlock, Member::Lattice];
    let result = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let outcome = pool.install(|| run(&prints, 64, &members));
        serde_json::to_string(&outcome).unwrap()
    };
    let reference = result(1);
    for threads in [8, 1, 8] {
        assert_eq!(result(threads), reference, "{threads} threads");
    }
}

#[test]
fn more_effort_never_loses_copies() {
    let prints = [triangle(0), triangle(1)];
    for member in [Member::DoubleLattice, Member::TwoBlock] {
        let mut previous = 0;
        for attempts in [1, 4, 16, 64] {
            let count = run(&prints, attempts, &[member]).placements.len();
            assert!(
                count >= previous,
                "{member:?} at {attempts}: {count} after {previous}"
            );
            previous = count;
        }
    }
}

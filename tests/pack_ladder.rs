//! The resolution ladder on generated prints (no real data): a finer ladder never returns
//! fewer copies than its coarse level, the result is identical on any thread count, `refine`
//! continues a coarse outcome to the same layout as a full run, and the upper bound is never
//! below what was placed.

use goat_packer::keepout::BodyRule;
use goat_packer::pack::{DEFAULT_MEMBERS, Member, PackOutcome, pack_members, refine_members};
use goat_packer::{ObjectPrint, PackOptions, PlateRules, Point, Rect};

fn ring(points: &[(f64, f64)]) -> Vec<Point> {
    let mut ring: Vec<Point> = points.iter().map(|&(x, y)| Point { x, y }).collect();
    ring.push(ring[0]);
    ring
}

/// A 20 x 10 mm rhombus with a slightly larger support base: it nests in sheared rows, and
/// how many fit depends on the pixel size.
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

fn rules() -> PlateRules {
    PlateRules {
        printable_area: Rect::new(0.0, 0.0, 70.0, 50.0),
        body_edge_band_mm: 1.0,
        print_edge_reserve_mm: 0.0,
        support_clip_mm: 0.0,
        support_clip_sides: [true; 4],
        keepouts: Vec::new(),
        body_rule: BodyRule::Silhouette,
    }
}

fn options(resolutions: &[f64]) -> PackOptions {
    PackOptions {
        bed: Rect::new(0.0, 0.0, 70.0, 50.0),
        body_bed: None,
        exclusions: Vec::new(),
        body_exclusions: Vec::new(),
        resolution: resolutions[0],
        resolutions: resolutions.to_vec(),
        clearance: 0.5,
        angle_step_deg: 90.0,
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

fn run(resolutions: &[f64]) -> PackOutcome {
    pack_members(&[rhombus()], &rules(), None, &options(resolutions), &DEFAULT_MEMBERS)
        .expect("a valid layout")
}

#[test]
fn a_finer_ladder_never_returns_fewer_copies() {
    let coarse = run(&[1.0]);
    let ladder = run(&[1.0, 0.5, 0.25]);
    assert!(
        ladder.placements.len() >= coarse.placements.len(),
        "{} < {}",
        ladder.placements.len(),
        coarse.placements.len()
    );
    // Whatever level won, the published layout passed both checks.
    assert!(ladder.validation.vector_failures.is_empty());
    assert!(ladder.validation.raster_failures.is_empty());
    assert!(ladder.levels.len() >= 2, "{:?}", ladder.levels);
    let counts: Vec<usize> = ladder.levels.iter().map(|l| l.count).collect();
    assert_eq!(counts.iter().max().copied(), Some(ladder.placements.len()));
}

#[test]
fn the_ladder_is_identical_on_any_thread_count() {
    let on = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("pool");
        let outcome = pool.install(|| run(&[1.0, 0.5, 0.25]));
        let counts: Vec<(f64, usize)> = outcome
            .levels
            .iter()
            .map(|l| (l.resolution_mm, l.count))
            .collect();
        (serde_json::to_string(&outcome).expect("json"), counts)
    };
    assert_eq!(on(1), on(4));
}

#[test]
fn refining_a_coarse_outcome_equals_the_full_ladder() {
    let ladder = [1.0, 0.5, 0.25];
    let full = run(&ladder);
    let coarse = run(&[1.0]);
    let members = DEFAULT_MEMBERS;
    let refined = refine_members(
        &[rhombus()],
        &rules(),
        None,
        &options(&ladder),
        &members,
        coarse,
    )
    .expect("refined");
    assert_eq!(refined.placements.len(), full.placements.len());
    assert_eq!(refined.resolution_mm, full.resolution_mm);
    let at = |o: &PackOutcome| -> Vec<(usize, i64, i64)> {
        o.placements
            .iter()
            .map(|p| (p.print, (p.x_mm * 1e6) as i64, (p.y_mm * 1e6) as i64))
            .collect()
    };
    assert_eq!(at(&refined), at(&full));
}

#[test]
fn the_bound_is_reported_and_not_below_the_count() {
    let outcome = run(&[1.0, 0.5]);
    let bound = outcome.upper_bound.expect("a bound");
    assert!(bound >= outcome.placements.len(), "{bound}");
    assert_eq!(outcome.class_bounds.len(), 1);
    assert!(outcome.class_bounds[0].expect("class bound") >= outcome.placements.len());
    for level in &outcome.levels {
        assert!(level.upper_bound.is_some_and(|b| b >= level.count), "{level:?}");
    }
}

#[test]
fn greedy_only_runs_stay_on_the_first_level() {
    let outcome = pack_members(
        &[rhombus()],
        &rules(),
        None,
        &options(&[1.0, 0.5]),
        &[Member::Sweep],
    )
    .expect("a valid layout");
    assert_eq!(outcome.levels.len(), 1);
    assert_eq!(outcome.resolution_mm, 1.0);
}

#[test]
fn a_bad_resolution_in_the_ladder_is_refused() {
    let result = pack_members(
        &[rhombus()],
        &rules(),
        None,
        &options(&[1.0, 0.0]),
        &DEFAULT_MEMBERS,
    );
    assert!(result.is_err());
}

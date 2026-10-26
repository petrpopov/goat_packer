use goat_packer::{
    Footprint, PackOptions, PackResult, PackedPlacement, Point, Rect, grid_layout, masks_overlap,
    pack_footprint, rasterize_footprint, validate_result,
};

fn polygon(points: &[(f64, f64)]) -> Footprint {
    let mut loop_: Vec<Point> = points.iter().map(|&(x, y)| Point { x, y }).collect();
    loop_.push(loop_[0]);
    let min_x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let max_x = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let min_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_y = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    Footprint {
        loops: vec![loop_],
        body_loops: Vec::new(),
        bed_loops: None,
        width: max_x - min_x,
        height: max_y - min_y,
        min_z: 0.0,
        max_z: 1.0,
        triangle_count: points.len() - 2,
    }
}

#[test]
fn concave_shapes_can_interlock_while_bounding_boxes_overlap() {
    let l = polygon(&[
        (-2., -2.),
        (2., -2.),
        (2., -1.),
        (-1., -1.),
        (-1., 2.),
        (-2., 2.),
    ]);
    let first = rasterize_footprint(&l, 0.0, 0.25, 0.0);
    let second = rasterize_footprint(&l, 180.0, 0.25, 0.0);
    assert!(!masks_overlap(
        &first.physical,
        0,
        0,
        &second.physical,
        12,
        12
    ));
    assert!(first.physical.width > 0 && second.physical.width > 0);
}

#[test]
fn packer_places_four_squares_without_overlap() {
    let square = polygon(&[(-2., -2.), (2., -2.), (2., 2.), (-2., 2.)]);
    let options = PackOptions {
        bed: Rect::new(0.0, 0.0, 10.0, 10.0),
        exclusions: vec![],
        resolution: 0.5,
        clearance: 0.5,
        angle_step_deg: 90.0,
        attempts: 24,
        candidates_per_step: 250,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 7,
        support_overlap_mm: None,
    };
    let result = pack_footprint(&square, &options);
    assert!(result.placements.len() >= 4, "{result:#?}");
    validate_result(&square, &options, &result).unwrap();
}

#[test]
fn packer_never_uses_exclusion_area() {
    let square = polygon(&[(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)]);
    let options = PackOptions {
        bed: Rect::new(0.0, 0.0, 8.0, 8.0),
        exclusions: vec![Rect::new(0.0, 0.0, 4.0, 4.0)],
        resolution: 0.5,
        clearance: 0.5,
        angle_step_deg: 90.0,
        attempts: 8,
        candidates_per_step: 100,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 9,
        support_overlap_mm: None,
    };
    let result = pack_footprint(&square, &options);
    validate_result(&square, &options, &result).unwrap();
}

/// Bed bounds must follow `bed_loops` — the printed patch ∪ silhouette — not the mesh
/// silhouette alone. On a real 17-object P1S goat plate the silhouette cleared the bed edge
/// by 0.06 mm, but thin leg tips print ~2 mm past the mesh (Arachne miter), so the real
/// print left the bed and BambuStudio killed the final slice ("too close to exclusion area"
/// / "no object fully inside it"). A layout whose silhouette fits but whose printed extent
/// does not must be rejected.
#[test]
fn bed_bounds_measure_bed_loops_not_the_silhouette_alone() {
    let mut fp = polygon(&[(-2., -2.), (2., -2.), (2., 2.), (-2., 2.)]);
    // A 3 mm printed spike past the mesh's right edge, like the measured tip overshoot.
    let mut spike: Vec<Point> = [(-2., -0.5), (5., -0.25), (5., 0.25), (-2., 0.5)]
        .iter()
        .map(|&(x, y)| Point { x, y })
        .collect();
    spike.push(spike[0]);
    fp.bed_loops = Some(vec![spike]);
    let options = PackOptions {
        bed: Rect::new(0.0, 0.0, 10.0, 10.0),
        exclusions: vec![],
        resolution: 0.5,
        clearance: 0.5,
        angle_step_deg: 90.0,
        attempts: 24,
        candidates_per_step: 250,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 7,
        support_overlap_mm: None,
    };
    // The mesh square fits at x=8 (max 8+2=10), but the spike would print to 8+5=13: a
    // silhouette-only check accepts it, the bed_loops check must not.
    let silhouette_only_fit = PackResult {
        placements: vec![PackedPlacement {
            x_mm: 8.0,
            y_mm: 5.0,
            angle_deg: 0.0,
        }],
        resolution_mm: options.resolution,
        requested_clearance_mm: options.clearance,
        angle_step_deg: options.angle_step_deg,
        attempts: options.attempts,
        attempts_run: options.attempts,
        seed: options.seed,
    };
    assert!(
        validate_result(&fp, &options, &silhouette_only_fit).is_err(),
        "placement printing past the bed edge via bed_loops must be rejected"
    );
    // At x=4.5 the spike ends at 9.5 — inside the bed; the layout must validate.
    let spike_fits = PackResult {
        placements: vec![PackedPlacement {
            x_mm: 4.5,
            y_mm: 5.0,
            angle_deg: 0.0,
        }],
        resolution_mm: options.resolution,
        requested_clearance_mm: options.clearance,
        angle_step_deg: options.angle_step_deg,
        attempts: options.attempts,
        attempts_run: options.attempts,
        seed: options.seed,
    };
    validate_result(&fp, &options, &spike_fits).unwrap();
}

#[test]
fn grid_tiles_the_bed_and_validate_result_accepts_it() {
    // 3x6 with 1 mm clearance has 4x7 mm pitches. Clearance may extend off the bed:
    // unlike the old three-cell baseline, all six physically fitting cells must survive.
    let square = polygon(&[(-1.5, -3.), (1.5, -3.), (1.5, 3.), (-1.5, 3.)]);
    let options = PackOptions {
        bed: Rect::new(0.0, 0.0, 13.0, 13.0),
        exclusions: vec![],
        resolution: 0.5,
        clearance: 1.0,
        angle_step_deg: 90.0,
        attempts: 1,
        candidates_per_step: 10,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 1,
        support_overlap_mm: None,
    };
    let result = grid_layout(&square, &options);
    assert_eq!(result.placements.len(), 6, "{result:#?}");
    // The canonical long-axis frame has two columns and three equally spaced rows.
    let ys: std::collections::BTreeSet<i32> = result
        .placements
        .iter()
        .map(|p| (p.y_mm * 4.0) as i32)
        .collect();
    assert_eq!(
        ys.len(),
        3,
        "three rows in the canonical long-axis frame: {result:#?}"
    );
    let first = &result.placements[0];
    assert!(first.x_mm < 4.0 && first.y_mm < 4.0, "{first:#?}");
    validate_result(&square, &options, &result).unwrap();
}

#[test]
fn grid_skips_cells_that_touch_an_exclusion() {
    // 4x4 squares, 0 clearance, 0.5 mm pixels: 4 mm cells on a 16x8 bed give a 4x2 strict
    // lattice; the exclusion covers the right half of the bed, so the two right columns are
    // dropped, not nudged sideways.
    let square = polygon(&[(-2., -2.), (2., -2.), (2., 2.), (-2., 2.)]);
    let options = PackOptions {
        bed: Rect::new(0.0, 0.0, 16.0, 8.0),
        exclusions: vec![Rect::new(8.0, 0.0, 16.0, 8.0)],
        resolution: 0.5,
        clearance: 0.0,
        angle_step_deg: 90.0,
        attempts: 1,
        candidates_per_step: 10,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 1,
        support_overlap_mm: None,
    };
    let result = grid_layout(&square, &options);
    // Two 4 mm columns survive (centres at x = 2 and 6 mm) x two rows = 4 cells, all left
    // of the exclusion; the right half's cells are dropped without shifting anyone.
    assert_eq!(result.placements.len(), 4);
    assert!(
        result.placements.iter().all(|p| p.x_mm < 8.0),
        "{result:#?}"
    );
    validate_result(&square, &options, &result).unwrap();
}

/// The old strategies hard-capped layouts at 128 objects; with `max_objects: 0` (unlimited)
/// a bed full of tiny parts must pack past that, and an explicit cap must stop exactly there.
#[test]
fn layouts_exceed_the_old_128_cap_and_honor_an_explicit_one() {
    // 2x2 squares, 0 clearance, 1 mm pixels on a 40x40 bed: a 20x20 grid = 400 cells.
    let square = polygon(&[(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)]);
    let mut options = PackOptions {
        bed: Rect::new(0.0, 0.0, 40.0, 40.0),
        exclusions: vec![],
        resolution: 1.0,
        clearance: 0.0,
        angle_step_deg: 90.0,
        attempts: 1,
        candidates_per_step: 10,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 1,
        support_overlap_mm: None,
    };
    let unlimited = grid_layout(&square, &options);
    assert_eq!(unlimited.placements.len(), 400, "{unlimited:#?}");

    options.max_objects = 150;
    let capped = grid_layout(&square, &options);
    assert_eq!(capped.placements.len(), 150);
    validate_result(&square, &options, &capped).unwrap();
}

/// Two-zone rule: supports may overlap other supports, but a body keeps clearance from
/// anything foreign. A wide support slab lets neighbours stand closer than their full patches
/// would allow, while overlapping bodies stay rejected.
#[test]
fn two_zone_rule_allows_support_overlap_but_rejects_body_contact() {
    // A 3 mm tall support slab (the full patch) carrying a 1 mm tall body in its middle.
    let body: Vec<(f64, f64)> = vec![(-6., -0.5), (6., -0.5), (6., 0.5), (-6., 0.5)];
    let slab: Vec<(f64, f64)> = vec![(-6., -1.5), (6., -1.5), (6., 1.5), (-6., 1.5)];
    let close = |dy: f64| PackedPlacement {
        x_mm: 10.0,
        y_mm: 10.0 + dy,
        angle_deg: 0.0,
    };
    let mut loop_body: Vec<Point> = body.iter().map(|&(x, y)| Point { x, y }).collect();
    loop_body.push(loop_body[0]);
    let mut loop_slab: Vec<Point> = slab.iter().map(|&(x, y)| Point { x, y }).collect();
    loop_slab.push(loop_slab[0]);
    let footprint = Footprint {
        bed_loops: None,
        loops: vec![loop_slab.clone()],
        body_loops: vec![loop_body],
        width: 12.0,
        height: 3.0,
        min_z: 0.0,
        max_z: 1.0,
        triangle_count: 4,
    };
    let options = PackOptions {
        bed: Rect::new(0.0, 0.0, 40.0, 40.0),
        exclusions: vec![],
        resolution: 0.5,
        clearance: 0.0,
        angle_step_deg: 90.0,
        attempts: 1,
        candidates_per_step: 10,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 1,
        support_overlap_mm: None,
    };

    // dy = 2.0 mm: the slabs overlap (each 3 mm tall), the bodies (1 mm tall) do not — the
    // full-patch rule would reject this, the two-zone rule must accept it.
    let slabs_overlap = PackResult {
        placements: vec![close(0.0), close(2.0)],
        resolution_mm: 0.5,
        requested_clearance_mm: 0.0,
        angle_step_deg: 90.0,
        attempts: 1,
        attempts_run: 1,
        seed: 1,
    };
    validate_result(&footprint, &options, &slabs_overlap)
        .expect("support-on-support overlap is allowed");

    // Bodies on top of each other must still be rejected.
    let bodies_overlap = PackResult {
        placements: vec![close(0.0), close(0.25)],
        ..slabs_overlap
    };
    assert!(validate_result(&footprint, &options, &bodies_overlap).is_err());
}

#[test]
fn grid_picks_the_orientation_that_fits_more_cells() {
    // 8x2 rectangle on a 10x20 bed with 0.2 mm clearance at 0.5 mm pixels: the upright
    // lattice fits one column of 7 rows; rotated fits 3 columns x 2 rows = 6 — upright wins.
    let rectangle = polygon(&[(-4., -1.), (4., -1.), (4., 1.), (-4., 1.)]);
    let options = PackOptions {
        bed: Rect::new(0.0, 0.0, 10.0, 20.0),
        exclusions: vec![],
        resolution: 0.5,
        clearance: 0.2,
        angle_step_deg: 90.0,
        attempts: 1,
        candidates_per_step: 10,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 1,
        support_overlap_mm: None,
    };
    let result = grid_layout(&rectangle, &options);
    // The undilated edge fits: seven rows at 3 mm pitch, instead of the old six.
    assert_eq!(result.placements.len(), 7, "{result:#?}");
    assert!(
        result
            .placements
            .iter()
            .all(|p| (p.angle_deg % 180.0).abs() < 1e-7)
    );
    validate_result(&rectangle, &options, &result).unwrap();
}

mod support;

fn stable_options() -> PackOptions {
    PackOptions {
        bed: Rect::new(-3.25, 10.75, 30.75, 38.75),
        exclusions: vec![Rect::new(10., 10.75, 14., 14.75)],
        resolution: 0.25,
        clearance: 0.5,
        angle_step_deg: 15.,
        attempts: 96,
        candidates_per_step: 128,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 1,
        support_overlap_mm: None,
    }
}

#[test]
fn rotations_translations_winding_seed_and_thread_count_do_not_change_count() {
    let fp = polygon(&[
        (-5., -3.),
        (5., -3.),
        (5., -1.),
        (-1., -1.),
        (-1., 4.),
        (-5., 4.),
    ]);
    let options = stable_options();
    let baseline = pack_footprint(&fp, &options);
    assert!(!baseline.placements.is_empty());
    let reference = serde_json::to_value(&baseline.placements).unwrap();
    for threads in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        for angle in [0., 13., 45., 90., 137., 179., 271.] {
            let moved = support::transform(&fp, angle, 123.456, -54.321, true);
            let mut opts = options.clone();
            opts.seed = threads as u64 * 17;
            opts.stall_attempts = 1;
            let result = pool.install(|| pack_footprint(&moved, &opts));
            assert_eq!(
                baseline.placements.len(),
                result.placements.len(),
                "rotation {angle}, threads {threads}"
            );
            validate_result(&moved, &opts, &result).unwrap();
            let same = pool.install(|| pack_footprint(&fp, &opts));
            assert_eq!(reference, serde_json::to_value(same.placements).unwrap());
        }
    }
}

#[test]
fn mixed_orientations_can_fill_a_windmill() {
    let fp = polygon(&[(-3., -2.), (3., -2.), (3., 2.), (-3., 2.)]);
    let mut options = stable_options();
    options.bed = Rect::new(0., 0., 10., 10.);
    options.exclusions.clear();
    options.clearance = 0.;
    options.angle_step_deg = 90.;
    let result = pack_footprint(&fp, &options);
    assert_eq!(result.placements.len(), 4, "{result:?}");
    validate_result(&fp, &options, &result).unwrap();
}

#[test]
fn search_has_no_128_cap_and_respects_explicit_caps() {
    let fp = polygon(&[(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)]);
    let mut options = stable_options();
    options.bed = Rect::new(0., 0., 40., 40.);
    options.exclusions.clear();
    options.clearance = 0.;
    options.resolution = 1.;
    options.angle_step_deg = 90.;
    for limit in [0, 1, 17, 150] {
        options.max_objects = limit;
        let result = pack_footprint(&fp, &options);
        assert_eq!(
            result.placements.len(),
            if limit == 0 { 400 } else { limit }
        );
        validate_result(&fp, &options, &result).unwrap();
    }
}

#[test]
fn locked_orientation_is_preserved_and_oversized_parts_return_empty() {
    let fp = polygon(&[(-4., -1.), (4., -1.), (4., 1.), (-4., 1.)]);
    let mut opts = stable_options();
    opts.lock_orientation = true;
    opts.bed = Rect::new(0., 0., 10., 10.);
    opts.exclusions.clear();
    let result = pack_footprint(&fp, &opts);
    assert!(result.placements.iter().all(|p| p.angle_deg == 0.));
    validate_result(&fp, &opts, &result).unwrap();
    opts.bed = Rect::new(0., 0., 1., 1.);
    assert!(pack_footprint(&fp, &opts).placements.is_empty());
}

#[test]
fn subpixel_parts_cannot_become_invisible_or_loop_forever() {
    let fp = polygon(&[(-0.01, -0.01), (0.01, -0.01), (0.01, 0.01), (-0.01, 0.01)]);
    let mut opts = stable_options();
    opts.bed = Rect::new(0., 0., 2., 2.);
    opts.exclusions.clear();
    opts.resolution = 1.;
    opts.clearance = 0.;
    opts.angle_step_deg = 90.;
    let result = pack_footprint(&fp, &opts);
    assert_eq!(result.placements.len(), 1);
    validate_result(&fp, &opts, &result).unwrap();
}

#[test]
fn bed_extent_includes_patch_and_silhouette_even_when_patch_is_smaller() {
    let mut fp = polygon(&[(-2., -2.), (2., -2.), (2., 2.), (-2., 2.)]);
    fp.bed_loops = Some(polygon(&[(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)]).loops);
    let mask = rasterize_footprint(&fp, 0., 0.5, 0.);
    assert_eq!(mask.bed_physical_bounds.min_x, -2.);
    assert_eq!(mask.bed_physical_bounds.max_y, 2.);
}

#[test]
fn validator_rejects_nonfinite_poses_and_forbidden_rotations() {
    let fp = polygon(&[(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)]);
    let mut opts = stable_options();
    opts.lock_orientation = true;
    let mut result = pack_footprint(&fp, &opts);
    result.placements[0].angle_deg = 90.;
    assert!(validate_result(&fp, &opts, &result).is_err());
    result.placements[0].angle_deg = 0.;
    result.placements[0].x_mm = f64::NAN;
    assert!(validate_result(&fp, &opts, &result).is_err());
}

#[test]
fn increasing_effort_never_discards_a_better_layout() {
    let fp = polygon(&[(-3., -2.), (3., -2.), (3., 2.), (-3., 2.)]);
    let mut opts = stable_options();
    opts.bed = Rect::new(0., 0., 10., 10.);
    opts.exclusions.clear();
    opts.clearance = 0.;
    opts.angle_step_deg = 30.;
    let mut best = 0;
    for attempts in [1, 8, 24, 64, 96, 200, 1000] {
        opts.attempts = attempts;
        let result = pack_footprint(&fp, &opts);
        assert!(
            result.placements.len() >= best,
            "effort {attempts} regressed from {best}"
        );
        best = result.placements.len();
        validate_result(&fp, &opts, &result).unwrap();
    }
}

#[test]
fn validator_keeps_fractional_pose_phase_instead_of_rounding_away_overlap() {
    let fp = polygon(&[(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)]);
    let mut opts = stable_options();
    opts.bed = Rect::new(0., 0., 4., 4.);
    opts.exclusions.clear();
    opts.lock_orientation = true;
    opts.resolution = 1.;
    opts.clearance = 0.;
    let mut result = pack_footprint(&fp, &opts);
    result.placements = vec![
        PackedPlacement {
            x_mm: 1.2,
            y_mm: 1.,
            angle_deg: 0.,
        },
        PackedPlacement {
            x_mm: 1.8,
            y_mm: 1.,
            angle_deg: 0.,
        },
    ];
    assert!(validate_result(&fp, &opts, &result).is_err());
}

#[test]
fn grid_checks_distant_islands_not_just_immediate_neighbours() {
    let mut fp = polygon(&[(-3., -1.), (-1., -1.), (-1., 1.), (-3., 1.)]);
    fp.loops
        .extend(polygon(&[(1., -1.), (3., -1.), (3., 1.), (1., 1.)]).loops);
    fp.width = 6.;
    let mut opts = stable_options();
    opts.lock_orientation = true;
    opts.bed = Rect::new(0., 0., 20., 8.);
    opts.exclusions.clear();
    opts.clearance = 0.;
    opts.resolution = 0.5;
    // At x shift 2 the islands interlock; at twice that shift (4) they collide.
    let result = grid_layout(&fp, &opts);
    validate_result(&fp, &opts, &result).unwrap();
}

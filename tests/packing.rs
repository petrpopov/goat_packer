use goat_packer::{
    Footprint, PackOptions, PackResult, PackedPlacement, Point, Rect, grid_layout,
    masks_overlap, pack_footprint, rasterize_footprint, validate_result,
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
    };
    let result = pack_footprint(&square, &options);
    validate_result(&square, &options, &result).unwrap();
}

#[test]
fn grid_tiles_the_bed_and_validate_result_accepts_it() {
    // 3x6 footprint on a 13x13 bed with 1 mm clearance at 0.5 mm pixels: the dilated mask
    // spans 8x14 px — a strict 3-column by 1-row lattice (a second row's 28 px would leave
    // the 26 px bed), with every placement on the same fixed pitch.
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
    };
    let result = grid_layout(&square, &options);
    assert_eq!(result.placements.len(), 3, "{result:#?}");
    // Strict pitch: three columns share one y (the winning orientation is the rotated one).
    let ys: std::collections::BTreeSet<i32> =
        result.placements.iter().map(|p| (p.y_mm * 4.0) as i32).collect();
    assert_eq!(ys.len(), 1, "one row: {result:#?}");
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
    };
    let result = grid_layout(&square, &options);
    // Two 4 mm columns survive (centres at x = 2 and 6 mm) x two rows = 4 cells, all left
    // of the exclusion; the right half's cells are dropped without shifting anyone.
    assert_eq!(result.placements.len(), 4);
    assert!(result.placements.iter().all(|p| p.x_mm < 8.0), "{result:#?}");
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
    let close = |dy: f64| PackedPlacement { x_mm: 10.0, y_mm: 10.0 + dy, angle_deg: 0.0 };
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
    let bodies_overlap = PackResult { placements: vec![close(0.0), close(0.25)], ..slabs_overlap };
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
    };
    let result = grid_layout(&rectangle, &options);
    // Upright: one 4.5 mm-centred column, 6 rows of 3 mm (40 px / 6 px pitch).
    assert_eq!(result.placements.len(), 6, "{result:#?}");
    assert!(result.placements.iter().all(|p| p.angle_deg == 0.0));
    validate_result(&rectangle, &options, &result).unwrap();
}

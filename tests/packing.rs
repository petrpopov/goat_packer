use goat_packer::{
    Footprint, PackOptions, Point, Rect, masks_overlap, pack_footprint, rasterize_footprint,
    validate_result,
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
        seed: 9,
    };
    let result = pack_footprint(&square, &options);
    validate_result(&square, &options, &result).unwrap();
}

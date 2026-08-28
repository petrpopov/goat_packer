use goat_packer::{
    Footprint, Point, Rect, masks_overlap, placement_is_allowed, rasterize_footprint,
};

fn rectangle(width: f64, height: f64) -> Footprint {
    let x = width / 2.0;
    let y = height / 2.0;
    Footprint {
        loops: vec![vec![
            Point { x: -x, y: -y },
            Point { x, y: -y },
            Point { x, y },
            Point { x: -x, y },
            Point { x: -x, y: -y },
        ]],
        width,
        height,
        min_z: 0.0,
        max_z: 1.0,
        triangle_count: 2,
    }
}

#[test]
fn rasterizes_rotation_and_clearance_separately() {
    let fp = rectangle(4.0, 2.0);
    let horizontal = rasterize_footprint(&fp, 0.0, 0.5, 1.0);
    let vertical = rasterize_footprint(&fp, 90.0, 0.5, 1.0);

    assert!(horizontal.physical.width > horizontal.physical.height);
    assert!(vertical.physical.height > vertical.physical.width);
    assert!(horizontal.clearance.pixel_count > horizontal.physical.pixel_count);
}

#[test]
fn clearance_masks_block_close_shapes_even_when_physical_masks_do_not_touch() {
    let fp = rectangle(2.0, 2.0);
    let mask = rasterize_footprint(&fp, 0.0, 0.25, 1.0);

    assert!(masks_overlap(&mask.clearance, 0, 0, &mask.clearance, 10, 0));
    assert!(!masks_overlap(
        &mask.clearance,
        0,
        0,
        &mask.clearance,
        14,
        0
    ));
}

#[test]
fn placement_obeys_bed_and_exclusion_rectangles() {
    let fp = rectangle(4.0, 4.0);
    let mask = rasterize_footprint(&fp, 0.0, 0.5, 0.5);
    let bed = Rect::new(0.0, 0.0, 20.0, 20.0);
    let exclusion = Rect::new(0.0, 0.0, 5.0, 5.0);

    assert!(placement_is_allowed(&mask, 20, 20, bed, &[exclusion]));
    assert!(!placement_is_allowed(&mask, 4, 4, bed, &[exclusion]));
    assert!(!placement_is_allowed(&mask, 1, 20, bed, &[exclusion]));
}

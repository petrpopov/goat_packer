use goat_packer::{
    Footprint, Point, Rect, masks_overlap, placement_is_allowed, rasterize_footprint,
};

fn rectangle(width: f64, height: f64) -> Footprint {
    let x = width / 2.0;
    let y = height / 2.0;
    Footprint {
        support_landing_loops: Vec::new(),
        layer_zones: Vec::new(),
        body_loops: Vec::new(),
        bed_loops: None,
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

#[test]
fn thin_islands_and_tips_do_not_disappear_between_pixel_centres() {
    let mut fp = rectangle(2.0, 2.0);
    // A disconnected upper tooth thinner than one 0.5 mm cell, with no sampled centre.
    fp.loops.push(vec![
        Point { x: 3.05, y: 0.05 },
        Point { x: 5.05, y: 0.05 },
        Point { x: 5.05, y: 0.10 },
        Point { x: 3.05, y: 0.10 },
        Point { x: 3.05, y: 0.05 },
    ]);
    let mask = rasterize_footprint(&fp, 0.0, 0.5, 0.0);
    assert!(mask.physical.contains_local(8, 0));
}

#[test]
fn body_outside_first_layer_patch_is_not_clipped() {
    let mut fp = rectangle(2.0, 2.0);
    fp.body_loops = rectangle(10.0, 2.0).loops;
    let mask = rasterize_footprint(&fp, 0.0, 0.5, 0.0);
    assert!(mask.body_clearance.contains_local(8, 0));
    assert_eq!(mask.bed_physical_bounds.max_x, 5.0);
    assert!(!placement_is_allowed(
        &mask,
        4,
        10,
        Rect::new(0.0, 0.0, 20.0, 20.0),
        &[]
    ));
    assert!(goat_packer::placements_collide(&mask, &mask, 12, 0));
}

#[test]
fn empty_and_one_sided_height_bands_keep_their_indices_at_every_rotation() {
    use goat_packer::LayerZone;
    let mut fp = rectangle(2.0, 2.0);
    fp.layer_zones = vec![
        LayerZone::default(),
        LayerZone {
            support_loops: rectangle(8.0, 0.04).loops,
            body_loops: vec![],
        },
        LayerZone {
            support_loops: vec![],
            body_loops: rectangle(2.0, 2.0).loops,
        },
    ];
    for angle in [0.0, 37.0, 90.0] {
        let mask = rasterize_footprint(&fp, angle, 0.5, 0.0);
        assert_eq!(mask.layer_zones.len(), 3);
        assert_eq!(mask.layer_zones[0].0.pixel_count, 0);
        assert!(mask.layer_zones[1].0.pixel_count > 0);
        assert_eq!(mask.layer_zones[1].1.pixel_count, 0);
        assert_eq!(mask.layer_zones[2].0.pixel_count, 0);
        assert!(mask.layer_zones[2].1.pixel_count > 0);
    }
    let mask = rasterize_footprint(&fp, 0.0, 0.5, 0.0);
    assert_eq!(mask.bed_physical_bounds.max_x, 4.0);
    assert!(!placement_is_allowed(
        &mask,
        4,
        10,
        Rect::new(0.0, 0.0, 20.0, 20.0),
        &[]
    ));
}

#[test]
fn diagonal_clearance_is_not_rounded_down_by_a_discrete_disc() {
    let fp = rectangle(0.5, 0.5);
    let mask = rasterize_footprint(&fp, 0.0, 0.5, 1.0);
    // Physical squares separated diagonally by sqrt(0.5²+0.5²) < 1 mm.
    assert!(goat_packer::placements_collide(&mask, &mask, 2, 2));
}

#[test]
fn merging_samples_keeps_probe_supports_and_absolute_height_bands() {
    use goat_packer::LayerZone;
    let mut fp = rectangle(2.0, 2.0);
    fp.layer_zones = vec![
        LayerZone::default(),
        LayerZone {
            support_loops: rectangle(8.0, 0.5).loops,
            body_loops: vec![],
        },
    ];
    fp.merge_layer_zones(&[
        LayerZone::default(),
        LayerZone {
            support_loops: rectangle(0.5, 8.0).loops,
            body_loops: vec![],
        },
        LayerZone {
            support_loops: vec![],
            body_loops: rectangle(2.0, 2.0).loops,
        },
    ]);
    assert_eq!(fp.layer_zones.len(), 3);
    assert!(fp.layer_zones[0].support_loops.is_empty());
    let mask = rasterize_footprint(&fp, 0.0, 0.25, 0.0);
    assert!(mask.layer_zones[1].0.contains_local(12, 0));
    assert!(mask.layer_zones[1].0.contains_local(0, 12));
    assert_eq!(mask.layer_zones[1].1.pixel_count, 0);
    assert_eq!(mask.bed_physical_bounds, Rect::new(-4.0, -4.0, 4.0, 4.0));
    assert!(goat_packer::point_in_footprint(
        &fp.printed_extent_loops(),
        3.0,
        0.0
    ));
    assert!(goat_packer::point_in_footprint(
        &fp.printed_extent_loops(),
        0.0,
        3.0
    ));
}

#[test]
fn conservative_rotated_masks_never_miss_vector_intersections_or_clearance() {
    use geo::{Area, BooleanOps, Distance, Euclidean, Polygon};
    use goat_packer::{PackedPlacement, placements_collide};
    let fp = rectangle(3.7, 0.09);
    let polygon = |p: &PackedPlacement| -> Polygon {
        Polygon::new(
            p.place_loops(&fp.loops)[0]
                .iter()
                .map(|p| (p.x, p.y))
                .collect::<Vec<_>>()
                .into(),
            vec![],
        )
    };
    for angle in [0.0, 13.0, 45.0, 90.0, 137.0] {
        for clearance in [0.0, 0.3, 0.75] {
            let a = rasterize_footprint(&fp, 0.0, 0.5, clearance);
            let b = rasterize_footprint(&fp, angle, 0.5, clearance);
            let pa = polygon(&PackedPlacement {
                x_mm: 0.0,
                y_mm: 0.0,
                angle_deg: 0.0,
            });
            for dx in -9..=9 {
                for dy in -9..=9 {
                    let pb = polygon(&PackedPlacement {
                        x_mm: f64::from(dx) * 0.5,
                        y_mm: f64::from(dy) * 0.5,
                        angle_deg: angle,
                    });
                    let conflict = pa.intersection(&pb).unsigned_area() > 1e-9
                        || Euclidean.distance(&pa, &pb) < clearance - 1e-7;
                    assert!(
                        !conflict || placements_collide(&a, &b, dx, dy),
                        "angle={angle}, gap={clearance}, offset={dx},{dy}"
                    );
                }
            }
        }
    }
}

#[test]
fn measuring_overshoot_of_a_subpixel_silhouette_terminates() {
    let silhouette = rectangle(0.05, 0.05);
    let patch = rectangle(1.5, 1.5);
    let overshoot = goat_packer::patch_overshoot_mm(&patch, &silhouette, 0.5);
    assert!(overshoot.is_finite() && overshoot > 0.0);
}

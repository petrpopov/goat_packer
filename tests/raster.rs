use goat_packer::{
    Footprint, Point, PrintKind, Rect, placement_is_allowed, placements_collide,
    rasterize_footprint,
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
    let horizontal = rasterize_footprint(&fp, 0.0, 0.5, 0.0);
    let vertical = rasterize_footprint(&fp, 90.0, 0.5, 0.0);
    let grown = rasterize_footprint(&fp, 0.0, 0.5, 1.0);
    let silhouette =
        |mask: &goat_packer::RasterMask| mask.mask(PrintKind::Silhouette, 0).cloned().unwrap();

    assert!(silhouette(&horizontal).width > silhouette(&horizontal).height);
    assert!(silhouette(&vertical).height > silhouette(&vertical).width);
    assert!(silhouette(&grown).pixel_count > silhouette(&horizontal).pixel_count);
}

#[test]
fn clearance_masks_block_close_shapes_even_when_the_shapes_do_not_touch() {
    let fp = rectangle(2.0, 2.0);
    let mask = rasterize_footprint(&fp, 0.0, 0.25, 1.0);

    // 2.5 mm apart: 0.5 mm between the squares.
    assert!(placements_collide(&mask, &mask, 10, 0));
    assert!(placements_collide(&mask, &mask, 11, 0));
    // 3 mm: exactly the clearance. Grid-aligned edges stay aligned when grown, so the
    // equality passes and costs no extra pixel.
    assert!(!placements_collide(&mask, &mask, 12, 0));
}

/// The clearance is a disc: diagonal neighbours keep the same Euclidean gap as axial ones.
/// The square kernel this replaced (0.7 mm became 2 px per side at 0.25 mm) forbade every
/// shift below 8 px in both axes, 1.41 mm of diagonal gap.
#[test]
fn diagonal_neighbours_keep_the_euclidean_gap_not_a_square_one() {
    let fp = rectangle(1.0, 1.0);
    let mask = rasterize_footprint(&fp, 0.0, 0.25, 0.7);
    // Axial: 2 mm apart leaves 1 mm, 1.5 mm leaves 0.5 mm.
    assert!(!placements_collide(&mask, &mask, 8, 0));
    assert!(placements_collide(&mask, &mask, 6, 0));
    // Diagonal: (1.75, 1.75) leaves 1.06 mm between the corners, more than the clearance
    // plus a pixel diagonal (the most a cover can add), so it must be admissible.
    assert!(!placements_collide(&mask, &mask, 7, 7));
    // (1.25, 1.25) leaves 0.25 mm in x and y: the corners are 0.35 mm apart.
    assert!(placements_collide(&mask, &mask, 5, 5));
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
    let silhouette = mask.mask(PrintKind::Silhouette, 0).unwrap();
    assert!(silhouette.contains_local(8, 0));
}

#[test]
fn body_outside_first_layer_patch_is_not_clipped() {
    let mut fp = rectangle(2.0, 2.0);
    fp.body_loops = rectangle(10.0, 2.0).loops;
    let mask = rasterize_footprint(&fp, 0.0, 0.5, 0.0);
    assert!(
        mask.mask(PrintKind::Silhouette, 0)
            .unwrap()
            .contains_local(8, 0)
    );
    assert_eq!(mask.extent_bounds.max_x, 5.0);
    assert!(!placement_is_allowed(
        &mask,
        4,
        10,
        Rect::new(0.0, 0.0, 20.0, 20.0),
        &[]
    ));
    assert!(placements_collide(&mask, &mask, 12, 0));
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
        let pixels = |kind, band| mask.mask(kind, band).unwrap().pixel_count;
        assert!(mask.mask(PrintKind::SupportBand, 3).is_none());
        assert_eq!(pixels(PrintKind::SupportBand, 0), 0);
        assert!(pixels(PrintKind::SupportBand, 1) > 0);
        assert_eq!(pixels(PrintKind::BodyBand, 1), 0);
        assert_eq!(pixels(PrintKind::SupportBand, 2), 0);
        assert!(pixels(PrintKind::BodyBand, 2) > 0);
    }
    let mask = rasterize_footprint(&fp, 0.0, 0.5, 0.0);
    assert_eq!(mask.extent_bounds.max_x, 4.0);
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
    assert!(placements_collide(&mask, &mask, 2, 2));
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
    let support = mask.mask(PrintKind::SupportBand, 1).unwrap();
    assert!(support.contains_local(12, 0));
    assert!(support.contains_local(0, 12));
    assert_eq!(mask.mask(PrintKind::BodyBand, 1).unwrap().pixel_count, 0);
    assert_eq!(mask.extent_bounds, Rect::new(-4.0, -4.0, 4.0, 4.0));
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
    use goat_packer::PackedPlacement;
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
                    // The round joins are inscribed chords: the masks may admit up to their
                    // sag, clearance * (1 - cos(pi / 100)), below the clearance.
                    let sag = clearance * (1.0 - (std::f64::consts::PI / 100.0).cos());
                    let conflict = pa.intersection(&pb).unsigned_area() > 1e-9
                        || Euclidean.distance(&pa, &pb) < clearance - sag - 1e-7;
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

/// Plate zones against kinds (design 2026-11-02, §7.2), on a hand-made print: a 4x4 mm body
/// (one part), a 2x2 mm landing, a support base strip right of the body and height bands
/// (bodies in bands 0-2, the last one narrower; a support in band 1).
mod zones {
    use goat_packer::keepout::{
        BodyRule, Keepout, KeepoutClass, KeepoutFrame, KeepoutShape, PlateRules, ZoneClass,
    };
    use goat_packer::kind_masks::{
        KindMasks, KindShapes, MaskRules, OriginGrid, PlateZone, ZoneArea, feasible_origins,
        forbidden_origins, plate_zones, zone_conflict,
    };
    use goat_packer::{ObjectPrint, Point, Rect};

    const RES: f64 = 0.25;

    fn square(cx: f64, cy: f64, half_x: f64, half_y: f64) -> Vec<Point> {
        let (x0, y0, x1, y1) = (cx - half_x, cy - half_y, cx + half_x, cy + half_y);
        [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
            .iter()
            .map(|&(x, y)| Point { x, y })
            .collect()
    }

    fn print(with_supports: bool) -> ObjectPrint {
        let body = square(0.0, 0.0, 2.0, 2.0);
        let support = |loops: Vec<Vec<Point>>| if with_supports { loops } else { vec![] };
        ObjectPrint {
            silhouette: vec![body.clone()],
            part_hulls: vec![body.clone()],
            part_boxes: vec![Rect::new(-2.0, -2.0, 2.0, 2.0)],
            landing: vec![square(0.0, 0.0, 1.0, 1.0)],
            support_base: support(vec![square(3.0, 0.0, 0.5, 1.0)]),
            body: vec![
                vec![body.clone()],
                vec![body],
                vec![square(0.0, 0.0, 1.0, 1.0)],
            ],
            support: vec![vec![], support(vec![square(3.25, 0.0, 0.75, 0.5)]), vec![]],
            interface: vec![vec![], vec![], vec![]],
            ..ObjectPrint::default()
        }
    }

    fn masks(with_supports: bool) -> KindMasks {
        let rules = MaskRules {
            clearance_mm: 0.7,
            body_rule: BodyRule::Silhouette,
            support_overlap_mm: None,
        };
        KindShapes::new(&print(with_supports), rules).rasterize(RES)
    }

    fn rules(keepouts: Vec<Keepout>) -> PlateRules {
        PlateRules {
            printable_area: Rect::new(0.0, 0.0, 40.0, 40.0),
            body_edge_band_mm: 0.0,
            print_edge_reserve_mm: 0.0,
            keepouts,
            body_rule: BodyRule::Silhouette,
        }
    }

    fn keepout(class: KeepoutClass, shape: KeepoutShape, z: Option<[f64; 2]>) -> Keepout {
        Keepout {
            id: format!("{class:?}"),
            class,
            shape,
            z_range_mm: z,
            frame: KeepoutFrame::Plate,
        }
    }

    const GRID: OriginGrid = OriginGrid {
        origin: Point { x: 0.0, y: 0.0 },
        resolution: RES,
    };

    /// The conflict at origin `(x, y)` mm (whole pixels).
    fn conflict(masks: &KindMasks, zones: &[PlateZone], x: f64, y: f64) -> Option<String> {
        zone_conflict(masks, zones, &GRID, (x / RES) as i32, (y / RES) as i32)
    }

    #[test]
    fn hard_zones_test_part_hulls_touching_passes_and_forbid_every_support() {
        let rect = KeepoutShape::Rect(Rect::new(10.0, 10.0, 14.0, 14.0));
        let zones =
            plate_zones(&rules(vec![keepout(KeepoutClass::Hard, rect, None)]), None).unwrap();
        let bare = masks(false);
        assert_eq!(
            conflict(&bare, &zones, 8.0, 12.0),
            None,
            "the hull touches the zone"
        );
        assert!(
            conflict(&bare, &zones, 8.25, 12.0)
                .unwrap()
                .contains("body enters")
        );
        // With supports the base (x 10.5..11.5 at origin 8) lands in the zone.
        let supported = masks(true);
        let error = conflict(&supported, &zones, 8.0, 12.0).unwrap();
        assert!(error.contains("SupportBase"), "{error}");
    }

    #[test]
    fn a_soft_strip_blocks_only_the_landing() {
        let strip = KeepoutShape::Strips(vec![[5.0, 20.0, 35.0, 20.0, 1.0]]);
        let soft = keepout(KeepoutClass::Soft, strip, Some([0.0, 0.2]));
        let zones = plate_zones(&rules(vec![soft]), None).unwrap();
        let supported = masks(true);
        // The body (y 20..24) covers the strip (y 19.5..20.5), the landing (y 21..23) does not.
        assert_eq!(conflict(&supported, &zones, 20.0, 22.0), None);
        let error = conflict(&supported, &zones, 20.0, 21.25).unwrap();
        assert!(error.contains("Landing"), "{error}");
    }

    #[test]
    fn the_restricted_band_tests_part_boxes_with_equality_and_lets_supports_in() {
        let mut plate = rules(vec![]);
        plate.body_edge_band_mm = 3.0;
        let zones = plate_zones(&plate, None).unwrap();
        let mut left = print(true);
        left.support_base = vec![square(-3.0, 0.0, 0.5, 1.0)];
        let rules = MaskRules {
            clearance_mm: 0.7,
            body_rule: BodyRule::Silhouette,
            support_overlap_mm: None,
        };
        let left = KindShapes::new(&left, rules).rasterize(RES);
        // Body box 3..7 touches the band's inner edge; the base (x 1.5..2.5) is in the band.
        assert_eq!(conflict(&left, &zones, 5.0, 20.0), None);
        assert!(
            conflict(&left, &zones, 4.75, 20.0)
                .unwrap()
                .contains("body leaves")
        );
    }

    #[test]
    fn the_print_edge_reserve_binds_support_material_only() {
        let mut plate = rules(vec![]);
        plate.print_edge_reserve_mm = 0.5;
        let zones = plate_zones(&plate, None).unwrap();
        let supported = masks(true);
        // The band-1 support reaches x + 4: 39.5 at origin 35.5.
        assert_eq!(conflict(&supported, &zones, 35.5, 20.0), None);
        let error = conflict(&supported, &zones, 35.75, 20.0).unwrap();
        assert!(error.contains("SupportBand"), "{error}");
        // The body alone may reach the printable edge.
        assert_eq!(conflict(&masks(false), &zones, 38.0, 20.0), None);
    }

    fn tower(class: ZoneClass, rect: Rect, z: Option<[f64; 2]>) -> PlateZone {
        PlateZone {
            id: format!("{class:?}"),
            class,
            area: ZoneArea::Rect(rect),
            z_range_mm: z,
        }
    }

    #[test]
    fn the_tower_body_keeps_bodies_the_clearance_and_supports_the_support_gap() {
        let mut zones = plate_zones(&rules(vec![]), None).unwrap();
        zones.push(tower(
            ZoneClass::TowerBody,
            Rect::new(20.0, 20.0, 25.0, 25.0),
            None,
        ));
        let supported = masks(true);
        // Body right edge 18.5 at origin 16.5: 1.5 mm from the tower; the support 0.5 mm.
        let error = conflict(&supported, &zones, 16.5, 22.0).unwrap();
        assert!(
            error.contains("SupportBand") || error.contains("SupportBase"),
            "{error}"
        );
        // At 15: the support ends at 19 (1 mm off), the body at 17.
        assert_eq!(conflict(&supported, &zones, 15.0, 22.0), None);
        // Bodies keep the clearance: 0.5 mm is too close for the bare print.
        assert!(conflict(&masks(false), &zones, 17.5, 22.0).is_some());
        assert_eq!(conflict(&masks(false), &zones, 16.75, 22.0), None);
    }

    #[test]
    fn the_tower_brim_admits_supports_and_binds_only_the_bodies_of_its_bands() {
        let brim = Rect::new(19.0, 19.0, 26.0, 26.0);
        let mut zones = plate_zones(&rules(vec![]), None).unwrap();
        zones.push(tower(ZoneClass::TowerBrim, brim, Some([0.0, 3.0])));
        let supported = masks(true);
        // Origin 15.25: the band-1 support (x 17.75..19.25) lies on the brim; the bodies end
        // at 17.25, 1.75 mm off.
        assert_eq!(conflict(&supported, &zones, 15.25, 22.0), None);
        // Origin 16.5: the low bodies are 0.5 mm from the brim.
        assert!(
            conflict(&supported, &zones, 16.5, 22.0)
                .unwrap()
                .contains("BodyBand")
        );
        // A brim only in band 2 (z 4..6) meets the narrow top body (x 15.5..17.5) only.
        let mut high = plate_zones(&rules(vec![]), None).unwrap();
        high.push(tower(ZoneClass::TowerBrim, brim, Some([4.5, 5.0])));
        assert_eq!(conflict(&supported, &high, 16.5, 22.0), None);
    }

    #[test]
    fn a_z_limited_hard_zone_meets_only_the_supports_of_its_bands() {
        let rect = KeepoutShape::Rect(Rect::new(19.0, 0.0, 22.0, 40.0));
        let high = keepout(KeepoutClass::Hard, rect.clone(), Some([4.5, 5.0]));
        let low = keepout(KeepoutClass::Hard, rect, Some([2.5, 3.0]));
        let supported = masks(true);
        // Origin 15.5: the band-1 support (x 17.75..19.75) enters x >= 19; the hull stays out.
        let zones = |k: Keepout| plate_zones(&rules(vec![k]), None).unwrap();
        assert_eq!(conflict(&supported, &zones(high), 15.5, 20.0), None);
        assert!(conflict(&supported, &zones(low), 15.5, 20.0).is_some());
    }

    #[test]
    fn a_movable_zone_is_a_removable_layer_over_the_base_map() {
        let base_zones = plate_zones(&rules(vec![]), None).unwrap();
        let supported = masks(true);
        let base = feasible_origins(&supported, &base_zones, &GRID);
        let towers = [tower(
            ZoneClass::TowerBody,
            Rect::new(20.0, 20.0, 25.0, 25.0),
            None,
        )];
        let layer = forbidden_origins(&supported, &towers, &GRID, &base);
        let mut all = base_zones.clone();
        all.extend(towers.iter().cloned());
        let direct = feasible_origins(&supported, &all, &GRID);
        assert_eq!(base.without(&layer), direct);
        assert_eq!(base.count(), direct.count() + layer.count());
        assert!(layer.count() > 0);
    }

    #[test]
    fn gcode_frame_keepouts_need_the_extruder_offset() {
        let mut strip = keepout(
            KeepoutClass::Soft,
            KeepoutShape::Strips(vec![[18.0, 1.0, 240.0, 1.0, 0.81]]),
            Some([0.0, 0.2]),
        );
        strip.frame = KeepoutFrame::Gcode;
        let plate = rules(vec![strip]);
        assert!(plate_zones(&plate, None).is_err());
        let zones = plate_zones(&plate, Some(Point { x: 0.0, y: 2.0 })).unwrap();
        let ZoneArea::Loops(loops) = &zones[1].area else {
            panic!("a strip is a polygon zone: {:?}", zones[1]);
        };
        let bounds = Rect::of_points(loops.iter().flatten());
        assert!((bounds.min_y - (3.0 - 0.405)).abs() < 1e-9, "{bounds:?}");
        assert!((bounds.min_x - (18.0 - 0.405)).abs() < 1e-9, "{bounds:?}");
    }
}

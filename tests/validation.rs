//! The validation split (design 2026-11-02, §8.5): `validate_raster` for every shortlisted
//! candidate, `validate_vector` for the published winner. Generated prints only.
//!
//! The print is an L-shaped body (concave, so its hull is larger than its silhouette) on a
//! wider support base, with two height bands: band 0 holds the foot, band 1 the upright and
//! a support column to its right.

use goat_packer::keepout::{
    BodyRule, Keepout, KeepoutClass, KeepoutFrame, KeepoutShape, PlateRules, ZoneClass,
};
use goat_packer::kind_masks::{OriginGrid, PlateZone, ZoneArea};
use goat_packer::{
    LayoutRules, ObjectPrint, Point, PrintPlacement, Rect, validate_raster, validate_vector,
};

const CLEARANCE: f64 = 0.7;

fn ring(points: &[(f64, f64)]) -> Vec<Point> {
    let mut ring: Vec<Point> = points.iter().map(|&(x, y)| Point { x, y }).collect();
    ring.push(ring[0]);
    ring
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
    ring(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

fn print() -> ObjectPrint {
    let l_shape = ring(&[
        (-3.0, -2.0),
        (3.0, -2.0),
        (3.0, -1.0),
        (-2.0, -1.0),
        (-2.0, 2.0),
        (-3.0, 2.0),
    ]);
    ObjectPrint {
        part_hulls: vec![ring(&[
            (-3.0, -2.0),
            (3.0, -2.0),
            (3.0, -1.0),
            (-2.0, 2.0),
            (-3.0, 2.0),
        ])],
        part_boxes: vec![Rect::new(-3.0, -2.0, 3.0, 2.0)],
        silhouette: vec![l_shape],
        landing: vec![rect(-3.0, -2.0, 3.0, -1.5)],
        support_base: vec![rect(-4.0, -3.0, 4.0, -2.0)],
        body: vec![
            vec![rect(-3.0, -2.0, 3.0, -1.0)],
            vec![rect(-3.0, -1.0, -2.0, 2.0)],
        ],
        support: vec![vec![], vec![rect(3.5, -1.0, 5.0, 0.0)]],
        interface: vec![vec![], vec![]],
        ..ObjectPrint::default()
    }
}

/// A plain 2x2 mm body, nothing else: for exact gap measurements.
fn square_print() -> ObjectPrint {
    ObjectPrint {
        silhouette: vec![rect(-1.0, -1.0, 1.0, 1.0)],
        part_hulls: vec![rect(-1.0, -1.0, 1.0, 1.0)],
        part_boxes: vec![Rect::new(-1.0, -1.0, 1.0, 1.0)],
        landing: vec![rect(-1.0, -1.0, 1.0, 1.0)],
        ..ObjectPrint::default()
    }
}

fn plate(keepouts: Vec<Keepout>, body_edge_band_mm: f64) -> PlateRules {
    PlateRules {
        printable_area: Rect::new(0.0, 0.0, 100.0, 100.0),
        body_edge_band_mm,
        print_edge_reserve_mm: 0.0,
        keepouts,
        body_rule: BodyRule::Silhouette,
    }
}

fn keepout(class: KeepoutClass, shape: KeepoutShape) -> Keepout {
    Keepout {
        id: format!("{class:?}"),
        class,
        shape,
        z_range_mm: None,
        frame: KeepoutFrame::Plate,
    }
}

fn rules(keepouts: Vec<Keepout>) -> LayoutRules {
    LayoutRules::new(&plate(keepouts, 0.0), None, CLEARANCE, Default::default(), None).unwrap()
}

fn at(points: &[(f64, f64)]) -> Vec<PrintPlacement> {
    points
        .iter()
        .map(|&(x_mm, y_mm)| PrintPlacement {
            print: 0,
            x_mm,
            y_mm,
        })
        .collect()
}

fn grid() -> OriginGrid {
    OriginGrid {
        origin: Point { x: 0.0, y: 0.0 },
        resolution: 0.25,
    }
}

fn vector(print: &ObjectPrint, rules: &LayoutRules, points: &[(f64, f64)]) -> Result<(), String> {
    validate_vector(std::slice::from_ref(print), &at(points), rules)
}

fn raster(print: &ObjectPrint, rules: &LayoutRules, points: &[(f64, f64)]) -> Result<(), String> {
    validate_raster(std::slice::from_ref(print), &at(points), rules, grid())
}

/// The print moved by `(dx, dy)`: a copy that is its own class, placed at the origin.
fn translated(print: &ObjectPrint, dx: f64, dy: f64) -> ObjectPrint {
    let map = |loops: &Vec<Vec<Point>>| -> Vec<Vec<Point>> {
        let moved = |p: &Point| Point {
            x: p.x + dx,
            y: p.y + dy,
        };
        loops
            .iter()
            .map(|ring| ring.iter().map(moved).collect())
            .collect()
    };
    ObjectPrint {
        silhouette: map(&print.silhouette),
        part_hulls: map(&print.part_hulls),
        part_boxes: print
            .part_boxes
            .iter()
            .map(|b| b.translated(dx, dy))
            .collect(),
        landing: map(&print.landing),
        brim: map(&print.brim),
        support_base: map(&print.support_base),
        body: print.body.iter().map(map).collect(),
        support: print.support.iter().map(map).collect(),
        interface: print.interface.iter().map(map).collect(),
        ..print.clone()
    }
}

#[test]
fn the_vector_check_catches_a_violation_the_raster_check_misses() {
    // The band-1 support collapsed to its centreline: a ring with no area. The masks grow
    // kinds through a polygon pipeline that drops it; the vector check reads the rings
    // themselves and measures the line 0.2 mm from the neighbour's band-1 body.
    let mut sliver = print();
    sliver.support[1] = vec![ring(&[(3.5, -0.5), (5.0, -0.5)])];
    let rules = rules(vec![]);
    let layout = [(20.0, 20.0), (28.2, 20.0)];
    assert!(raster(&sliver, &rules, &layout).is_ok());
    let error = vector(&sliver, &rules, &layout).unwrap_err();
    assert!(
        error.contains("SupportBand of 0 within 0.5 mm of BodyBand of 1 in band 1"),
        "{error}"
    );
    // The same column with its area: both checks see it.
    assert!(raster(&print(), &rules, &layout).is_err());
    assert!(vector(&print(), &rules, &layout).is_err());
}

#[test]
fn shapes_cached_per_class_give_the_same_verdicts_as_shapes_per_copy() {
    let hard = keepout(
        KeepoutClass::Hard,
        KeepoutShape::Rect(Rect::new(60.0, 60.0, 70.0, 70.0)),
    );
    let rules = rules(vec![hard]);
    // Sub-pixel phases, shared by some copies (one cached mask) and not by others; whether
    // each layout is valid.
    let layouts: [(&[(f64, f64)], bool); 5] = [
        (
            &[
                (20.1, 20.1),
                (30.1, 20.1),
                (40.1, 20.1),
                (20.1, 30.1),
                (30.03, 30.07),
            ],
            true,
        ),
        // Bodies 0.4 mm apart (clearance 0.7).
        (&[(20.1, 20.1), (40.0, 40.0), (26.5, 20.1)], false),
        // The band-1 support 0.3 mm from the neighbour's band-1 body.
        (&[(20.1, 20.1), (28.4, 20.1)], false),
        // A part hull in the hard zone.
        (&[(20.1, 20.1), (62.2, 58.5)], false),
        // Everything far apart and clear.
        (&[(10.0, 10.0), (50.0, 50.0), (80.1, 20.7)], true),
    ];
    for (points, valid) in layouts {
        let copies: Vec<ObjectPrint> = points
            .iter()
            .map(|&(x, y)| translated(&print(), x, y))
            .collect();
        let origins: Vec<PrintPlacement> = (0..points.len())
            .map(|print| PrintPlacement {
                print,
                x_mm: 0.0,
                y_mm: 0.0,
            })
            .collect();
        let cached = vector(&print(), &rules, points);
        assert_eq!(cached.is_ok(), valid, "{points:?}: {cached:?}");
        assert_eq!(cached, validate_vector(&copies, &origins, &rules));
        let cached = raster(&print(), &rules, points);
        assert_eq!(cached.is_ok(), valid, "{points:?}: {cached:?}");
        assert_eq!(cached, validate_raster(&copies, &origins, &rules, grid()));
    }
}

#[test]
fn the_first_conflict_in_placement_order_does_not_depend_on_the_thread_count() {
    let hard = keepout(
        KeepoutClass::Hard,
        KeepoutShape::Rect(Rect::new(60.0, 60.0, 70.0, 70.0)),
    );
    let rules = rules(vec![hard]);
    // Pairs (0, 3) and (1, 2) collide, placement 4 enters the hard zone: (1, 2) comes first.
    let layout = [
        (20.0, 20.0),
        (40.0, 40.0),
        (46.5, 40.0),
        (26.5, 20.0),
        (62.0, 62.0),
    ];
    let run = |threads| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            (
                vector(&print(), &rules, &layout),
                raster(&print(), &rules, &layout),
            )
        })
    };
    let (vector_one, raster_one) = run(1);
    assert_eq!(run(4), (vector_one.clone(), raster_one.clone()));
    assert!(vector_one.unwrap_err().starts_with("placements 1 and 2 "));
    assert!(raster_one.unwrap_err().starts_with("placements 1 and 2 "));
}

#[test]
fn vector_gaps_are_exact_within_the_join_sag() {
    let rules = rules(vec![]);
    let square = square_print();
    // Axis: 2 mm bodies 0.7 mm apart. The masks' round joins may sag 0.35 um into the gap.
    assert!(vector(&square, &rules, &[(10.0, 10.0), (12.7, 10.0)]).is_ok());
    assert!(vector(&square, &rules, &[(10.0, 10.0), (12.6999, 10.0)]).is_ok());
    assert!(vector(&square, &rules, &[(10.0, 10.0), (12.699, 10.0)]).is_err());
    // Diagonal: corner to corner.
    let d = 2.0 + CLEARANCE / 2f64.sqrt();
    assert!(vector(&square, &rules, &[(10.0, 10.0), (10.0 + d, 10.0 + d)]).is_ok());
    assert!(
        vector(
            &square,
            &rules,
            &[(10.0, 10.0), (10.0 + d - 0.001, 10.0 + d)]
        )
        .is_err()
    );
}

#[test]
fn part_hulls_may_touch_a_hard_zone_but_not_enter_it() {
    // A hard zone in the L's concave corner: clear of the silhouette, inside the hull.
    let corner = Rect::new(18.5, 19.5, 19.0, 20.0);
    let rules = rules(vec![keepout(
        KeepoutClass::Hard,
        KeepoutShape::Rect(corner),
    )]);
    assert!(
        vector(&print(), &rules, &[(20.0, 20.0)])
            .unwrap_err()
            .contains("Hard zone")
    );
    // The same zone touching the right side of the foot.
    let touching = Rect::new(23.0, 18.0, 25.0, 19.0);
    let rules = self::rules(vec![keepout(
        KeepoutClass::Hard,
        KeepoutShape::Rect(touching),
    )]);
    let mut bodies_only = print();
    bodies_only.support = vec![vec![], vec![]];
    bodies_only.support_base.clear();
    assert!(vector(&bodies_only, &rules, &[(20.0, 20.0)]).is_ok());
    assert!(vector(&bodies_only, &rules, &[(20.01, 20.0)]).is_err());
}

#[test]
fn part_boxes_may_meet_the_restricted_edge_band() {
    let rules = LayoutRules::new(&plate(vec![], 3.5), None, CLEARANCE, Default::default(), None).unwrap();
    // The part box reaches x = -3 + 6.5 = 3.5: exactly the band.
    assert!(vector(&print(), &rules, &[(6.5, 50.0)]).is_ok());
    assert!(vector(&print(), &rules, &[(6.49, 50.0)]).is_err());
}

#[test]
fn a_soft_strip_blocks_the_landing_only() {
    // A purge line along y = 17.5 under the support base (y 17..18 at y = 20).
    let strip = keepout(
        KeepoutClass::Soft,
        KeepoutShape::Strips(vec![[0.0, 17.5, 100.0, 17.5, 0.4]]),
    );
    let rules = rules(vec![strip]);
    assert!(vector(&print(), &rules, &[(20.0, 20.0)]).is_ok());
    // The landing's lower edge (y = -2) on the strip's upper edge (17.7): touching passes.
    assert!(vector(&print(), &rules, &[(20.0, 19.7)]).is_ok());
    let error = vector(&print(), &rules, &[(20.0, 19.6)]).unwrap_err();
    assert!(error.contains("Soft zone"), "{error}");
}

#[test]
fn the_tower_body_keeps_supports_away_and_its_brim_only_low_bodies() {
    let mut rules = rules(vec![]);
    let tower = |class, area, z_range_mm| PlateZone {
        id: format!("{class:?}"),
        class,
        area: ZoneArea::Rect(area),
        z_range_mm,
    };
    // Tower body 0.4 mm right of the band-1 support column (x 25 at x = 20).
    rules.zones.push(tower(
        ZoneClass::TowerBody,
        Rect::new(25.4, 15.0, 35.0, 25.0),
        None,
    ));
    let error = vector(&print(), &rules, &[(20.0, 20.0)]).unwrap_err();
    assert!(error.contains("TowerBody"), "{error}");
    assert!(vector(&print(), &rules, &[(19.8, 20.0)]).is_ok());
    // Brims up to z 0.5: one under the support base (allowed), one beside the band-1 upright
    // (never at its height). The band-0 foot and the landing keep the clearance.
    rules.zones.pop();
    let brim = |area| tower(ZoneClass::TowerBrim, area, Some([0.0, 0.5]));
    rules.zones.push(brim(Rect::new(15.0, 16.0, 25.0, 17.2)));
    rules.zones.push(brim(Rect::new(10.0, 20.0, 16.75, 23.0)));
    assert!(vector(&print(), &rules, &[(20.0, 20.0)]).is_ok());
    assert!(vector(&print(), &rules, &[(19.6, 20.0)]).is_ok());
    let error = vector(&print(), &rules, &[(20.0, 19.6)]).unwrap_err();
    assert!(error.contains("TowerBrim"), "{error}");
}

#[test]
fn bad_layouts_are_errors_not_panics() {
    let rules = rules(vec![]);
    let unknown = [PrintPlacement {
        print: 1,
        x_mm: 10.0,
        y_mm: 10.0,
    }];
    assert!(validate_vector(&[print()], &unknown, &rules).is_err());
    assert!(validate_raster(&[print()], &unknown, &rules, grid()).is_err());
    assert!(vector(&print(), &rules, &[(f64::NAN, 10.0)]).is_err());
    let mut broken = print();
    broken.support[1][0][2].x = f64::NAN;
    assert!(vector(&broken, &rules, &[(10.0, 10.0)]).is_err());
    assert!(raster(&broken, &rules, &[(10.0, 10.0)]).is_err());
    let negative = LayoutRules {
        clearance_mm: -1.0,
        ..rules.clone()
    };
    assert!(vector(&print(), &negative, &[(10.0, 10.0)]).is_err());
}

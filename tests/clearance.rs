//! Euclidean clearance (design 2026-11-02, §8.1): kinds are grown by half their gap as
//! vectors with round joins and then covered conservatively. Over generated polygons at every
//! integer shift: a shift the raster admits keeps the true vector distance at least the gap
//! (less the micron-scale chord sag of the joins), and a shift the raster forbids is closer
//! than the gap plus one pixel diagonal.
use geo::{Area, BooleanOps, Distance, Euclidean, Intersects, LineString, Polygon};
use goat_packer::keepout::{
    BodyRule, Keepout, KeepoutClass, KeepoutFrame, KeepoutShape, PlateRules,
};
use goat_packer::kind_masks::{
    KindMasks, KindShapes, MaskRules, OriginGrid, feasible_origins, plate_zones,
};
use goat_packer::{ObjectPrint, Point, Rect, placements_collide};

/// A fixed-seed generator: the same polygons on every run and machine.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next()
    }
}

/// A star-shaped (so simple) polygon around the origin with 5..13 vertices.
fn star(rng: &mut Lcg) -> Vec<Point> {
    let count = 5 + (rng.next() * 9.0) as usize;
    let mut angles: Vec<f64> = (0..count)
        .map(|_| rng.between(0.0, std::f64::consts::TAU))
        .collect();
    angles.sort_by(f64::total_cmp);
    let mut ring: Vec<Point> = angles
        .iter()
        .map(|&a| {
            let r = rng.between(0.6, 3.0);
            Point {
                x: r * a.cos(),
                y: r * a.sin(),
            }
        })
        .collect();
    ring.push(ring[0]);
    ring
}

/// A rectangle a hair larger than whole pixels on every side. Grown by a whole number of
/// pixels its sides spill a hair into the next pixel, so the cover is as tight as it gets:
/// a mask grown even slightly less than the gap would admit a shift a hair too close.
fn snug_rectangle(rng: &mut Lcg, resolution: f64) -> Vec<Point> {
    const HAIR: f64 = 0.003;
    let half = |rng: &mut Lcg| (1.0 + (rng.next() * 4.0).floor()) * resolution + HAIR;
    let (w, h) = (half(rng), half(rng));
    let mut ring: Vec<Point> = [(-w, -h), (w, -h), (w, h), (-w, h)]
        .iter()
        .map(|&(x, y)| Point { x, y })
        .collect();
    ring.push(ring[0]);
    ring
}

fn silhouette_print(ring: &[Point]) -> ObjectPrint {
    ObjectPrint {
        silhouette: vec![ring.to_vec()],
        part_hulls: vec![ring.to_vec()],
        part_boxes: vec![Rect::of_points(ring.iter())],
        landing: vec![ring.to_vec()],
        ..ObjectPrint::default()
    }
}

fn polygon(ring: &[Point], dx: f64, dy: f64) -> Polygon {
    Polygon::new(
        LineString::from(
            ring.iter()
                .map(|p| (p.x + dx, p.y + dy))
                .collect::<Vec<_>>(),
        ),
        vec![],
    )
}

fn distance(a: &Polygon, b: &Polygon) -> f64 {
    if a.intersects(b) {
        0.0
    } else {
        Euclidean.distance(a, b)
    }
}

/// The chord sag both round joins may give away (`kind_masks::buffer`).
fn sag(gap: f64) -> f64 {
    gap * (1.0 - (std::f64::consts::PI / 100.0).cos())
}

fn masks(ring: &[Point], gap: f64, resolution: f64) -> KindMasks {
    let rules = MaskRules {
        clearance_mm: gap,
        foreign: Default::default(),
        body_rule: BodyRule::Silhouette,
        support_overlap_mm: None,
    };
    KindShapes::new(&silhouette_print(ring), rules).rasterize(resolution)
}

#[test]
fn raster_admissible_shifts_keep_the_euclidean_gap_and_over_reserve_at_most_a_pixel_diagonal() {
    let mut rng = Lcg(20_261_102);
    let mut admitted = 0usize;
    for case in 0..48 {
        let gap = [0.0, 0.3, 0.5, 0.7, 1.0][case % 5];
        let resolution = [0.25, 0.5][case % 2];
        let (a, b) = if case < 24 {
            (star(&mut rng), star(&mut rng))
        } else {
            (
                snug_rectangle(&mut rng, resolution),
                snug_rectangle(&mut rng, resolution),
            )
        };
        let (ma, mb) = (masks(&a, gap, resolution), masks(&b, gap, resolution));
        let pa = polygon(&a, 0.0, 0.0);
        let reach = ((6.0 + gap) / resolution).ceil() as i32 + 2;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let pb = polygon(&b, f64::from(dx) * resolution, f64::from(dy) * resolution);
                let d = distance(&pa, &pb);
                if placements_collide(&ma, &mb, dx, dy) {
                    assert!(
                        d <= gap + std::f64::consts::SQRT_2 * resolution + 1e-7,
                        "case {case}: shift {dx},{dy} forbidden at distance {d}, gap {gap}"
                    );
                } else {
                    admitted += 1;
                    assert!(
                        d >= gap - sag(gap) - 1e-7,
                        "case {case}: shift {dx},{dy} admitted at distance {d}, gap {gap}"
                    );
                    // Gap 0: touching passes, positive-area overlap never does.
                    let overlap = pa.intersection(&pb).unsigned_area();
                    assert!(
                        overlap < 1e-9,
                        "case {case}: shift {dx},{dy} overlaps {overlap}"
                    );
                }
            }
        }
    }
    assert!(admitted > 0);
}

/// A zone with a gap (the tower body keeps bodies the clearance away) follows the same
/// contract: an admitted origin keeps the gap, a forbidden one is within a pixel diagonal.
#[test]
fn zone_gaps_are_euclidean_too() {
    let mut rng = Lcg(7);
    for case in 0..12 {
        let ring = star(&mut rng);
        let gap = [0.5, 0.7, 1.0][case % 3];
        let resolution = [0.25, 0.5][case % 2];
        let print = masks(&ring, gap, resolution);
        let tower = Rect::new(10.0, 10.0, 13.5, 12.75);
        let rules = PlateRules {
            printable_area: Rect::new(0.0, 0.0, 30.0, 30.0),
            body_edge_band_mm: 0.0,
            print_edge_reserve_mm: 0.0,
            keepouts: Vec::new(),
            body_rule: BodyRule::Silhouette,
        };
        let mut zones = plate_zones(&rules, None).unwrap();
        zones.push(goat_packer::kind_masks::PlateZone {
            id: "tower".to_owned(),
            class: goat_packer::keepout::ZoneClass::TowerBody,
            area: goat_packer::kind_masks::ZoneArea::Rect(tower),
            z_range_mm: None,
        });
        let grid = OriginGrid {
            origin: Point { x: 0.0, y: 0.0 },
            resolution,
        };
        let origins = feasible_origins(&print, &zones, &grid);
        let free = feasible_origins(&print, &zones[..1], &grid);
        let tower_polygon = polygon(
            &[
                Point {
                    x: tower.min_x,
                    y: tower.min_y,
                },
                Point {
                    x: tower.max_x,
                    y: tower.min_y,
                },
                Point {
                    x: tower.max_x,
                    y: tower.max_y,
                },
                Point {
                    x: tower.min_x,
                    y: tower.max_y,
                },
                Point {
                    x: tower.min_x,
                    y: tower.min_y,
                },
            ],
            0.0,
            0.0,
        );
        let (x0, y0, x1, y1) = free.bounds().unwrap();
        for y in y0..=y1 {
            for x in x0..=x1 {
                if !free.contains(x, y) {
                    continue;
                }
                let placed = polygon(&ring, f64::from(x) * resolution, f64::from(y) * resolution);
                let d = distance(&placed, &tower_polygon);
                if origins.contains(x, y) {
                    assert!(d >= gap - sag(gap) - 1e-7, "case {case}: {x},{y} at {d}");
                } else {
                    let bound = gap + std::f64::consts::SQRT_2 * resolution + 1e-7;
                    assert!(d <= bound, "case {case}: {x},{y} forbidden at {d}");
                }
            }
        }
    }
}

/// The search's base map and the per-origin check (validation) are one predicate.
#[test]
fn the_base_map_and_the_single_origin_check_agree() {
    let mut rng = Lcg(42);
    let ring = star(&mut rng);
    let mut print = silhouette_print(&ring);
    print.support_base = vec![
        star(&mut rng)
            .iter()
            .map(|p| Point {
                x: p.x * 1.6,
                y: p.y * 1.6,
            })
            .collect(),
    ];
    print.landing = vec![
        star(&mut rng)
            .iter()
            .map(|p| Point {
                x: p.x * 0.5,
                y: p.y * 0.5,
            })
            .collect(),
    ];
    let rules = MaskRules {
        clearance_mm: 0.7,
        foreign: Default::default(),
        body_rule: BodyRule::Silhouette,
        support_overlap_mm: None,
    };
    let masks = KindShapes::new(&print, rules).rasterize(0.5);
    let keepout = |id: &str, class, shape| Keepout {
        id: id.to_owned(),
        class,
        shape,
        z_range_mm: None,
        frame: KeepoutFrame::Plate,
    };
    let plate = PlateRules {
        printable_area: Rect::new(0.0, 0.0, 40.0, 30.0),
        body_edge_band_mm: 2.0,
        print_edge_reserve_mm: 0.5,
        keepouts: vec![
            keepout(
                "hard",
                KeepoutClass::Hard,
                KeepoutShape::Rect(Rect::new(0.0, 0.0, 9.0, 7.0)),
            ),
            keepout(
                "restricted",
                KeepoutClass::Restricted,
                KeepoutShape::Rect(Rect::new(30.0, 20.0, 33.0, 30.0)),
            ),
            keepout(
                "purge",
                KeepoutClass::Soft,
                KeepoutShape::Strips(vec![
                    [12.0, 1.0, 35.0, 1.0, 1.8],
                    [35.0, 1.0, 35.0, 12.0, 0.8],
                ]),
            ),
            keepout(
                "cut",
                KeepoutClass::Hard,
                KeepoutShape::Polygon(vec![
                    Point { x: 15.0, y: 30.0 },
                    Point { x: 25.0, y: 30.0 },
                    Point { x: 20.0, y: 22.0 },
                ]),
            ),
        ],
        body_rule: BodyRule::Silhouette,
    };
    let zones = plate_zones(&plate, None).unwrap();
    let grid = OriginGrid {
        origin: Point { x: 0.0, y: 0.0 },
        resolution: 0.5,
    };
    let origins = feasible_origins(&masks, &zones, &grid);
    assert!(origins.count() > 0);
    let mut forbidden = 0;
    for y in -5..70 {
        for x in -5..90 {
            let conflict = goat_packer::kind_masks::zone_conflict(&masks, &zones, &grid, x, y);
            assert_eq!(
                origins.contains(x, y),
                conflict.is_none(),
                "{x},{y}: {conflict:?}"
            );
            forbidden += usize::from(conflict.is_some());
        }
    }
    assert!(forbidden > 0);
}

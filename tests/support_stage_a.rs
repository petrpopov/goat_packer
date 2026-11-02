//! Support model stage A (work package S2) on generated data only: the polygon facade against
//! `geo` on random polygons, the derived constants of spec 01 §4 / 02 §1.2 per flavour grid,
//! the layer grid, the unsupported settings of design §5.7, overhang detection on generated
//! meshes (cantilever, shelf, sharp tail, painted enforcer, guard), the clearance volumes and
//! determinism.

use std::collections::BTreeMap;

use geo::{Area, BooleanOps, Buffer, Coord, Intersects, LineString, MultiPolygon, Polygon};
use goat_packer::embedded_mesh::{ObjectMesh, Part, PartKind};
use goat_packer::polygon_ops::{
    self as ops, Grid, IntPoint, Join, Ring, Rings, area, bbox, contains, intersection, offset,
};
use goat_packer::support_model::clearance::{Avoidance, BedPlacement, Clearance, bed_obstacle};
use goat_packer::support_model::flavour::Flavour;
use goat_packer::support_model::layers::LayerGrid;
use goat_packer::support_model::settings::{Derived, ProjectConfig, SupportSettings};
use goat_packer::support_model::{StageA, StageAInput, SupportError, stage_a};

// ---------------------------------------------------------------------------------------
// Polygon facade against geo.

/// A fixed-seed xorshift generator: the property tests are reproducible.
struct Random(u64);

impl Random {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next()
    }
}

/// A random star-shaped (hence simple) counter-clockwise polygon on the integer grid.
fn star(random: &mut Random, centre: [f64; 2], scale: f64) -> Ring {
    let count = 5 + (random.next() * 40.0) as usize;
    let mut angles: Vec<f64> = (0..count)
        .map(|_| random.range(0.0, std::f64::consts::TAU))
        .collect();
    angles.sort_by(f64::total_cmp);
    angles.dedup();
    let mut ring: Ring = angles
        .iter()
        .map(|angle| {
            let radius = scale * random.range(0.3, 1.0);
            IntPoint::new(
                (centre[0] + radius * angle.cos()) as i32,
                (centre[1] + radius * angle.sin()) as i32,
            )
        })
        .collect();
    ring.dedup();
    ring
}

fn to_geo(rings: &[Ring]) -> MultiPolygon<f64> {
    let polygons = ops::islands(rings)
        .iter()
        .map(|island| {
            let line = |ring: &Ring| {
                LineString::from(
                    ring.iter()
                        .map(|p| Coord {
                            x: f64::from(p.x),
                            y: f64::from(p.y),
                        })
                        .collect::<Vec<_>>(),
                )
            };
            Polygon::new(line(&island[0]), island[1..].iter().map(line).collect())
        })
        .collect();
    MultiPolygon(polygons)
}

fn close(ours: f64, theirs: f64, tolerance: f64) -> bool {
    (ours - theirs).abs() <= tolerance * theirs.abs().max(1.0)
}

#[test]
fn booleans_match_geo_on_random_polygons() {
    let mut random = Random(0x5eed_1234_abcd_0001);
    for _ in 0..200 {
        let a = vec![star(&mut random, [0.0, 0.0], 100_000.0)];
        let centre = [
            random.range(-80_000.0, 80_000.0),
            random.range(-80_000.0, 80_000.0),
        ];
        let b = vec![star(&mut random, centre, 100_000.0)];
        let (geo_a, geo_b) = (to_geo(&a), to_geo(&b));
        // Areas agree up to the snapping of crossings to the integer grid (≤ 1 unit along
        // edges of ~10⁵ units).
        let tolerance = 1e-4;
        assert!(close(
            area(&ops::union(&a, &b)),
            geo_a.union(&geo_b).unsigned_area(),
            tolerance
        ));
        let (ours, theirs) = (
            area(&intersection(&a, &b)),
            geo_a.intersection(&geo_b).unsigned_area(),
        );
        assert!(
            close(ours, theirs, tolerance),
            "{ours} vs {theirs}: {a:?} {b:?}"
        );
        assert!(close(
            area(&ops::difference(&a, &b)),
            geo_a.difference(&geo_b).unsigned_area(),
            tolerance
        ));
        assert!(close(
            area(&ops::xor(&a, &b)),
            geo_a.xor(&geo_b).unsigned_area(),
            tolerance
        ));
        let sum = area(&ops::union(&a, &b)) + area(&intersection(&a, &b));
        assert!(close(sum, area(&a) + area(&b), tolerance));
    }
}

#[test]
fn containment_matches_geo() {
    let mut random = Random(0x5eed_0000_0000_0002);
    for _ in 0..50 {
        let rings = vec![star(&mut random, [0.0, 0.0], 50_000.0)];
        let shape = to_geo(&rings);
        for _ in 0..50 {
            let x = random.range(-60_000.0, 60_000.0) as i32;
            let point = IntPoint::new(x, random.range(-60_000.0, 60_000.0) as i32);
            let theirs = shape.intersects(&geo::Point::new(f64::from(point.x), f64::from(point.y)));
            assert_eq!(contains(&rings, point), theirs, "{point:?}");
        }
    }
}

#[test]
fn round_offsets_match_geo_buffers() {
    let mut random = Random(0x5eed_0000_0000_0003);
    for _ in 0..60 {
        let rings = vec![star(&mut random, [0.0, 0.0], 100_000.0)];
        let distance = random.range(2_000.0, 30_000.0);
        let ours = offset(&rings, distance, Join::Round(distance / 400.0));
        let theirs = to_geo(&rings).buffer(distance).unsigned_area();
        assert!(
            close(area(&ours), theirs, 2e-3),
            "{} vs {theirs}",
            area(&ours)
        );
        // Every original vertex is inside the grown area, at least `distance` from its edge.
        for &vertex in &rings[0] {
            assert!(contains(&ours, vertex));
            // Input decimation (0.005·d) plus the chord sag of the arcs (d/400).
            assert!(ops::distance_to_boundary(&ours, vertex) >= distance * 0.99 - 2.0);
        }
        let shrunk = offset(&ours, -distance, Join::Round(distance / 400.0));
        assert!(
            area(&shrunk) >= area(&rings) * 0.999,
            "closing must not lose area"
        );
    }
}

#[test]
fn simplification_moves_the_outline_less_than_the_tolerance() {
    let mut random = Random(0x5eed_0000_0000_0004);
    for _ in 0..100 {
        let rings = vec![star(&mut random, [0.0, 0.0], 100_000.0)];
        let simplified = ops::simplify(&rings, 3_000.0, false);
        let changed = area(&ops::xor(&rings, &simplified));
        let perimeter = 2.0 * std::f64::consts::PI * 100_000.0 * 2.0;
        assert!(changed <= perimeter * 3_000.0, "{changed}");
        assert!(simplified.iter().map(Vec::len).sum::<usize>() <= rings[0].len());
    }
}

// ---------------------------------------------------------------------------------------
// Settings of the three owner projects (only the keys stage A reads) and derived constants.

const COMMON: &str = r#""layer_height": "0.28", "min_layer_height": ["0.08"],
    "slice_closing_radius": "0.049", "resolution": "0.03", "slicing_mode": "regular",
    "elefant_foot_compensation": "0.15", "nozzle_diameter": ["0.4"], "line_width": "0.42",
    "support_line_width": "0.42", "outer_wall_line_width": "0.42", "enable_support": "1",
    "support_type": "tree(auto)", "support_style": "default",
    "support_critical_regions_only": "1", "support_remove_small_overhang": "1",
    "tree_support_branch_angle": "45", "tree_support_branch_distance": "5",
    "tree_support_branch_diameter": "2", "tree_support_branch_diameter_angle": "5",
    "support_object_xy_distance": "0.75", "support_on_build_plate_only": "1",
    "support_interface_top_layers": "3", "support_interface_bottom_layers": "3",
    "support_interface_spacing": "0.5", "support_base_pattern": "rectilinear",
    "support_base_pattern_spacing": "2.5", "min_feature_size": "25%""#;

fn project(extra: &str) -> SupportSettings {
    let json = format!("{{{COMMON}, {extra}}}");
    SupportSettings::from_config(&ProjectConfig::from_json(&json).unwrap()).unwrap()
}

fn u1() -> SupportSettings {
    project(
        r#""initial_layer_print_height": "0.25", "support_threshold_angle": "30",
        "support_object_first_layer_gap": "0.75", "support_top_z_distance": "0.24",
        "support_bottom_z_distance": "0.24", "max_bridge_length": "10",
        "tree_support_branch_angle_organic": "40", "tree_support_branch_distance_organic": "1",
        "tree_support_branch_diameter_organic": "2", "tree_support_angle_slow": "25",
        "tree_support_tip_diameter": "0.8", "tree_support_top_rate": "30%",
        "tree_support_wall_count": "0", "elefant_foot_compensation_layers": "1""#,
    )
}

fn p1s() -> SupportSettings {
    project(
        r#""initial_layer_print_height": "0.2", "support_threshold_angle": "40",
        "support_object_first_layer_gap": "1.2", "support_top_z_distance": "0.28",
        "support_bottom_z_distance": "0.28", "tree_support_wall_count": "0",
        "printable_area": ["0x0", "256x0", "256x256", "0x256"],
        "bed_exclude_area": ["0x0", "18x0", "18x28", "0x28"]"#,
    )
}

fn a2l() -> SupportSettings {
    project(
        r#""initial_layer_print_height": "0.2", "support_threshold_angle": "35",
        "support_object_first_layer_gap": "0.75", "support_top_z_distance": "0.24",
        "support_bottom_z_distance": "0.24", "tree_support_wall_count": "-1""#,
    )
}

/// Spec 01 §4 table, both grids: B values (P1S, A2L) in 10 nm units, O/S values (U1) in nm.
#[test]
fn derived_constants_pin_spec_01_section_4() {
    for (settings, flavour) in [(p1s(), Flavour::Bambu0208), (a2l(), Flavour::Bambu0208)] {
        let d = Derived::new(&settings, flavour);
        let values = [
            d.layer_height,
            d.line_width,
            d.branch_radius,
            d.tip_radius,
            d.move_fast,
            d.move_slow,
            d.volume_move_fast,
            d.volume_move_slow,
            d.tip_layers,
            d.foot_radius,
            d.foot_layers,
            d.recommended_min_radius(0),
            d.increase_until_layer,
            d.min_depth_to_model,
            d.xy,
            d.xy_min,
            d.connect_length,
            d.roof_line_distance,
            d.resolution,
            d.extra_outset,
            d.outset_step,
            d.tip_dedup_cell,
            d.branch_distance,
            d.foot_increment_units,
        ];
        assert_eq!(
            values,
            [
                28_000, 41_999, 99_999, 40_000, 27_999, 13_056, 27_997, 13_054, 4, 374_999, 13,
                372_992, 4, 4, 75_000, 20_999, 177_997, 91_998, 2_999, 40_000, 5_249, 4_000,
                499_999, 18_549
            ]
        );
        assert_eq!((d.radius(0, 0.0), d.radius(4, 0.0)), (40_000, 99_999));
        assert_eq!(
            [d.radius(1, 0.0), d.radius(2, 0.0), d.radius(3, 0.0)],
            [54_999, 69_999, 84_999]
        );
        assert!((d.radius_growth - 2_449.68).abs() < 0.01);
        assert_eq!(d.foot_growth, 20_999.5);
        assert!(d.tight_xy && !d.force_tip_to_roof && !d.to_model_allowed);
        assert_eq!((d.z_top_layers, d.z_bottom_layers, d.z_delta), (1, 1, 2));
        assert_eq!(
            (d.roof_layers, d.floor_layers, d.bottom_layers, d.wall_count),
            (3, 3, 3, 1)
        );
        assert!((d.top_gap_mm - 0.28).abs() < 1e-12);
        assert_eq!(d.branch_angle.to_degrees().round(), 45.0);
    }
    let d = Derived::new(&u1(), Flavour::Snapmaker240);
    let values = [
        d.layer_height,
        d.line_width,
        d.branch_radius,
        d.tip_radius,
        d.move_fast,
        d.move_slow,
        d.volume_move_fast,
        d.volume_move_slow,
        d.tip_layers,
        d.foot_radius,
        d.foot_layers,
        d.recommended_min_radius(0),
        d.increase_until_layer,
        d.xy,
        d.xy_min,
        d.connect_length,
        d.roof_line_distance,
        d.resolution,
        d.extra_outset,
        d.outset_step,
        d.tip_dedup_cell,
        d.branch_distance,
        d.foot_increment_units,
    ];
    assert_eq!(
        values,
        [
            280_000, 419_999, 1_000_000, 400_000, 234_947, 130_566, 234_945, 130_564, 4, 3_750_000,
            13, 3_729_993, 4, 750_000, 209_999, 1_779_997, 919_999, 30_000, 400_000, 52_499,
            40_000, 1_000_000, 185_502
        ]
    );
    assert!((d.radius_growth - 24_496.8).abs() < 0.1);
    assert_eq!(d.foot_growth, 209_999.5);
    // Spec 02 §1.2: R(5) = 1.0245 mm, R(89) = 3.082 mm, r_rec(12) = 1.21 mm, g_ft = 0.1855 mm.
    assert_eq!(d.radius(5, 0.0), 1_024_496);
    assert_eq!(d.radius(89, 0.0) / 1_000, 3_082);
    assert_eq!(d.recommended_min_radius(12) / 10_000, 120);
    assert!((d.foot_increment - 185_503.2).abs() < 1.0);
    assert_eq!(d.branch_angle.to_degrees().round(), 40.0);
}

#[test]
fn fill_parameters_pin_spec_01_section_4() {
    let bambu = Derived::new(&p1s(), Flavour::Bambu0208).fill;
    let orca = Derived::new(&u1(), Flavour::Orca242).fill;
    assert!((bambu.flow_spacing - 0.359_911_5).abs() < 1e-6);
    assert!((bambu.base_density - 0.067_148_8).abs() < 1e-6);
    assert!((orca.base_density - 0.264_658_0).abs() < 1e-6);
    assert!((orca.interface_density - 0.418_544_8).abs() < 1e-6);
    assert_eq!(
        (bambu.base_line_spacing, orca.base_line_spacing),
        (535_991, 1_359_911)
    );
    assert_eq!(
        (bambu.roof_line_spacing, orca.roof_line_spacing),
        (85_991, 859_911)
    );
}

/// Tip seed circle (10 µm) and vertical-enforcer disc (0.5 mm) with upstream's chord error of
/// 25 grid units: 15 / 100 vertices on B's grid, 45 / 315 on the nm grid (01 §2.6, 02 §1.3).
#[test]
fn circle_vertex_counts_depend_on_the_grid() {
    let counts = |grid: Grid| {
        (
            Derived::circle_vertices(grid.scaled(0.01), 25.0),
            Derived::circle_vertices(grid.scaled(0.5), 25.0),
        )
    };
    assert_eq!(counts(Grid::TenNanometre), (15, 100));
    assert_eq!(counts(Grid::Nanometre), (45, 315));
}

#[test]
fn every_unsupported_setting_is_named() {
    let cases = [
        ("precise_z_height", "1"),
        ("zaa_enabled", "1"),
        ("raft_layers", "2"),
        ("slicing_mode", "even_odd"),
        ("xy_contour_compensation", "0.1"),
        ("xy_hole_compensation", "0.1"),
        ("enable_circle_compensation", "1"),
        ("make_overhang_printable", "1"),
        ("filament_shrink", "99%"),
        ("skirt_loops", "1"),
        ("detect_thin_wall", "1"),
        ("support_type", "normal(auto)"),
        ("support_style", "tree_hybrid"),
        ("support_on_build_plate_only", "0"),
        ("support_top_z_distance", "0"),
        ("independent_support_layer_height", "1"),
        ("support_base_pattern", "honeycomb"),
        ("support_base_pattern", "lightning"),
        ("support_filament", "2"),
        ("support_interface_filament", "2"),
        ("mixed_filament_gradient_mode", "1"),
        ("dithering_z_step_size", "0.1"),
    ];
    for (key, value) in cases {
        let json = format!("{{{COMMON}, \"initial_layer_print_height\": \"0.25\"}}");
        let mut config = ProjectConfig::from_json(&json).unwrap();
        config.set(key, value);
        let settings = SupportSettings::from_config(&config).unwrap();
        let mesh = mesh(vec![cuboid([0.0, 0.0, 0.0], [10.0, 10.0, 5.0])]);
        let error = run(&mesh, &settings, Flavour::Snapmaker240).err();
        assert_eq!(
            error.as_ref().and_then(SupportError::setting),
            Some(key),
            "{key} = {value}"
        );
    }
    // Fuzzy skin only counts on the first layer; variable layer heights come from ranges.
    let mut settings = u1();
    settings.fuzzy_skin_first_layer = true;
    let error = run(
        &mesh(vec![cuboid([0.0; 3], [5.0, 5.0, 5.0])]),
        &settings,
        Flavour::Orca242,
    )
    .err();
    assert_eq!(
        error.as_ref().and_then(SupportError::setting),
        Some("fuzzy_skin_first_layer")
    );
    let mut settings = u1();
    settings.bottom_z_distance = 0.0;
    let error = run(
        &mesh(vec![cuboid([0.0; 3], [5.0, 5.0, 5.0])]),
        &settings,
        Flavour::Orca242,
    )
    .err();
    assert_eq!(
        error.as_ref().and_then(SupportError::setting),
        Some("support_bottom_z_distance")
    );
    let mut settings = p1s();
    settings.wrapping_detection = true;
    let error = run(
        &mesh(vec![cuboid([0.0; 3], [5.0, 5.0, 5.0])]),
        &settings,
        Flavour::Bambu0208,
    )
    .err();
    assert_eq!(
        error.as_ref().and_then(SupportError::setting),
        Some("enable_wrapping_detection")
    );
    let mut settings = u1();
    settings.layer_ranges = vec![(2.0, 4.0, Some(0.2))];
    let error = run(
        &mesh(vec![cuboid([0.0; 3], [5.0, 5.0, 5.0])]),
        &settings,
        Flavour::Orca242,
    )
    .err();
    assert_eq!(
        error.as_ref().and_then(SupportError::setting),
        Some("layer_height_profile")
    );
}

// ---------------------------------------------------------------------------------------
// Layer grid (spec 01 §1.2.2): the goat height 44.7435 mm.

#[test]
fn goat_height_gives_160_layers_on_every_printer() {
    let height = 44.7435;
    for (settings, last_top, first_slice) in [
        (u1(), 44.77, 0.125),
        (p1s(), 44.72, 0.10),
        (a2l(), 44.72, 0.10),
    ] {
        let grid = LayerGrid::new(&settings, height).unwrap();
        assert_eq!(grid.len(), 160);
        let last = grid.layers[159];
        assert!((last.top - last_top).abs() < 1e-9, "{}", last.top);
        assert!((grid.layers[0].slice_z - first_slice).abs() < 1e-7);
        assert!((grid.layers[1].slice_z - (grid.layers[0].top + 0.14)).abs() < 1e-6);
    }
    // U1 ranges (14.9-18.24 and 33.14-60 at 0.28) collapse into the fixed profile.
    let mut settings = u1();
    settings.layer_ranges = vec![(14.9, 18.24, Some(0.28)), (33.14, 60.0, Some(0.28))];
    assert_eq!(LayerGrid::new(&settings, height).unwrap().len(), 160);
}

// ---------------------------------------------------------------------------------------
// Generated meshes (outward winding, object frame).

/// Axis-aligned box; vertex index = x + 2y + 4z bits; triangles 0 and 1 are the bottom.
fn cuboid(min: [f64; 3], size: [f64; 3]) -> Part {
    let vertices = (0..8)
        .map(|bits| {
            [0, 1, 2].map(|axis| {
                min[axis]
                    + if bits >> axis & 1 == 1 {
                        size[axis]
                    } else {
                        0.0
                    }
            })
        })
        .collect();
    let triangles = vec![
        [0, 2, 3],
        [0, 3, 1],
        [4, 5, 7],
        [4, 7, 6],
        [0, 1, 5],
        [0, 5, 4],
        [2, 6, 7],
        [2, 7, 3],
        [0, 4, 6],
        [0, 6, 2],
        [1, 3, 7],
        [1, 7, 5],
    ];
    Part {
        vertices,
        triangles,
        ..Part::default()
    }
}

fn mesh(parts: Vec<Part>) -> ObjectMesh {
    ObjectMesh::from_plate_parts(parts).unwrap()
}

/// Several shells as one volume: one part's slices are never split by the Orca fast path
/// for parts with disjoint boxes (spec 01 §1.4).
fn shells(parts: Vec<Part>) -> Part {
    let mut merged = Part::default();
    for part in parts {
        let base = merged.vertices.len() as u32;
        merged.vertices.extend(part.vertices);
        merged
            .triangles
            .extend(part.triangles.iter().map(|t| t.map(|v| v + base)));
    }
    merged
}

fn run(
    mesh: &ObjectMesh,
    settings: &SupportSettings,
    flavour: Flavour,
) -> Result<StageA, SupportError> {
    stage_a(&StageAInput {
        mesh,
        settings,
        flavour,
        bed: None,
    })
}

/// Layers whose hand-off holds a typed overhang matching `pick`.
fn layers_with(
    stage: &StageA,
    pick: impl Fn(&goat_packer::support_model::overhangs::OverhangFlags) -> bool,
) -> Vec<usize> {
    (0..stage.overhangs.layers.len())
        .filter(|&layer| {
            stage.overhangs.layers[layer]
                .iter()
                .any(|overhang| pick(&overhang.flags))
        })
        .collect()
}

/// A post with a long arm at the top: the arm's underside is a cantilever in every build.
fn t_bar() -> ObjectMesh {
    mesh(vec![
        cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 8.0]),
        cuboid([0.0, 0.0, 8.0], [20.0, 4.0, 2.0]),
    ])
}

#[test]
fn the_t_bar_arm_is_a_cantilever_in_every_flavour() {
    for (settings, flavour) in [
        (u1(), Flavour::Snapmaker240),
        (u1(), Flavour::Orca250),
        (p1s(), Flavour::Bambu0208),
    ] {
        let stage = run(&t_bar(), &settings, flavour).unwrap();
        let layers = layers_with(&stage, |flags| flags.cantilever);
        assert_eq!(layers.len(), 1, "{flavour:?}: {layers:?}");
        // The first layer sliced above the arm's underside at z = 8.
        let (below, layer) = (
            stage.grid.layers[layers[0] - 1],
            stage.grid.layers[layers[0]],
        );
        assert!(
            below.slice_z < 8.0 && layer.slice_z > 8.0,
            "{flavour:?}: {layer:?}"
        );
        assert!(stage.overhangs.handoff[layers[0]].len() >= 1);
    }
}

/// A block with a 2.8 mm shelf: a plain overhang, too short for a cantilever.
fn shelf() -> ObjectMesh {
    mesh(vec![
        cuboid([0.0, 0.0, 0.0], [20.0, 20.0, 6.0]),
        cuboid([0.0, 0.0, 6.0], [20.0, 22.8, 2.0]),
    ])
}

#[test]
fn critical_only_drops_a_plain_shelf_and_painted_enforcers_bring_it_back() {
    for (settings, flavour) in [(u1(), Flavour::Snapmaker240), (p1s(), Flavour::Bambu0208)] {
        let critical = run(&shelf(), &settings, flavour).unwrap();
        assert!(critical.overhangs.highest_layer().is_none(), "{flavour:?}");
        let mut everything = settings.clone();
        everything.critical_regions_only = false;
        let plain = run(&shelf(), &everything, flavour).unwrap();
        let found = layers_with(&plain, |flags| !flags.cantilever && !flags.sharp_tail);
        assert_eq!(found.len(), 1, "{flavour:?}: {found:?}");
        // Paint the shelf's underside as an enforcer (code "4" on both bottom triangles).
        let mut painted = shelf();
        painted.parts[1].paint_supports =
            BTreeMap::from([(0, "4".to_owned()), (1, "4".to_owned())]);
        let enforced = run(&painted, &settings, flavour).unwrap();
        let layers = enforced
            .overhangs
            .highest_layer()
            .expect("an enforced overhang");
        let area_mm2 = flavour
            .grid()
            .mm2(area(&enforced.overhangs.handoff[layers]));
        assert!(area_mm2 > 20.0, "{flavour:?}: {area_mm2}");
    }
}

/// A 2 mm column hanging under the T-bar's arm: its floating bottom seeds a sharp tail.
#[test]
fn a_hanging_column_is_a_sharp_tail() {
    let hanging = mesh(vec![shells(vec![
        cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 8.0]),
        cuboid([0.0, 0.0, 8.0], [20.0, 4.0, 2.0]),
        cuboid([16.0, 1.0, 5.0], [2.0, 2.0, 3.0]),
    ])]);
    for (settings, flavour) in [(u1(), Flavour::Snapmaker240), (p1s(), Flavour::Bambu0208)] {
        let stage = run(&hanging, &settings, flavour).unwrap();
        let tails = layers_with(&stage, |flags| flags.sharp_tail);
        let bottom = stage
            .grid
            .layers
            .iter()
            .position(|layer| layer.slice_z > 5.0)
            .unwrap();
        assert!(tails.contains(&bottom), "{flavour:?}: {tails:?}");
        // B also calls the floating bottom a cantilever (no vertex near the layer below).
        let floating = layers_with(&stage, |flags| flags.cantilever).contains(&bottom);
        assert_eq!(
            floating,
            flavour.floating_ring_is_cantilever(),
            "{flavour:?}"
        );
        assert!(
            stage.overhangs.stats.tails[bottom + 3] > 0,
            "the tail grows upward"
        );
    }
}

#[test]
fn more_than_a_hundred_overhang_islands_trip_the_guard() {
    let mut parts = vec![cuboid([0.0, 0.0, 0.0], [30.0, 30.0, 3.0])];
    for row in 0..11 {
        for column in 0..11 {
            parts.push(cuboid(
                [column as f64 * 2.5, row as f64 * 2.5, 6.0],
                [1.2, 1.2, 1.0],
            ));
        }
    }
    let error = run(&mesh(vec![shells(parts)]), &u1(), Flavour::Orca242).err();
    assert_eq!(
        error.as_ref().and_then(SupportError::setting),
        Some("overhang_islands")
    );
}

#[test]
fn disjoint_parts_are_refused_by_orca_builds_only() {
    let two = mesh(vec![
        cuboid([0.0; 3], [5.0, 5.0, 5.0]),
        cuboid([10.0, 0.0, 0.0], [5.0, 5.0, 5.0]),
    ]);
    let error = run(&two, &u1(), Flavour::Orca242).err();
    assert_eq!(
        error.as_ref().and_then(SupportError::setting),
        Some("parts")
    );
    assert!(run(&two, &p1s(), Flavour::Bambu0208).is_ok());
}

#[test]
fn support_blockers_are_refused_for_orca_2_5() {
    let mut blocker = cuboid([0.0, 0.0, 6.0], [5.0, 5.0, 2.0]);
    blocker.kind = PartKind::SupportBlocker;
    let object = mesh(vec![cuboid([0.0; 3], [10.0, 10.0, 8.0]), blocker]);
    let error = run(&object, &u1(), Flavour::Orca250).err();
    assert_eq!(
        error.as_ref().and_then(SupportError::setting),
        Some("support_blocker")
    );
    assert!(run(&object, &u1(), Flavour::Orca242).is_ok());
}

// ---------------------------------------------------------------------------------------
// Clearance volumes (spec 02 §2).

#[test]
fn radius_keys_follow_the_ladder() {
    let derived = Derived::new(&u1(), Flavour::Snapmaker240);
    let stage = run(&t_bar(), &u1(), Flavour::Snapmaker240).unwrap();
    let keys = stage.clearance.keys();
    let mm = |value: i64| value as f64 / 1e6;
    let ladder: Vec<f64> = [
        1, 400_000, 400_001, 1_500_001, 2_600_001, 3_900_001, 5_850_001,
    ]
    .iter()
    .map(|&radius| mm(keys.round_up(radius)))
    .collect();
    assert_eq!(ladder, [0.4, 0.4, 1.5, 2.6, 3.9, 5.85, 8.775]);
    assert!(keys.ignorable().is_empty());
    // Regular keys add δ = 0.54 mm; a radius of 0 keys exactly δ (02 §2.2).
    assert_eq!(keys.key(0, false), derived.xy_delta);
    assert_eq!(mm(keys.key(1_000_000, false)), 2.6);
    assert_eq!(mm(keys.key(3_729_993, false)), 5.85);
    assert_eq!(mm(keys.key(1_000_000, true)), 1.5);
}

#[test]
fn collision_grows_the_layer_outline_by_key_plus_xy_min() {
    let stage = run(&t_bar(), &u1(), Flavour::Snapmaker240).unwrap();
    let (clearance, derived) = (&stage.clearance, &stage.derived);
    let key = clearance.keys().key(400_000, true);
    let found = bbox(&clearance.collision_at(key, 5)).unwrap();
    let post = bbox(&stage.sections.outline[5]).unwrap();
    // Collision reaches K + xy_min = 0.4 + 0.21 mm beyond the post (the layer above adds
    // only the reduced ρ₁; mitred corners stay within the box).
    let reach = key + derived.xy_min;
    assert_eq!(
        i64::from(found.max.x) - i64::from(post.max.x),
        reach,
        "{found:?}"
    );
    assert_eq!(
        i64::from(post.min.y) - i64::from(found.min.y),
        reach,
        "{found:?}"
    );
    for kind in [Avoidance::Slow, Avoidance::FastSafe, Avoidance::Fast] {
        let avoid = clearance.avoidance(400_000, 20, kind, true);
        let collision = clearance.collision(400_000, 20, true);
        let outside = ops::difference(&collision, &avoid);
        assert!(
            area(&outside) < 1e6,
            "{kind:?}: avoidance must contain the collision"
        );
    }
    let wall = clearance.wall_restriction(1_000_000, 10, true);
    assert!(area(&wall) > 0.0);
}

#[test]
fn clearance_is_the_same_on_any_thread_order() {
    let stage = run(&t_bar(), &u1(), Flavour::Snapmaker240).unwrap();
    let fresh = Clearance::new(&stage.sections.outline, Rings::new(), &stage.derived);
    let shared = &stage.clearance;
    let requests: Vec<(i64, usize)> = (0..6)
        .flat_map(|r| (1..30).map(move |l| (400_000 + r * 500_000, l)))
        .collect();
    std::thread::scope(|scope| {
        for chunk in requests.chunks(29) {
            scope.spawn(move || {
                for &(radius, layer) in chunk.iter().rev() {
                    shared.avoidance(radius, layer, Avoidance::Fast, false);
                }
            });
        }
    });
    for &(radius, layer) in &requests {
        assert_eq!(
            *shared.avoidance(radius, layer, Avoidance::Fast, false),
            *fresh.avoidance(radius, layer, Avoidance::Fast, false)
        );
    }
}

#[test]
fn bambu_bed_obstacle_is_a_ring_in_four_pieces() {
    let placement = BedPlacement {
        printable_area: vec![[0.0, 0.0], [256.0, 0.0], [256.0, 256.0], [0.0, 256.0]],
        exclude_area: vec![[0.0, 0.0], [18.0, 0.0], [18.0, 28.0], [0.0, 28.0]],
        object_origin: [128.0, 128.0],
    };
    let ring = bed_obstacle(&placement, Grid::TenNanometre);
    assert_eq!(ring.len(), 4);
    let units = |mm: f64| Grid::TenNanometre.coordinate(mm);
    // The bed interior is free, the cutter corner and the outside are obstacle.
    assert!(!contains(&ring, IntPoint::new(units(10.0), units(10.0))));
    assert!(contains(&ring, IntPoint::new(units(-120.0), units(-120.0))));
    assert!(contains(&ring, IntPoint::new(units(130.0), units(60.0))));
    let stage = run(&t_bar(), &p1s(), Flavour::Bambu0208).unwrap();
    let with_bed = stage_a(&StageAInput {
        mesh: &t_bar(),
        settings: &p1s(),
        flavour: Flavour::Bambu0208,
        bed: Some(&placement),
    })
    .unwrap();
    assert!(stage.clearance.border().is_empty());
    assert_eq!(with_bed.clearance.border().len(), 4);
}

// ---------------------------------------------------------------------------------------
// Determinism (design §5.2 rules).

#[test]
fn stage_a_is_bit_identical_across_runs() {
    let object = mesh(vec![shells(vec![
        cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 8.0]),
        cuboid([0.0, 0.0, 8.0], [20.0, 4.0, 2.0]),
        cuboid([16.0, 1.0, 5.0], [2.0, 2.0, 3.0]),
    ])]);
    for (settings, flavour) in [(u1(), Flavour::Snapmaker240), (p1s(), Flavour::Bambu0208)] {
        let first = run(&object, &settings, flavour).unwrap();
        let second = std::thread::spawn({
            let (object, settings) = (object.clone(), settings.clone());
            move || run(&object, &settings, flavour).unwrap().overhangs
        })
        .join()
        .unwrap();
        assert_eq!(first.overhangs, second);
        assert_eq!(
            first.sections,
            run(&object, &settings, flavour).unwrap().sections
        );
    }
}

//! Mesh kinds of work package S1 on generated meshes: exact sections, slab body bands,
//! silhouette, hulls and boxes, rotation classes, determinism.

use goat_packer::embedded_mesh::{ObjectMesh, Part, PartKind};
use goat_packer::mesh_section::{
    BodyBands, SectionOptions, body_bands, body_print, convex_hull, object_section, part_section,
    raw_section_loops, slab_polygon, wall_bulge_mm,
};
use goat_packer::{Point, Rect, point_in_footprint, polygon_area};

// ---------------------------------------------------------------------------------------
// Generated meshes (outward winding, plate coordinates).

/// Axis-aligned box; vertex index = x + 2y + 4z bits.
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
        [0, 3, 1], // bottom
        [4, 5, 7],
        [4, 7, 6], // top
        [0, 1, 5],
        [0, 5, 4], // y = min
        [2, 6, 7],
        [2, 7, 3], // y = max
        [0, 4, 6],
        [0, 6, 2], // x = min
        [1, 3, 7],
        [1, 7, 5], // x = max
    ];
    Part {
        vertices,
        triangles,
        ..Part::default()
    }
}

/// A vertical prism over a counter-clockwise polygon that is star-shaped from `fan`.
fn prism(polygon: &[[f64; 2]], z0: f64, z1: f64, fan: usize) -> Part {
    let n = polygon.len() as u32;
    let mut vertices: Vec<[f64; 3]> = polygon.iter().map(|p| [p[0], p[1], z0]).collect();
    vertices.extend(polygon.iter().map(|p| [p[0], p[1], z1]));
    let mut triangles = Vec::new();
    for step in 1..n - 1 {
        let (a, b, c) = (
            fan as u32,
            (fan as u32 + step) % n,
            (fan as u32 + step + 1) % n,
        );
        triangles.push([a, c, b]);
        triangles.push([n + a, n + b, n + c]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        triangles.push([i, j, n + j]);
        triangles.push([i, n + j, n + i]);
    }
    Part {
        vertices,
        triangles,
        ..Part::default()
    }
}

/// Square pyramid: base `size` at z = 0 centred on the origin, apex at `height`.
fn pyramid(size: f64, height: f64) -> Part {
    let h = size / 2.0;
    Part {
        vertices: vec![
            [-h, -h, 0.0],
            [h, -h, 0.0],
            [h, h, 0.0],
            [-h, h, 0.0],
            [0.0, 0.0, height],
        ],
        triangles: vec![
            [0, 2, 1],
            [0, 3, 2],
            [0, 1, 4],
            [1, 2, 4],
            [2, 3, 4],
            [3, 0, 4],
        ],
        ..Part::default()
    }
}

/// Square pyramid standing on its apex at the origin, base `size` at `height`.
fn inverted_pyramid(size: f64, height: f64) -> Part {
    let h = size / 2.0;
    Part {
        vertices: vec![
            [-h, -h, height],
            [h, -h, height],
            [h, h, height],
            [-h, h, height],
            [0.0, 0.0, 0.0],
        ],
        triangles: vec![
            [0, 1, 2],
            [0, 2, 3],
            [0, 4, 1],
            [1, 4, 2],
            [2, 4, 3],
            [3, 4, 0],
        ],
        ..Part::default()
    }
}

const L_SHAPE: [[f64; 2]; 6] = [
    [0., 0.],
    [10., 0.],
    [10., 5.],
    [5., 5.],
    [5., 10.],
    [0., 10.],
];

fn object(parts: Vec<Part>) -> ObjectMesh {
    ObjectMesh::from_plate_parts(parts).unwrap()
}

fn area(loops: &[Vec<Point>]) -> f64 {
    loops.iter().map(|ring| polygon_area(ring)).sum()
}

fn bounds(loops: &[Vec<Point>]) -> Rect {
    Rect::of_points(loops.iter().flatten())
}

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

// ---------------------------------------------------------------------------------------
// Sections.

#[test]
fn cube_section_catches_top_faces_and_misses_bottom_faces() {
    let mesh = object(vec![cuboid([0.0; 3], [10.0; 3])]);
    let part = &mesh.parts[0];
    let middle = raw_section_loops(part, 5.0);
    assert_eq!(middle.len(), 1);
    assert!(
        near(polygon_area(&middle[0]), 100.0, 1e-6),
        "outer loop runs counter-clockwise"
    );
    // The plane at the top face is caught through the side triangles' top edges.
    assert!(near(
        area(&part_section(part, 10.0, &SectionOptions::EXACT)),
        100.0,
        1e-6
    ));
    // The bottom face is missed: every triangle touching z = 0 lies at or above the plane.
    assert!(part_section(part, 0.0, &SectionOptions::EXACT).is_empty());
    assert!(part_section(part, 10.5, &SectionOptions::EXACT).is_empty());
}

#[test]
fn l_prism_section_is_the_l() {
    let mesh = object(vec![prism(&L_SHAPE, 0.0, 4.0, 3)]);
    let section = object_section(&mesh, 2.0, &SectionOptions::slicer(0.049));
    assert_eq!(section.len(), 1);
    assert!(near(area(&section), 75.0, 1e-6));
    // The reflex corner of the L survives closing (it is not a gap).
    assert!(
        !point_in_footprint(&section, 2.6, 2.6),
        "frame-centred notch stays empty"
    );
}

#[test]
fn closing_bridges_gaps_narrower_than_twice_the_radius() {
    let narrow = object(vec![
        cuboid([0.0, 0.0, 0.0], [5.0, 5.0, 2.0]),
        cuboid([5.05, 0.0, 0.0], [5.0, 5.0, 2.0]),
    ]);
    let wide = object(vec![
        cuboid([0.0, 0.0, 0.0], [5.0, 5.0, 2.0]),
        cuboid([5.2, 0.0, 0.0], [5.0, 5.0, 2.0]),
    ]);
    let options = SectionOptions::slicer(0.049);
    let bridged: Vec<_> = narrow
        .parts
        .iter()
        .map(|p| part_section(p, 1.0, &options))
        .collect();
    assert_eq!(bridged[0].len(), 1, "each part is closed on its own");
    let joined = object_section(&narrow, 1.0, &options);
    assert_eq!(
        joined.len(),
        2,
        "parts are closed per volume, then united (spec 01 §1.3-1.4)"
    );
    let one_part = object(vec![prism(
        &[
            [0., 0.],
            [10., 0.],
            [10., 5.],
            [5.05, 5.],
            [5.05, 0.5],
            [5.0, 0.5],
            [5.0, 5.0],
            [0., 5.],
        ],
        0.0,
        2.0,
        0,
    )]);
    let notch = part_section(&one_part.parts[0], 1.0, &options);
    assert!(
        near(area(&notch), 50.0, 1e-3),
        "a 0.05 mm notch closes: area {}",
        area(&notch)
    );
    assert_eq!(object_section(&wide, 1.0, &options).len(), 2);
}

#[test]
fn open_chains_close_across_small_gaps_and_soups_chain_by_coordinates() {
    let mut torn = cuboid([0.0; 3], [2.0, 2.0, 2.0]);
    torn.triangles.remove(10); // one triangle of the x = max side
    let mesh = object(vec![torn]);
    let section = part_section(&mesh.parts[0], 1.0, &SectionOptions::EXACT);
    assert_eq!(section.len(), 1, "the 1 mm gap is closed");
    assert!(near(area(&section), 4.0, 1e-6));

    let indexed = cuboid([0.0; 3], [10.0; 3]);
    let mut soup = Part::default();
    for triangle in &indexed.triangles {
        let base = soup.vertices.len() as u32;
        soup.vertices
            .extend(triangle.map(|v| indexed.vertices[v as usize]));
        soup.triangles.push([base, base + 1, base + 2]);
    }
    let soup_mesh = object(vec![soup]);
    let loops = raw_section_loops(&soup_mesh.parts[0], 3.0);
    assert_eq!(loops.len(), 1);
    assert!(near(polygon_area(&loops[0]), 100.0, 1e-6));
}

#[test]
fn negative_part_cuts_the_section_but_never_the_body() {
    let mut hole = cuboid([3.0, 3.0, -1.0], [4.0, 4.0, 12.0]);
    hole.kind = PartKind::Negative;
    let mut modifier = cuboid([-5.0, -5.0, 0.0], [20.0, 20.0, 10.0]);
    modifier.kind = PartKind::Modifier;
    let mesh = object(vec![cuboid([0.0; 3], [10.0; 3]), hole, modifier]);
    assert_eq!(
        mesh.frame_origin,
        [5.0, 5.0, 0.0],
        "only normal parts set the frame"
    );
    let section = object_section(&mesh, 5.0, &SectionOptions::EXACT);
    assert!(near(area(&section), 100.0 - 16.0, 1e-6));
    let bands = body_bands(&mesh, &BodyBands::mesh_only());
    assert!(
        bands.iter().all(|band| near(area(band), 100.0, 1e-6)),
        "bodies stay conservative"
    );
}

// ---------------------------------------------------------------------------------------
// Bands.

#[test]
fn pyramid_bands_are_the_widest_section_of_each_slab() {
    let mesh = object(vec![pyramid(20.0, 10.0)]);
    let exact = body_bands(&mesh, &BodyBands::mesh_only());
    assert_eq!(exact.len(), 5);
    for (band, loops) in exact.iter().enumerate() {
        let width = 20.0 * (1.0 - 2.0 * band as f64 / 10.0);
        assert!(
            near(area(loops), width * width, 1e-4),
            "band {band}: {}",
            area(loops)
        );
    }
    let printed = BodyBands::printed(0.28);
    let padded = body_bands(&mesh, &printed);
    // Slab 5 starts at 9.86: a top layer ending above the mesh top can still reach band 5.
    assert_eq!(padded.len(), 6);
    let width = |z: f64| 20.0 * (1.0 - z / 10.0) + 2.0 * printed.bulge_mm;
    let band_bounds = bounds(&padded[2]);
    assert!(near(
        band_bounds.max_x - band_bounds.min_x,
        width(4.0 - 0.14),
        1e-3
    ));
}

#[test]
fn padded_bands_contain_the_mid_layer_section_of_every_layer_reaching_in() {
    // Upside down, every section is wider than the ones below it.
    let mesh = object(vec![inverted_pyramid(20.0, 10.0)]);
    let bands = body_bands(
        &mesh,
        &BodyBands {
            bulge_mm: 0.0,
            ..BodyBands::printed(0.28)
        },
    );
    let exact = body_bands(&mesh, &BodyBands::mesh_only());
    // Layer (1.92, 2.20] reaches into band 0 and prints the section at 2.06 (2.06 wide).
    let section = object_section(&mesh, 2.06, &SectionOptions::EXACT);
    let edge = bounds(&section).max_x - 1e-6;
    assert!(point_in_footprint(&bands[0], edge, 0.0));
    assert!(
        !point_in_footprint(&exact[0], edge, 0.0),
        "the unpadded slab misses it"
    );
}

#[test]
fn band_count_follows_the_slabs() {
    let mesh = object(vec![cuboid([0.0; 3], [4.0, 4.0, 5.0])]);
    assert_eq!(body_bands(&mesh, &BodyBands::mesh_only()).len(), 3);
    let tall = object(vec![cuboid([0.0; 3], [4.0, 4.0, 6.0])]);
    assert_eq!(
        body_bands(&tall, &BodyBands::mesh_only()).len(),
        3,
        "z = 6 starts no band"
    );
    assert_eq!(
        body_bands(&tall, &BodyBands::printed(0.28)).len(),
        4,
        "slab 3 starts at 5.86"
    );
}

#[test]
fn bulge_grows_every_band_by_the_bead() {
    assert!(near(wall_bulge_mm(0.28), 0.0300443, 1e-6));
    let mesh = object(vec![cuboid([0.0; 3], [10.0; 3])]);
    let bands = body_bands(&mesh, &BodyBands::printed(0.28));
    let grown = 10.0 + 2.0 * wall_bulge_mm(0.28);
    for band in &bands {
        let rect = bounds(band);
        assert!(near(rect.max_x - rect.min_x, grown, 2e-6));
        assert!(
            near(area(band), grown * grown, 1e-4),
            "mitred corners stay square"
        );
    }
}

#[test]
fn slab_polygon_clips_a_triangle_between_two_heights() {
    let triangle = [[0.0, 0.0, 0.0], [10.0, 0.0, 10.0], [0.0, 10.0, 10.0]];
    let piece = slab_polygon(&triangle, 2.0, 4.0);
    let ring: Vec<Point> = piece
        .iter()
        .map(|p| Point { x: p[0], y: p[1] })
        .chain([Point {
            x: piece[0][0],
            y: piece[0][1],
        }])
        .collect();
    assert_eq!(piece.len(), 4);
    assert!(near(polygon_area(&ring), 0.5 * (16.0 - 4.0), 1e-9));
    assert!(slab_polygon(&triangle, 11.0, 12.0).is_empty());
}

// ---------------------------------------------------------------------------------------
// Parts, hulls, boxes, rotation, determinism.

#[test]
fn two_parts_keep_their_own_hulls_and_boxes() {
    let mesh = object(vec![
        cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]),
        prism(&[[10.0, 0.0], [14.0, 2.0], [10.0, 4.0]], 0.0, 5.0, 0),
    ]);
    assert_eq!(mesh.frame_origin, [7.0, 2.0, 0.0]);
    let print = body_print(&mesh, 0.0, &BodyBands::mesh_only());
    assert_eq!(print.part_hulls.len(), 2);
    assert_eq!(print.part_boxes.len(), 2);
    assert_eq!(print.part_boxes[0], Rect::new(-7.0, -2.0, -3.0, 2.0));
    assert_eq!(print.part_boxes[1], Rect::new(3.0, -2.0, 7.0, 2.0));
    assert!(near(polygon_area(&print.part_hulls[1]), 8.0, 1e-9));
    assert_eq!(print.silhouette.len(), 2);
    assert!(near(area(&print.silhouette), 16.0 + 8.0, 1e-6));
    assert_eq!(print.body.len(), 3);
    assert!(
        near(area(&print.body[2]), 8.0, 1e-6),
        "band 2 only holds the taller prism"
    );
}

#[test]
fn convex_hull_drops_interior_and_collinear_points() {
    let points: Vec<Point> = [
        [0., 0.],
        [2., 0.],
        [4., 0.],
        [4., 4.],
        [0., 4.],
        [1., 1.],
        [2., 3.],
    ]
    .iter()
    .map(|p| Point { x: p[0], y: p[1] })
    .collect();
    let hull = convex_hull(&points).unwrap();
    assert_eq!(hull.len(), 4);
    let mut closed = hull.clone();
    closed.push(hull[0]);
    assert!(near(polygon_area(&closed), 16.0, 1e-12));
}

#[test]
fn rotated_print_is_the_mesh_turned_about_the_frame_centre() {
    let mesh = object(vec![prism(&L_SHAPE, 0.0, 3.0, 3)]);
    let quarter = body_print(&mesh, 90.0, &BodyBands::mesh_only());
    assert_eq!(quarter.rotation_deg, 90.0);
    // The frame centre is the L's bbox centre (5, 5); a quarter turn maps the notch at
    // frame (2.5, 2.5) to (-2.5, 2.5).
    assert!(!point_in_footprint(&quarter.silhouette, -2.5, 2.5));
    assert!(point_in_footprint(&quarter.silhouette, 2.5, 2.5));
    let full_turn = body_print(&mesh, 360.0, &BodyBands::mesh_only());
    let unturned = body_print(&mesh, 0.0, &BodyBands::mesh_only());
    assert_eq!(
        full_turn.silhouette, unturned.silhouette,
        "multiples of 90° are exact"
    );
}

#[test]
fn body_print_is_deterministic() {
    let mesh = object(vec![prism(&L_SHAPE, 0.0, 7.0, 3), pyramid(6.0, 9.0)]);
    let bands = BodyBands::printed(0.28);
    let first = serde_json::to_string(&body_print(&mesh, 37.5, &bands)).unwrap();
    for _ in 0..3 {
        assert_eq!(
            serde_json::to_string(&body_print(&mesh, 37.5, &bands)).unwrap(),
            first
        );
    }
}

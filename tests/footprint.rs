use goat_packer::{
    Footprint, Point, Rect, extract_footprint, first_layer_footprint, loops_touch_rect,
    patch_overshoot_mm, patch_within_silhouette, polygon_area,
};
use std::{fs::File, io::Write};

fn write_binary_stl(path: &std::path::Path, triangles: &[[[f32; 3]; 3]]) {
    let mut f = File::create(path).unwrap();
    f.write_all(&[0u8; 80]).unwrap();
    f.write_all(&(triangles.len() as u32).to_le_bytes())
        .unwrap();
    for tri in triangles {
        f.write_all(&[0u8; 12]).unwrap();
        for vertex in tri {
            for value in vertex {
                f.write_all(&value.to_le_bytes()).unwrap();
            }
        }
        f.write_all(&[0u8; 2]).unwrap();
    }
}

#[test]
fn extracts_and_centers_flat_bottom_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("box.stl");
    let tris = [
        [[10., 20., 0.], [14., 20., 0.], [14., 22., 0.]],
        [[10., 20., 0.], [14., 22., 0.], [10., 22., 0.]],
        [[10., 20., 2.], [14., 20., 2.], [14., 22., 2.]],
    ];
    write_binary_stl(&path, &tris);

    let fp = extract_footprint(&path).unwrap();

    assert_eq!(fp.triangle_count, 3);
    assert_eq!(fp.loops.len(), 1);
    assert!((fp.width - 4.0).abs() < 1e-6);
    assert!((fp.height - 2.0).abs() < 1e-6);
    assert!(fp.loops[0].contains(&Point { x: -2.0, y: -1.0 }));
    assert!(fp.loops[0].contains(&Point { x: 2.0, y: 1.0 }));
}

/// A lying model whose head overhangs the resting strip: the footprint must be the whole
/// top-down silhouette, or packed neighbours would collide with the head even though every
/// sole clears every sole.
#[test]
fn overhanging_upper_geometry_widens_the_footprint_beyond_the_resting_strip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("overhang.stl");
    // Narrow base at z=0: a 2 x 10 strip.
    let mut tris: Vec<[[f32; 3]; 3]> = square([
        [10., 10., 0.],
        [12., 10., 0.],
        [12., 20., 0.],
        [10., 20., 0.],
    ])
    .to_vec();
    // Wide head at z=8 leaning past the strip on both sides: 10 x 2 slab.
    tris.extend_from_slice(&square([
        [7., 14., 8.],
        [17., 14., 8.],
        [17., 16., 8.],
        [7., 16., 8.],
    ]));
    write_binary_stl(&path, &tris);

    let fp = extract_footprint(&path).unwrap();

    // The silhouette is the union: 2x10 strip + 10x2 slab - 2x2 where the head sits over the
    // strip = 36 mm2; a resting-strip-only footprint would have reported 20.
    let area: f64 = fp.loops.iter().map(|loop_| polygon_area(loop_).abs()).sum();
    assert!((area - 36.0).abs() < 1e-6, "area {area}");
    assert!((fp.width - 10.0).abs() < 1e-6);
}

#[test]
fn goat4_real_footprint_has_expected_dimensions_and_closed_loops() {
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/goat4_v2.stl"
    ));
    let fp = extract_footprint(path).unwrap();
    assert_eq!(fp.triangle_count, 15_624);
    assert!((fp.width - 57.950_335).abs() < 0.001);
    assert!((fp.height - 81.379_105).abs() < 0.001);
    assert_eq!(fp.loops.len(), 2, "outer silhouette plus keyring hole");
    assert!(fp.loops.iter().all(|loop_| loop_.len() >= 4));
    // The top-down silhouette (horns and muzzle reach past the flat sole's 1683.274 mm2).
    let even_odd_area = polygon_area(&fp.loops[0]).abs()
        - fp.loops[1..]
            .iter()
            .map(|points| polygon_area(points).abs())
            .sum::<f64>();
    assert!((even_odd_area - 1_683.920_295).abs() < 0.01);
}

/// Two bottom patches that touch only at a corner (pinch point). The edge-parity walker used to
/// report "bottom boundary is non-manifold: degree 4" here; the union must accept it.
#[test]
fn corner_touching_bottom_patches_yield_both_loops() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pinch.stl");
    let mut tris: Vec<[[f32; 3]; 3]> =
        square([[0., 0., 0.], [10., 0., 0.], [10., 10., 0.], [0., 10., 0.]]).to_vec();
    tris.extend_from_slice(&square([
        [10., 10., 0.],
        [20., 10., 0.],
        [20., 20., 0.],
        [10., 20., 0.],
    ]));
    write_binary_stl(&path, &tris);

    let fp = extract_footprint(&path).unwrap();

    // Whether the union keeps one self-touching outline or two separate polygons is up to
    // the overlay library; even-odd rasterization is correct either way.
    let area: f64 = fp.loops.iter().map(|loop_| polygon_area(loop_).abs()).sum();
    assert!((area - 200.0).abs() < 1e-6, "area {area}");
}

/// A coplanar triangle whose edge lies across the interior of another bottom edge (T-junction),
/// as produced by sloppy flattening of a rounded belly: the shared edge ends up in three flat
/// triangles and parity counting breaks. Union must still produce the covered outline.
#[test]
fn t_junction_and_overlapping_coplanar_triangles_still_yield_outline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tjunction.stl");
    let mut tris: Vec<[[f32; 3]; 3]> =
        square([[0., 0., 0.], [10., 0., 0.], [10., 10., 0.], [0., 10., 0.]]).to_vec();
    // Overlapping patch: one edge collinear with the square's bottom edge (T-junction), the
    // rest sticking out below it.
    tris.push([[0., 0., 0.], [4., 0., 0.], [2., -3., 0.]]);
    write_binary_stl(&path, &tris);

    let fp = extract_footprint(&path).unwrap();

    let area: f64 = fp.loops.iter().map(|loop_| polygon_area(loop_).abs()).sum();
    assert!((area - 106.0).abs() < 1e-6, "area {area}");
}

/// Mixed winding orders among coplanar bottom triangles must not cancel area in the union.
#[test]
fn mixed_winding_bottom_triangles_keep_full_area() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("winding.stl");
    let mut tris: Vec<[[f32; 3]; 3]> =
        square([[0., 0., 0.], [10., 0., 0.], [10., 10., 0.], [0., 10., 0.]]).to_vec();
    // Second square, triangle winding flipped relative to the first square's.
    let mut flipped = square([[10., 0., 0.], [20., 0., 0.], [20., 10., 0.], [10., 10., 0.]]);
    for tri in &mut flipped {
        tri.swap(0, 2);
    }
    tris.extend_from_slice(&flipped);
    write_binary_stl(&path, &tris);

    let fp = extract_footprint(&path).unwrap();

    let area: f64 = fp.loops.iter().map(|loop_| polygon_area(loop_).abs()).sum();
    assert!((area - 200.0).abs() < 1e-6, "area {area}");
}

/// Micron-scale polygons where a rounded belly grazes the bed are triangulation noise; only the
/// printable patch may survive as a loop.
#[test]
fn micron_scale_union_artifacts_are_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("noise.stl");
    let mut tris: Vec<[[f32; 3]; 3]> =
        square([[0., 0., 0.], [10., 0., 0.], [10., 10., 0.], [0., 10., 0.]]).to_vec();
    // Detached 50 x 50 um speck (well under the 0.01 mm2 loop floor).
    tris.extend_from_slice(&square([
        [20., 0., 0.],
        [20.05, 0., 0.],
        [20.05, 0.05, 0.],
        [20., 0.05, 0.],
    ]));
    write_binary_stl(&path, &tris);

    let fp = extract_footprint(&path).unwrap();

    assert_eq!(fp.loops.len(), 1, "speck dropped, square kept");
    assert!((polygon_area(&fp.loops[0]).abs() - 100.0).abs() < 1e-6);
}

fn square([a, b, c, d]: [[f32; 3]; 4]) -> [[[f32; 3]; 3]; 2] {
    [[a, b, c], [a, c, d]]
}

/// The probe slice's first layer is the honest contact patch: body walls plus supports and
/// brim. The prime tower's own layer-1 geometry (TYPE blocks mentioning "tower") stays out,
/// and so does an un-retraction after a long travel — it spends almost no filament and would
/// otherwise draw a phantom strip across the plate.
#[test]
fn first_layer_patch_unions_body_and_supports_but_not_tower_or_unretract() {
    let gcode = b"
;LAYER_CHANGE
;TYPE:Prime tower
;WIDTH:0.5
G1 X40.000 Y200.000 F30000
G1 X40.500 Y200.500 E0.4175
G1 X49.500 Y200.500 E0.8119
;TYPE:Support
;WIDTH:0.5
G1 X200.000 Y200.000 F30000
; un-retraction smeared over the travel: E is 100x too small for the distance
G1 X210.000 Y100.000 E0.02
G1 X200.000 Y100.000 F30000
G1 X200.500 Y100.500 E0.2274
G1 X205.500 Y100.500 E2.2740
G1 X205.500 Y104.500 E1.8192
G1 X200.500 Y104.500 E2.2740
G1 X200.500 Y100.500 E1.8192
;TYPE:Outer wall
;WIDTH:0.5
G1 X210.000 Y110.000 F30000
G1 X210.250 Y110.250 E0.1137
G1 X220.250 Y110.250 E4.5480
G1 X220.250 Y120.250 E4.5480
G1 X210.250 Y120.250 E4.5480
G1 X210.250 Y110.250 E4.5480
;LAYER_CHANGE
;TYPE:Outer wall
G1 X210.250 Y110.250 E4.5480
";
    let patch =
        first_layer_footprint(gcode, Some((215.25, 115.25))).expect("layer-1 extrusions exist");

    // Body bbox center is (215.25, 115.25); the patch's loops are centered on it.
    let area: f64 = patch.loops.iter().map(|l| polygon_area(l).abs()).sum();
    // Body square 10x10 + support strip 5x4, both plus their line width edges (~0.5); only
    // exterior rings count, so the walls' hollow middle is filled solid.
    assert!(area > 100.0 && area < 145.0, "area {area}");
    // Support sits 4.75 mm left of the body's left wall: the patch must reach past the body
    // on that side but not anywhere near the tower at x=40..50.
    let min_x = patch
        .loops
        .iter()
        .flatten()
        .map(|p| p.x)
        .fold(f64::INFINITY, f64::min);
    let max_x = patch
        .loops
        .iter()
        .flatten()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max);
    // Body spans -5..+5 around the center; the support reaches ~-15.
    assert!((min_x + 15.0).abs() < 1.5, "min_x {min_x}");
    assert!((max_x - 5.3).abs() < 1.5, "max_x {max_x}");
    // The patch bbox spans the support's far edge to the body's near edge: ~20 mm wide.
    assert!(
        patch.width > 19.0 && patch.width < 21.5,
        "width {}",
        patch.width
    );
    // The support block sits below-left of the body, so the patch is ~20 mm tall too.
    assert!(
        patch.height > 19.0 && patch.height < 21.5,
        "height {}",
        patch.height
    );
}

#[test]
fn first_layer_without_object_extrusions_yields_none() {
    // Only tower extrusions in layer 1 and nothing else: no patch to pack against.
    let gcode = b"
;LAYER_CHANGE
;TYPE:Prime tower
G1 X40.500 Y200.500 E0.4175
G1 X49.500 Y200.500 E0.8119
;LAYER_CHANGE
G1 X41.000 Y201.000 E0.4175
";
    assert!(first_layer_footprint(gcode, None).is_none());
}

/// Arachne thins real wall lines far below the nominal flow: on a real P1S goat the long leg
/// tips print at 0.001-0.004 filament-mm per mm — an order of magnitude under a normal 0.5 x
/// 0.2 wall (~0.04) — and those are exactly the segments that stick out past the mesh
/// silhouette. A filter calibrated only against un-retraction smears (~0.0002/mm) dropped
/// them, so the parsed patch under-measured the true printed outline and the packer reserved
/// only the mesh silhouette against keepouts: packed goats printed their leg tips straight
/// through the purge line. The cut must sit between the two measured classes.
#[test]
fn first_layer_patch_keeps_arachne_thin_tip_lines() {
    let gcode = b"
;LAYER_CHANGE
;TYPE:Outer wall
;WIDTH:0.5
G1 X60.000 Y60.000 F30000
G1 X60.250 Y60.250 E0.1137
G1 X70.250 Y60.250 E4.5480
G1 X70.250 Y70.250 E4.5480
G1 X60.250 Y70.250 E4.5480
G1 X60.250 Y60.250 E4.5480
; thin leg tip, 8.1 mm long at ~0.0022 filament-mm per mm - real print, must survive
G1 X75.000 Y70.000 F30000
G1 X77.000 Y62.000 E0.0182
";
    let patch =
        first_layer_footprint(gcode, Some((65.25, 65.25))).expect("layer-1 extrusions exist");

    let max_x = patch
        .loops
        .iter()
        .flatten()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max);
    // The spike's far end sits 11.75 mm right of the center; its 0.25 mm strip edge reaches
    // ~12.0. Without the spike the patch stops at +5.25.
    assert!(
        max_x > 11.0,
        "thin tip line missing from patch: max_x {max_x}"
    );
}

/// Bambu Studio's `enable_arc_fitting` draws whole walls as G2/G3 sweeps with an I/J centre.
/// A parser that reads only G0/G1 loses the ink outright: a pure-arc first layer parsed to
/// `None`, and after a run of arcs the tracked position stayed at the last G1, so the next
/// G1 inked a phantom chord across whatever the arcs went around. Arcs must be followed and
/// inked, or arc-fitted probes under-measure the print and misbranch the packer.
#[test]
fn first_layer_patch_flattens_arc_only_walls() {
    // A circle of radius 5 around (5, 0), drawn as two G2 semicircles.
    let gcode = b"
;LAYER_CHANGE
;TYPE:Outer wall
;WIDTH:0.5
G1 X0.000 Y0.000 F30000
G2 X10.000 Y0.000 I5.000 J0.000 E0.9
G2 X0.000 Y0.000 I-5.000 J0.000 E0.9
;LAYER_CHANGE
G1 X1.000 Y1.000 E0.9
";
    let patch = first_layer_footprint(gcode, Some((5.0, 0.0))).expect("arc walls are ink");
    // Circle d=10 plus the 0.5 line width; both extents are the diameter.
    assert!(
        patch.width > 9.5 && patch.width < 12.0,
        "width {}",
        patch.width
    );
    assert!(
        patch.height > 9.5 && patch.height < 12.0,
        "height {}",
        patch.height
    );
}

#[test]
fn first_layer_patch_does_not_chord_across_arc_runs() {
    // One CW semicircle bulging up to (5, 5), then a short line at its right end. From the
    // stale pre-arc position (0, 0) the trailing G1 would draw a chord under the bulge and
    // the patch would lose the whole top half of the print.
    let gcode = b"
;LAYER_CHANGE
;TYPE:Outer wall
;WIDTH:0.5
G1 X0.000 Y0.000 F30000
G2 X10.000 Y0.000 I5.000 J0.000 E0.9
G1 X10.000 Y-1.000 E0.1
;LAYER_CHANGE
G1 X1.000 Y1.000 E0.9
";
    let patch = first_layer_footprint(gcode, Some((5.0, 2.0))).expect("arc wall is ink");
    let max_y = patch
        .loops
        .iter()
        .flatten()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max);
    // The bulge apex sits 3 mm above the model center (5, 2); a chord-cut patch tops out
    // near -1.5.
    assert!(max_y > 2.5, "arc bulge missing from patch: max_y {max_y}");
}

/// `patch_overshoot_mm` measures exactly the spike standoff the bed/exclusion checks need:
/// a patch whose tip reaches 3 mm past the silhouette reports ~3, a contained patch 0.
#[test]
fn patch_overshoot_measures_thin_tip_standoff() {
    // 20 x 20 silhouette centered on the origin.
    let silhouette = loops_footprint(vec![rect_loop(-10.0, -10.0, 10.0, 10.0)]);
    // Same square plus a 3 mm spike off the right edge (x 10 -> 13).
    let spiked = loops_footprint(vec![rect_loop(-10.0, -10.0, 13.0, 10.0)]);

    let contained = loops_footprint(vec![rect_loop(-10.0, -10.0, 10.0, 10.0)]);
    assert_eq!(patch_overshoot_mm(&contained, &silhouette, 0.25), 0.0);

    let overshoot = patch_overshoot_mm(&spiked, &silhouette, 0.25);
    assert!(
        (overshoot - 3.0).abs() <= 0.5,
        "overshoot {overshoot} should track the 3 mm spike"
    );
}

fn rect_loop(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<Point> {
    vec![
        Point { x: min_x, y: min_y },
        Point { x: max_x, y: min_y },
        Point { x: max_x, y: max_y },
        Point { x: min_x, y: max_y },
        Point { x: min_x, y: min_y },
    ]
}

/// Bambu Studio dialect, mirroring a real probe slice (goat1 magnet, bambu_a2l): machine
/// start gcode flushes without XY moves, the prime tower prints FIRST inside layer 1 wrapped
/// in `; WIPE_TOWER_START/END` with its own `; layer #N` empty-grid counter inside, the
/// object follows, and object layers are delimited by `; CHANGE_LAYER`. A parser that only
/// understood the Orca markers (`;LAYER_CHANGE`/`;TYPE:`) read the WHOLE gcode here — all
/// 43 layers with the tower — which glued a 15 mm phantom tower onto every 54 mm goat and
/// halved the plate (18 objects instead of 30).
#[test]
fn first_layer_patch_parses_bambu_studio_dialect() {
    let gcode = b"
G1 X100.000 Y100.000 F30000
G1 E5.0 F1800
; CHANGE_LAYER
; Z_HEIGHT: 0.2
; LAYER_HEIGHT: 0.2
; FEATURE: Prime tower
; LINE_WIDTH: 0.500000
; WIPE_TOWER_START
G1  X225.600 Y187.000  E0.3421 F3000
G1  X234.600 Y160.500  E1.0072
; CP EMPTY GRID START
; layer #2
G1  X226.100 Y161.000  E0.0190
; WIPE_TOWER_END
; FEATURE: Outer wall
; LINE_WIDTH: 0.500000
G1 X60.000 Y60.000 F30000
G1 X60.250 Y60.250 E0.1137
G1 X70.250 Y60.250 E4.5480
G1 X70.250 Y70.250 E4.5480
G1 X60.250 Y70.250 E4.5480
G1 X60.250 Y60.250 E4.5480
; FEATURE: Support
; LINE_WIDTH: 0.500000
G1 X50.250 Y65.250 F30000
G1 X55.250 Y65.250 E2.2740
; CHANGE_LAYER
; Z_HEIGHT: 0.4
; FEATURE: Outer wall
G1 X200.000 Y200.000 F30000
G1 X210.000 Y200.000 E4.5480
";
    let patch = first_layer_footprint(gcode, Some((65.25, 65.25)))
        .expect("layer-1 object extrusions exist");

    let min_x = patch
        .loops
        .iter()
        .flatten()
        .map(|p| p.x)
        .fold(f64::INFINITY, f64::min);
    let max_x = patch
        .loops
        .iter()
        .flatten()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = patch
        .loops
        .iter()
        .flatten()
        .map(|p| p.y)
        .fold(f64::INFINITY, f64::min);
    let max_y = patch
        .loops
        .iter()
        .flatten()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max);
    // Body square 60.25..70.25 with a support strip to its left: the patch spans about
    // -15.25..+5.25 around the body center. The tower (x ~225), the empty grid inside it,
    // the layer-2 wall (x ~200) and any start-gcode extrusion would all push past these.
    assert!(min_x > -16.5 && min_x < -14.0, "min_x {min_x}");
    assert!(max_x > 4.0 && max_x < 6.5, "max_x {max_x}");
    assert!(min_y > -6.5 && min_y < -4.0, "min_y {min_y}");
    assert!(max_y > 4.0 && max_y < 6.5, "max_y {max_y}");
    assert!(
        patch.width > 19.0 && patch.width < 22.0,
        "width {}",
        patch.width
    );
    assert!(
        patch.height > 9.5 && patch.height < 11.5,
        "height {}",
        patch.height
    );
}

#[test]
fn first_layer_bambu_tower_only_yields_none() {
    // Bambu dialect, tower-only first layer: nothing to pack against, same as the Orca case.
    let gcode = b"
; CHANGE_LAYER
; Z_HEIGHT: 0.2
; FEATURE: Prime tower
; WIPE_TOWER_START
G1 X225.600 Y187.000 E0.3421
G1 X234.600 Y187.000 E1.0072
; layer #2
G1 X226.100 Y161.000 E0.0190
; WIPE_TOWER_END
; CHANGE_LAYER
; Z_HEIGHT: 0.4
G1 X60.250 Y60.250 E0.1137
";
    assert!(first_layer_footprint(gcode, None).is_none());
}

fn closed_loop(points: &[(f64, f64)]) -> Vec<Point> {
    let mut loop_: Vec<Point> = points.iter().map(|(x, y)| Point { x: *x, y: *y }).collect();
    loop_.push(loop_[0]);
    loop_
}

fn loops_footprint(loops: Vec<Vec<Point>>) -> Footprint {
    Footprint {
        body_loops: loops.clone(),
        bed_loops: None,
        loops,
        width: 0.0,
        height: 0.0,
        min_z: 0.0,
        max_z: 0.0,
        triangle_count: 0,
    }
}

/// A prismatic model's walls run along its outline, bulging only half a line width past it:
/// the patch counts as inside the silhouette and the packer may fall back to the (rotatable)
/// silhouette. Supports wandering a few mm out must flip the answer.
#[test]
fn patch_within_silhouette_separates_prismatic_from_supported_models() {
    let silhouette = loops_footprint(vec![closed_loop(&[
        (-10.0, -10.0),
        (10.0, -10.0),
        (10.0, 10.0),
        (-10.0, 10.0),
    ])]);
    // Prismatic: the same square, walls inset 0.3 mm; a 1.0 mm wall-bead miter on a sharp
    // tip and the ~1.6 mm arachne slivers along a curved belly (measured on the P1S goat:
    // 23 slivers, 2.45 mm² total) all stay under the 2.0 mm tolerance.
    let prismatic = loops_footprint(vec![closed_loop(&[
        (-10.0, -10.0),
        (11.0, -10.0),
        (11.0, 10.0),
        (-10.0, 10.0),
    ])]);
    assert!(patch_within_silhouette(&prismatic, &silhouette, 0.25, 2.0));

    // Supported: a 3 mm support blob sticking out past the right edge — well beyond any
    // wall-bead artifact the tolerance absorbs.
    let supported = loops_footprint(vec![
        closed_loop(&[(-9.7, -9.7), (9.7, -9.7), (9.7, 9.7), (-9.7, 9.7)]),
        closed_loop(&[(10.0, -2.0), (13.0, -2.0), (13.0, 2.0), (10.0, 2.0)]),
    ]);
    assert!(!patch_within_silhouette(&supported, &silhouette, 0.25, 2.0));
}

/// A keyring hole in the silhouette is not "sticking out": the patch's collision mask
/// deliberately fills it, so the containment runs against the hole-filled silhouette. This
/// exact shape (regular goat1: 4×4 mm hole, magnet twin hole-less) sent one twin of two
/// identical models down the sparse locked-patch branch.
#[test]
fn patch_within_silhouette_ignores_silhouette_holes() {
    let holed_silhouette = loops_footprint(vec![
        closed_loop(&[(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)]),
        closed_loop(&[(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)]),
    ]);
    // The patch covers the hole (exterior-only ring): contained.
    let covering = loops_footprint(vec![closed_loop(&[
        (-9.7, -9.7),
        (9.7, -9.7),
        (9.7, 9.7),
        (-9.7, 9.7),
    ])]);
    assert!(patch_within_silhouette(
        &covering,
        &holed_silhouette,
        0.25,
        1.2
    ));
}

#[test]
fn loops_touch_rect_detects_real_overlaps_only() {
    let square = vec![closed_loop(&[
        (10.0, 10.0),
        (30.0, 10.0),
        (30.0, 30.0),
        (10.0, 30.0),
    ])];

    // Overlapping corner of the loop area.
    assert!(loops_touch_rect(&square, Rect::new(25.0, 25.0, 35.0, 35.0)));
    // Rect fully inside the loop: its samples fall inside the loops.
    assert!(loops_touch_rect(&square, Rect::new(15.0, 15.0, 25.0, 25.0)));
    // Rect enclosing the whole loop: loop vertices fall inside the rect.
    assert!(loops_touch_rect(&square, Rect::new(0.0, 0.0, 40.0, 40.0)));
    // Near miss with a clear gap.
    assert!(!loops_touch_rect(&square, Rect::new(31.0, 0.0, 40.0, 9.0)));
    // Touching only a loop vertex region (no rect sample inside the loops).
    assert!(loops_touch_rect(&square, Rect::new(29.5, 29.5, 30.5, 30.5)));
    // A strip adjacent to the edge but not overlapping it.
    assert!(!loops_touch_rect(
        &square,
        Rect::new(30.2, 12.0, 36.0, 28.0)
    ));
}

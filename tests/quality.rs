use goat_packer::{
    PackOptions, Rect, axis_aligned_line_keepout, extract_footprint, pack_footprint,
    validate_result,
};

#[test]
fn p1s_search_meets_existing_seventeen_model_baseline() {
    let footprint = extract_footprint(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/goat4_v2.stl"
    )))
    .unwrap();
    let options = PackOptions {
        bed: Rect::new(0.0, 0.0, 256.0, 256.0),
        exclusions: vec![
            Rect::new(0.0, 0.0, 18.0, 28.0),
            axis_aligned_line_keepout(18.0, 1.0, 240.0, 1.0, 0.5).unwrap(),
            axis_aligned_line_keepout(240.0, 1.0, 240.0, 11.0, 0.5).unwrap(),
            axis_aligned_line_keepout(239.5, 11.0, 239.5, 1.5, 0.5).unwrap(),
            axis_aligned_line_keepout(239.5, 1.5, 18.0, 1.5, 0.5).unwrap(),
        ],
        resolution: 0.5,
        clearance: 0.7,
        angle_step_deg: 5.0,
        attempts: 1_200,
        candidates_per_step: 1_600,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 20_270_826,
        support_overlap_mm: None,
    };
    let result = pack_footprint(&footprint, &options);
    validate_result(&footprint, &options, &result).unwrap();
    assert!(
        result.placements.len() >= 17,
        "search regressed below the valid existing 17-model contour layout: {result:#?}"
    );
}

#[test]
fn u1_search_keeps_eighteen_models_with_edge_margin_and_purge_line() {
    let footprint = extract_footprint(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/goat4_v2.stl"
    )))
    .unwrap();
    let options = PackOptions {
        bed: Rect::new(0.5, 1.0, 270.5, 271.0).inset(3.5).unwrap(),
        exclusions: vec![axis_aligned_line_keepout(85.0, 1.0, 185.0, 1.0, 0.5).unwrap()],
        resolution: 0.5,
        clearance: 0.7,
        angle_step_deg: 5.0,
        attempts: 1_200,
        candidates_per_step: 1_600,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 20_270_829,
        support_overlap_mm: None,
    };
    let result = pack_footprint(&footprint, &options);
    validate_result(&footprint, &options, &result).unwrap();
    assert!(
        result.placements.len() >= 18,
        "U1 safe-area search regressed: {result:#?}"
    );
}

mod support;

fn fixture_options() -> PackOptions {
    let profile = goat_packer::colored_packer::load_printer_profile(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/profiles/p1s.json"
    )))
    .unwrap();
    let mut exclusions = profile.fixed_keepouts(1.7).unwrap();
    exclusions.push(Rect::new(225.85, 200.10, 246.15, 252.15));
    PackOptions {
        bed: profile.usable_bed().unwrap(),
        exclusions,
        resolution: 0.25,
        clearance: 0.7,
        angle_step_deg: 2.,
        attempts: 1000,
        candidates_per_step: 1000,
        max_objects: 0,
        lock_orientation: false,
        stall_attempts: 0,
        seed: 1,
        support_overlap_mm: None,
    }
}

#[test]
fn real_contours_remain_stable_after_rigid_transforms() {
    let fixtures = [
        include_str!("fixtures/goat4.json"),
        include_str!("fixtures/goat4m.json"),
        include_str!("fixtures/goat3.json"),
    ];
    let opts = fixture_options();
    for (index, json) in fixtures.iter().enumerate() {
        let fp: goat_packer::Footprint = serde_json::from_str(json).unwrap();
        let base = pack_footprint(&fp, &opts);
        validate_result(&fp, &opts, &base).unwrap();
        for angle in [13., 45., 137.] {
            let transformed = support::transform(&fp, angle, 137.139, -52.723, true);
            let result = pack_footprint(&transformed, &opts);
            validate_result(&transformed, &opts, &result).unwrap();
            assert_eq!(
                result.placements.len(),
                base.placements.len(),
                "fixture {index}, angle {angle}"
            );
        }
    }
}

#[test]
fn real_stl_reexport_rotation_and_translation_preserve_count() {
    let bytes = include_bytes!("fixtures/goat4_v2.stl");
    let opts = fixture_options();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rotated.stl");
    let mut counts = Vec::new();
    for angle in [0f64, 13., 45., 90., 137.] {
        let (s, c) = angle.to_radians().sin_cos();
        let mut rotated = bytes.to_vec();
        for triangle in rotated[84..].chunks_exact_mut(50) {
            for vertex in triangle[12..48].chunks_exact_mut(12) {
                let x = f32::from_le_bytes(vertex[..4].try_into().unwrap()) as f64;
                let y = f32::from_le_bytes(vertex[4..8].try_into().unwrap()) as f64;
                vertex[..4].copy_from_slice(&((x * c - y * s + 123.456) as f32).to_le_bytes());
                vertex[4..8].copy_from_slice(&((x * s + y * c - 54.321) as f32).to_le_bytes());
            }
        }
        std::fs::write(&path, rotated).unwrap();
        let fp = extract_footprint(&path).unwrap();
        let result = pack_footprint(&fp, &opts);
        validate_result(&fp, &opts, &result).unwrap();
        counts.push(result.placements.len());
    }
    assert!(
        counts.iter().all(|&n| n == counts[0]),
        "STL reexport counts: {counts:?}"
    );
}

#[test]
fn real_layout_clearance_passes_independent_vector_geometry_check() {
    use geo::{Distance, Euclidean, LineString, Polygon};
    let fp: goat_packer::Footprint =
        serde_json::from_str(include_str!("fixtures/goat4.json")).unwrap();
    let opts = fixture_options();
    let result = pack_footprint(&fp, &opts);
    let polygons: Vec<_> = result
        .placements
        .iter()
        .map(|p| {
            let (s, c) = p.angle_deg.to_radians().sin_cos();
            let rings: Vec<LineString> = fp
                .loops
                .iter()
                .map(|l| {
                    LineString::from(
                        l.iter()
                            .map(|v| (v.x * c - v.y * s + p.x_mm, v.x * s + v.y * c + p.y_mm))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            Polygon::new(rings[0].clone(), rings[1..].to_vec())
        })
        .collect();
    for (i, a) in polygons.iter().enumerate() {
        for (j, b) in polygons[..i].iter().enumerate() {
            let gap = Euclidean.distance(a, b);
            assert!(
                gap + 1e-6 >= opts.clearance,
                "placements {i},{j}: gap {gap} < {}",
                opts.clearance
            );
        }
    }
}

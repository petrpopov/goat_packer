use goat_packer::{
    PackOptions, Rect, axis_aligned_line_keepout, extract_footprint, pack_footprint,
    validate_result,
};

#[test]
fn p1s_search_meets_existing_seventeen_model_baseline() {
    let footprint =
        extract_footprint(std::path::Path::new("../Goat 4/source/goat4_v2.stl")).unwrap();
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
    let footprint =
        extract_footprint(std::path::Path::new("../Goat 4/source/goat4_v2.stl")).unwrap();
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
    };
    let result = pack_footprint(&footprint, &options);
    validate_result(&footprint, &options, &result).unwrap();
    assert!(
        result.placements.len() >= 18,
        "U1 safe-area search regressed: {result:#?}"
    );
}

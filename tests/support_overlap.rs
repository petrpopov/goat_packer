//! `support_overlap_mm` (spec 2026-10-26-ams-calibration-and-supports-design, section 6):
//! support zones of different objects may overlap by at most X; bodies keep the two-zone rule.
//!
//! The footprint is an 8x8 mm support skirt (the full first-layer patch) around a 4x4 mm body,
//! packed at a locked 0° so every geometric distance below is an exact axis-aligned formula.
use goat_packer::{
    Footprint, PackOptions, PackResult, PackedPlacement, Point, Rect, grid_layout,
    pack_footprint, validate_result,
};

const FULL_HALF: f64 = 4.0;
const BODY_HALF: f64 = 2.0;
const RESOLUTION: f64 = 0.25;
const CLEARANCE: f64 = 0.5;
/// A pixel of rasterization error on each side of each object.
const TOLERANCE: f64 = 2.0 * RESOLUTION + 1e-9;

fn square(half: f64) -> Vec<Point> {
    let mut ring: Vec<Point> = [(-half, -half), (half, -half), (half, half), (-half, half)]
        .iter()
        .map(|&(x, y)| Point { x, y })
        .collect();
    ring.push(ring[0]);
    ring
}

fn skirted() -> Footprint {
    Footprint {
        support_landing_loops: Vec::new(),
        loops: vec![square(FULL_HALF)],
        body_loops: vec![square(BODY_HALF)],
        bed_loops: None,
        width: 2.0 * FULL_HALF,
        height: 2.0 * FULL_HALF,
        min_z: 0.0,
        max_z: 1.0,
        triangle_count: 2,
    }
}

fn options(support_overlap_mm: Option<f64>) -> PackOptions {
    PackOptions {
        body_exclusions: Vec::new(),
        // One row of objects: every neighbour pair is a horizontal pair. 34.5 mm fits five
        // at the unlimited 6.5 mm pitch but only four at the 8 mm pitch of X = 0.
        bed: Rect::new(0.0, 0.0, 34.5, 9.0),
        exclusions: vec![],
        resolution: RESOLUTION,
        clearance: CLEARANCE,
        angle_step_deg: 90.0,
        attempts: 16,
        candidates_per_step: 10,
        max_objects: 0,
        lock_orientation: true,
        stall_attempts: 0,
        seed: 1,
        support_overlap_mm,
    }
}

/// Signed distance between two axis-aligned squares centred `(dx, dy)` apart with the given
/// half sizes: the Euclidean gap when apart, minus the penetration depth when overlapping.
fn signed_gap(dx: f64, dy: f64, half_a: f64, half_b: f64) -> f64 {
    let reach = half_a + half_b;
    let gx = (dx.abs() - reach).max(0.0);
    let gy = (dy.abs() - reach).max(0.0);
    if gx > 0.0 || gy > 0.0 {
        gx.hypot(gy)
    } else {
        -(reach - dx.abs().max(dy.abs()))
    }
}

fn pairs(result: &PackResult) -> Vec<(f64, f64)> {
    let p = &result.placements;
    let mut out = Vec::new();
    for i in 0..p.len() {
        for j in 0..i {
            out.push((p[i].x_mm - p[j].x_mm, p[i].y_mm - p[j].y_mm));
        }
    }
    out
}

/// Smallest gap between two supports (full patches) in the layout; negative = overlap depth.
fn min_support_gap(result: &PackResult) -> f64 {
    pairs(result)
        .into_iter()
        .map(|(dx, dy)| signed_gap(dx, dy, FULL_HALF, FULL_HALF))
        .fold(f64::INFINITY, f64::min)
}

/// Smallest gap between a body and anything foreign.
fn min_body_gap(result: &PackResult) -> f64 {
    pairs(result)
        .into_iter()
        .map(|(dx, dy)| signed_gap(dx, dy, BODY_HALF, FULL_HALF))
        .fold(f64::INFINITY, f64::min)
}

fn layouts(options: &PackOptions) -> [PackResult; 2] {
    let fp = skirted();
    let searched = pack_footprint(&fp, options);
    let grid = grid_layout(&fp, options);
    validate_result(&fp, options, &searched).unwrap();
    validate_result(&fp, options, &grid).unwrap();
    [searched, grid]
}

fn layout(placements: &[(f64, f64)], options: &PackOptions) -> PackResult {
    PackResult {
        placements: placements
            .iter()
            .map(|&(x_mm, y_mm)| PackedPlacement {
                x_mm,
                y_mm,
                angle_deg: 0.0,
            })
            .collect(),
        resolution_mm: options.resolution,
        requested_clearance_mm: options.clearance,
        angle_step_deg: options.angle_step_deg,
        attempts: 1,
        attempts_run: 1,
        seed: options.seed,
    }
}

#[test]
fn unlimited_lets_supports_overlap() {
    for result in layouts(&options(None)) {
        assert!(result.placements.len() >= 2, "{result:#?}");
        assert!(
            min_support_gap(&result) < 0.0,
            "default packing should nest support skirts: {result:#?}"
        );
        assert!(min_body_gap(&result) >= CLEARANCE - TOLERANCE, "{result:#?}");
    }
}

#[test]
fn zero_forbids_support_overlap_but_lets_supports_touch() {
    let unlimited = pack_footprint(&skirted(), &options(None));
    for result in layouts(&options(Some(0.0))) {
        assert!(result.placements.len() >= 2, "{result:#?}");
        let gap = min_support_gap(&result);
        // Printed supports never overlap...
        assert!(gap >= -TOLERANCE, "gap {gap}: {result:#?}");
        // ...but neighbours may touch: no clearance is added between supports.
        assert!(gap <= TOLERANCE, "gap {gap}: {result:#?}");
        assert!(min_body_gap(&result) >= CLEARANCE - TOLERANCE, "{result:#?}");
        // 8 mm pitch instead of 6.5 mm: the rule costs an object on this row.
        assert!(result.placements.len() < unlimited.placements.len());
    }
}

#[test]
fn x_bounds_support_overlap_depth() {
    for overlap in [0.5, 1.0, 1.5] {
        for result in layouts(&options(Some(overlap))) {
            assert!(result.placements.len() >= 2, "{result:#?}");
            let gap = min_support_gap(&result);
            // Printed supports overlap by at most X...
            assert!(-gap <= overlap + TOLERANCE, "X={overlap} overlap {}", -gap);
            // ...and the rule binds: neighbours pack right up to the allowance (X <= 1.5 mm,
            // the most the bodies' clearance leaves on this skirt).
            assert!(-gap >= overlap - TOLERANCE, "X={overlap} overlap {}", -gap);
            assert!(min_body_gap(&result) >= CLEARANCE - TOLERANCE, "X={overlap}");
        }
    }
}

#[test]
fn bodies_never_overlap_even_with_a_huge_allowance() {
    // X far beyond the skirt width: the support cores erode to nothing, the body rule remains.
    for result in layouts(&options(Some(50.0))) {
        assert!(min_body_gap(&result) >= CLEARANCE - TOLERANCE, "{result:#?}");
        assert_eq!(
            result.placements.len(),
            pack_footprint(&skirted(), &options(None)).placements.len()
        );
    }
}

#[test]
fn validate_result_rejects_a_layout_exceeding_x() {
    let fp = skirted();
    // 6.75 mm apart: skirts overlap by 1.25 mm, bodies keep 0.75 mm from the foreign skirt.
    let close = |options: &PackOptions| layout(&[(10.0, 4.5), (16.75, 4.5)], options);
    validate_result(&fp, &options(None), &close(&options(None)))
        .expect("unlimited support overlap accepts it");
    assert!(validate_result(&fp, &options(Some(1.0)), &close(&options(Some(1.0)))).is_err());
    assert!(validate_result(&fp, &options(Some(0.0)), &close(&options(Some(0.0)))).is_err());
    // 7.0 mm apart: skirts overlap by exactly 1.0 mm — allowed at X = 1.0.
    let at_limit = layout(&[(10.0, 4.5), (17.0, 4.5)], &options(Some(1.0)));
    validate_result(&fp, &options(Some(1.0)), &at_limit).unwrap();
    let beyond_limit = layout(&[(10.0, 4.5), (16.75, 4.5)], &options(Some(1.0)));
    assert!(validate_result(&fp, &options(Some(1.0)), &beyond_limit).is_err());
    // 8 mm apart: supports touch — allowed at X = 0; 7.75 mm (0.25 mm overlap) is not.
    let touching = layout(&[(10.0, 4.5), (18.0, 4.5)], &options(Some(0.0)));
    validate_result(&fp, &options(Some(0.0)), &touching).unwrap();
    let overlapping = layout(&[(10.0, 4.5), (17.75, 4.5)], &options(Some(0.0)));
    let error = validate_result(&fp, &options(Some(0.0)), &overlapping).unwrap_err();
    assert!(error.contains("supports overlap"), "{error}");
}

#[test]
fn without_a_support_zone_the_option_is_a_no_op() {
    let mut fp = skirted();
    fp.body_loops.clear();
    let mut unlocked = options(None);
    unlocked.lock_orientation = false;
    unlocked.angle_step_deg = 30.0;
    let baseline = pack_footprint(&fp, &unlocked);
    for overlap in [0.0, 1.0, 50.0] {
        let mut with = unlocked.clone();
        with.support_overlap_mm = Some(overlap);
        let result = pack_footprint(&fp, &with);
        assert_eq!(
            serde_json::to_string(&result.placements).unwrap(),
            serde_json::to_string(&baseline.placements).unwrap()
        );
        validate_result(&fp, &with, &result).unwrap();
    }
}

#[test]
fn negative_or_non_finite_overlap_is_rejected() {
    let fp = skirted();
    let result = layout(&[(10.0, 4.5)], &options(None));
    for bad in [-0.1, f64::NAN, f64::INFINITY] {
        assert!(validate_result(&fp, &options(Some(bad)), &result).is_err());
    }
}

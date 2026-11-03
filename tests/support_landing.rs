//! Support landing rule (spec 2026-10-31-support-landing-and-progress-design, rule 1) and the
//! body-only tower exclusion (rule 2).
//!
//! The footprint: an 8x8 mm first-layer patch (supports) around a 6x6 mm silhouette whose
//! first layer prints only a 4x4 mm landing zone, packed at a locked 0°, so every pitch below
//! is an exact axis-aligned sum of half sizes plus the clearance.
use goat_packer::{
    Footprint, PackOptions, PackResult, PackedPlacement, Point, Rect, grid_layout, pack_footprint,
    validate_result,
};

const FULL_HALF: f64 = 4.0;
const LANDING_HALF: f64 = 2.0;
const RESOLUTION: f64 = 0.25;
const CLEARANCE: f64 = 0.5;

fn square(half: f64) -> Vec<Point> {
    let mut ring: Vec<Point> = [(-half, -half), (half, -half), (half, half), (-half, half)]
        .iter()
        .map(|&(x, y)| Point { x, y })
        .collect();
    ring.push(ring[0]);
    ring
}

/// `landing`: whether the footprint carries landing loops (false = the old two-zone rule,
/// where the whole silhouette is what a foreign patch must not touch).
fn footprint(body_half: f64, landing: bool) -> Footprint {
    Footprint {
        loops: vec![square(FULL_HALF)],
        body_loops: vec![square(body_half)],
        support_landing_loops: if landing {
            vec![square(LANDING_HALF)]
        } else {
            Vec::new()
        },
        bed_loops: None,
        layer_zones: Vec::new(),
        width: 2.0 * FULL_HALF,
        height: 2.0 * FULL_HALF,
        min_z: 0.0,
        max_z: 1.0,
        triangle_count: 2,
    }
}

fn options() -> PackOptions {
    PackOptions {
        body_bed: None,
        // One row: 34.5 mm holds five objects at a 6.5 mm pitch, four at 7.5 mm.
        bed: Rect::new(0.0, 0.0, 34.5, 9.0),
        exclusions: vec![],
        body_exclusions: vec![],
        resolution: RESOLUTION,
        clearance: CLEARANCE,
        angle_step_deg: 90.0,
        attempts: 16,
        candidates_per_step: 10,
        max_objects: 0,
        lock_orientation: true,
        stall_attempts: 0,
        seed: 1,
        support_overlap_mm: None,
        foreign_gaps: Default::default(),
        plate_rules: None,
    }
}

fn layout(xs: &[f64]) -> PackResult {
    PackResult {
        placements: xs
            .iter()
            .map(|&x| PackedPlacement {
                x_mm: x,
                y_mm: 4.5,
                angle_deg: 0.0,
            })
            .collect(),
        resolution_mm: RESOLUTION,
        requested_clearance_mm: CLEARANCE,
        angle_step_deg: 90.0,
        attempts: 1,
        attempts_run: 1,
        seed: 1,
    }
}

#[test]
fn supports_may_land_under_a_foreign_silhouette_outside_its_landing_zone() {
    // Patch 4 + landing 2 + clearance 0.5 = 6.5; silhouettes 3 + 3 + 0.5 = 6.5.
    let pitch = 6.5 + RESOLUTION;
    let fp = footprint(3.0, true);
    assert!(validate_result(&fp, &options(), &layout(&[4.25, 4.25 + pitch])).is_ok());
    // The same pair under the old rule: patch 4 + silhouette 3 + 0.5 = 7.5 > 6.75.
    let old = footprint(3.0, false);
    assert!(validate_result(&old, &options(), &layout(&[4.25, 4.25 + pitch])).is_err());
}

#[test]
fn supports_must_not_touch_a_foreign_landing_zone() {
    let fp = footprint(3.0, true);
    assert!(validate_result(&fp, &options(), &layout(&[4.25, 4.25 + 6.0])).is_err());
}

#[test]
fn silhouettes_keep_clearance_when_the_landing_zone_is_narrower() {
    // Silhouettes 3.5 + 3.5 + 0.5 = 7.5 binds before the 6.5 landing pitch.
    let fp = footprint(3.5, true);
    assert!(validate_result(&fp, &options(), &layout(&[4.25, 4.25 + 6.75])).is_err());
    assert!(validate_result(&fp, &options(), &layout(&[4.25, 4.25 + 7.75])).is_ok());
}

#[test]
fn search_and_grid_pack_the_denser_landing_pitch() {
    for (landing, expected) in [(true, 5), (false, 4)] {
        let fp = footprint(3.0, landing);
        let options = options();
        let searched = pack_footprint(&fp, &options);
        let grid = grid_layout(&fp, &options);
        validate_result(&fp, &options, &searched).unwrap();
        validate_result(&fp, &options, &grid).unwrap();
        assert_eq!(
            searched.placements.len(),
            expected,
            "search, landing={landing}"
        );
        assert_eq!(grid.placements.len(), expected, "grid, landing={landing}");
    }
}

/// A body exclusion (the tower keepout) is a restricted zone: the part box may not overlap
/// it, touching passes, and supports may enter.
#[test]
fn body_exclusions_admit_supports_but_not_the_body() {
    let fp = footprint(3.0, true);
    let zone = Rect::new(12.0, 0.0, 14.0, 9.0);
    // Body reaches 8.25 + 3 = 11.25 < 12; the support base reaches 12.25.
    let near = layout(&[8.25]);
    let mut body_only = options();
    body_only.body_exclusions = vec![zone];
    assert!(validate_result(&fp, &body_only, &near).is_ok());
    let mut full = options();
    full.exclusions = vec![zone];
    assert!(validate_result(&fp, &full, &near).is_err());
    // The body box touching the zone passes; entering it does not.
    assert!(validate_result(&fp, &body_only, &layout(&[9.0])).is_ok());
    assert!(validate_result(&fp, &body_only, &layout(&[9.25])).is_err());
}

#[test]
fn search_respects_body_exclusions() {
    let fp = footprint(3.0, true);
    let mut options = options();
    options.body_exclusions = vec![Rect::new(15.0, 0.0, 19.5, 9.0)];
    let searched = pack_footprint(&fp, &options);
    let grid = grid_layout(&fp, &options);
    validate_result(&fp, &options, &searched).unwrap();
    validate_result(&fp, &options, &grid).unwrap();
}

/// Rule 3: a body bed (`PackOptions::body_bed`, the profile's edge margin) binds the model,
/// while the supports may print up to the wider `bed` (the printable area). The legacy body
/// bed becomes one restricted band as wide as its widest inset, so the insets here are even.
fn edge_options(bed: Rect, body_bed: Option<Rect>) -> PackOptions {
    PackOptions {
        bed,
        body_bed,
        ..options()
    }
}

#[test]
fn supports_may_enter_the_edge_margin_the_body_may_not() {
    let fp = footprint(3.0, true);
    let margin = Rect::new(0.0, 0.0, 32.5, 9.0);
    let options = edge_options(Rect::new(-2.0, -2.0, 34.5, 11.0), Some(margin));
    // Patch -1..7 inside the bed, body 0..6 inside the margin.
    assert!(validate_result(&fp, &options, &layout(&[3.0])).is_ok());
    // The body leaves the margin by 0.25 mm while the patch is still on the bed.
    let error = validate_result(&fp, &options, &layout(&[2.75])).unwrap_err();
    assert!(error.contains("body leaves"), "{error}");
    // Without a body bed the margin binds the whole patch, as before.
    assert!(validate_result(&fp, &edge_options(margin, None), &layout(&[3.0])).is_err());
}

#[test]
fn search_and_grid_use_the_edge_margin_for_the_supports() {
    let fp = footprint(3.0, true);
    let margin = Rect::new(0.0, 0.0, 32.5, 9.0);
    // Patch-bound: centres 4..28.5 at a 6.5 pitch hold 4; body-bound: 3..29.5 hold 5.
    for (options, expected) in [
        (edge_options(margin, None), 4),
        (
            edge_options(Rect::new(-1.0, -1.0, 33.5, 10.0), Some(margin)),
            5,
        ),
    ] {
        let searched = pack_footprint(&fp, &options);
        let grid = grid_layout(&fp, &options);
        validate_result(&fp, &options, &searched).unwrap();
        validate_result(&fp, &options, &grid).unwrap();
        assert_eq!(
            searched.placements.len(),
            expected,
            "search, {:?}",
            options.body_bed
        );
        assert_eq!(
            grid.placements.len(),
            expected,
            "grid, {:?}",
            options.body_bed
        );
    }
    // A body bed equal to the bed changes nothing.
    let plain = pack_footprint(&fp, &edge_options(margin, None));
    let same = pack_footprint(&fp, &edge_options(margin, Some(margin)));
    assert_eq!(
        serde_json::to_value(&plain.placements).unwrap(),
        serde_json::to_value(&same.placements).unwrap()
    );
}

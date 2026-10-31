//! Support landing rule (spec 2026-10-31-support-landing-and-progress-design, rule 1) and the
//! body-only tower exclusion (rule 2).
//!
//! The footprint: an 8x8 mm first-layer patch (supports) around a 6x6 mm silhouette whose
//! first layer prints only a 4x4 mm landing zone, packed at a locked 0°, so every pitch below
//! is an exact axis-aligned sum of half sizes plus the clearance.
use goat_packer::{
    Footprint, PackOptions, PackResult, PackedPlacement, Point, Rect, grid_layout,
    pack_footprint, validate_result,
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
        width: 2.0 * FULL_HALF,
        height: 2.0 * FULL_HALF,
        min_z: 0.0,
        max_z: 1.0,
        triangle_count: 2,
    }
}

fn options() -> PackOptions {
    PackOptions {
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
        assert_eq!(searched.placements.len(), expected, "search, landing={landing}");
        assert_eq!(grid.placements.len(), expected, "grid, landing={landing}");
    }
}

#[test]
fn body_exclusions_admit_supports_but_not_the_body() {
    let fp = footprint(3.0, true);
    let zone = Rect::new(12.0, 0.0, 14.0, 9.0);
    // Body reaches 8.25 + 3 + 0.5 = 11.75 < 12; the patch reaches 12.25 + 0.5.
    let near = layout(&[8.25]);
    let mut body_only = options();
    body_only.body_exclusions = vec![zone];
    assert!(validate_result(&fp, &body_only, &near).is_ok());
    let mut full = options();
    full.exclusions = vec![zone];
    assert!(validate_result(&fp, &full, &near).is_err());
    // The body itself may not enter: 9 + 3 + 0.5 = 12.5 > 12.
    assert!(validate_result(&fp, &body_only, &layout(&[9.0])).is_err());
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

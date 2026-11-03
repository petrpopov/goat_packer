//! Plate constraints: printable-area insets, purge keepouts, and the prime tower as zones the
//! packing portfolio places (`goat_packer::pack`). Generated prints and plates only.

use goat_packer::keepout::{BodyRule, Keepout, KeepoutClass, KeepoutFrame, KeepoutShape};
use goat_packer::kind_masks::{OriginGrid, plate_zones};
use goat_packer::pack::{TowerSpec, anchor_candidates, pack};
use goat_packer::{
    LayoutRules, ObjectPrint, PackOptions, PlateRules, Point, PrintPlacement, Rect,
    axis_aligned_line_keepout, validate_raster, validate_vector,
};

#[test]
fn insets_template_printable_area_by_required_edge_margin() {
    let printable = Rect::new(0.5, 1.0, 270.5, 271.0);
    let inset = printable.inset(3.5).unwrap();
    assert_eq!(inset.min_x, 4.0);
    assert_eq!(inset.min_y, 4.5);
    assert_eq!(inset.max_x, 267.0);
    assert_eq!(inset.max_y, 267.5);
    assert!(printable.inset(136.0).is_err());
}

#[test]
fn converts_axis_aligned_purge_segments_to_nozzle_width_keepouts() {
    let horizontal = axis_aligned_line_keepout(85.0, 1.0, 185.0, 1.0, 0.5).unwrap();
    assert_eq!(horizontal.min_x, 84.75);
    assert_eq!(horizontal.min_y, 0.75);
    assert_eq!(horizontal.max_x, 185.25);
    assert_eq!(horizontal.max_y, 1.25);

    let vertical = axis_aligned_line_keepout(240.0, 1.0, 240.0, 11.0, 0.5).unwrap();
    assert_eq!(vertical.min_x, 239.75);
    assert_eq!(vertical.min_y, 0.75);
    assert_eq!(vertical.max_x, 240.25);
    assert_eq!(vertical.max_y, 11.25);

    assert!(axis_aligned_line_keepout(0.0, 0.0, 1.0, 1.0, 0.5).is_err());
}

const CLEARANCE: f64 = 0.7;

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
        .iter()
        .map(|&(x, y)| Point { x, y })
        .collect()
}

/// A 4 x 3 mm body, two bands tall, on a support base reaching 2.5 mm past its right side.
fn print() -> ObjectPrint {
    let body = rect(-2.0, -1.5, 2.0, 1.5);
    ObjectPrint {
        silhouette: vec![body.clone()],
        part_hulls: vec![body.clone()],
        part_boxes: vec![Rect::new(-2.0, -1.5, 2.0, 1.5)],
        landing: vec![body.clone()],
        support_base: vec![rect(2.0, -1.0, 4.5, 1.0)],
        body: vec![vec![body.clone()], vec![body]],
        support: vec![vec![], vec![rect(2.0, -0.5, 3.0, 0.5)]],
        interface: vec![vec![], vec![]],
        ..ObjectPrint::default()
    }
}

fn plate(keepouts: Vec<Keepout>) -> PlateRules {
    PlateRules {
        printable_area: Rect::new(0.0, 0.0, 60.0, 40.0),
        body_edge_band_mm: 0.0,
        print_edge_reserve_mm: 0.0,
        keepouts,
        body_rule: BodyRule::Silhouette,
    }
}

fn hard(rect: Rect) -> Keepout {
    Keepout {
        id: "hard".to_owned(),
        class: KeepoutClass::Hard,
        shape: KeepoutShape::Rect(rect),
        z_range_mm: None,
        frame: KeepoutFrame::Plate,
    }
}

/// An 8 x 6 mm body starting a quarter millimetre before its anchor (like the measured
/// towers), a 2 mm layer-0 brim and one 1 mm chamfer ring, 3 mm tall.
fn tower() -> TowerSpec {
    TowerSpec {
        body: Rect::new(-0.25, -0.25, 8.0, 6.0),
        brim_l0_mm: 2.0,
        chamfer: vec![[0.5, 1.0]],
        top_z_mm: 3.0,
        anchor_range: Rect::new(2.0, 2.0, 50.0, 30.0),
        source_anchor: Point { x: 70.0, y: 25.0 },
    }
}

fn options() -> PackOptions {
    PackOptions {
        bed: Rect::new(0.0, 0.0, 60.0, 40.0),
        body_bed: None,
        exclusions: Vec::new(),
        body_exclusions: Vec::new(),
        resolution: 0.5,
        clearance: CLEARANCE,
        angle_step_deg: 90.0,
        attempts: 16,
        candidates_per_step: 0,
        stall_attempts: 0,
        max_objects: 0,
        lock_orientation: true,
        seed: 0,
        support_overlap_mm: None,
        foreign_gaps: Default::default(),
        plate_rules: None,
    }
}

/// The layout rules of `plate` with the tower at `anchor`.
fn tower_rules(plate: &PlateRules, anchor: Point) -> LayoutRules {
    let mut rules = LayoutRules::new(plate, None, CLEARANCE, Default::default(), None).unwrap();
    rules.zones.extend(tower().zones(anchor));
    rules
}

fn at(x_mm: f64, y_mm: f64) -> PrintPlacement {
    PrintPlacement {
        print: 0,
        x_mm,
        y_mm,
    }
}

#[test]
fn the_source_anchor_is_clamped_into_its_range_and_corners_keep_the_brim_on_the_plate() {
    let zones = plate_zones(&plate(vec![]), None).unwrap();
    let anchors = anchor_candidates(&tower(), &zones).unwrap();
    // The GUI would move the stored (70, 25) to the edge of its range.
    assert_eq!(anchors[0], Point { x: 50.0, y: 25.0 });
    // The brim reaches 2.25 mm before the anchor, so the low corners sit 2.25 mm in (the
    // range alone would allow 2); the high ones are the range's own.
    let corners = [(2.25, 2.25), (50.0, 2.25), (2.25, 30.0), (50.0, 30.0)];
    let expected: Vec<Point> = corners.iter().map(|&(x, y)| Point { x, y }).collect();
    assert_eq!(anchors[1..], expected[..]);
}

#[test]
fn an_anchor_whose_tower_meets_a_hard_zone_is_dropped() {
    let zones = plate_zones(&plate(vec![hard(Rect::new(0.0, 0.0, 12.0, 12.0))]), None).unwrap();
    let anchors = anchor_candidates(&tower(), &zones).unwrap();
    assert!(
        !anchors.contains(&Point { x: 2.25, y: 2.25 }),
        "{anchors:?}"
    );
    assert_eq!(anchors.len(), 4);
    // A tower that fits nowhere is an error, not a tower-less layout.
    let everywhere = plate_zones(&plate(vec![hard(Rect::new(0.0, 0.0, 60.0, 40.0))]), None);
    assert!(anchor_candidates(&tower(), &everywhere.unwrap()).is_err());
}

#[test]
fn bodies_keep_clearance_from_the_tower_while_supports_may_cross_its_brim() {
    // Tower body x [29.75, 38], y [19.75, 26]; brim 2 mm around it at layer 0.
    let rules = tower_rules(&plate(vec![]), Point { x: 30.0, y: 20.0 });
    let check = |x: f64, y: f64| validate_vector(&[print()], &[at(x, y)], &rules);
    // The support base lies on the brim and the chamfer ring (allowed), the body keeps
    // 0.75 mm from the brim.
    check(24.0, 22.0).unwrap();
    // The support base keeps `SUPPORT_BODY_GAP_MM` (0.5) from the tower body: equality
    // passes, 0.4 mm does not.
    check(24.75, 22.0).unwrap();
    assert!(check(24.85, 22.0).is_err());
    // A body beside the tower body but on its brim breaks the clearance (band 0 meets the
    // brim's layer 0); 0.75 mm off the brim it passes.
    check(32.0, 15.5).unwrap();
    assert!(check(32.0, 17.0).is_err());
}

#[test]
fn pack_keeps_the_layout_valid_around_the_tower_and_honours_the_cap() {
    let plate = plate(vec![]);
    let outcome = pack(&[print()], &plate, Some(&tower()), &options()).unwrap();
    let anchor = outcome.tower_anchor.expect("packed around a tower");
    let rules = tower_rules(&plate, anchor);
    let grid = OriginGrid {
        origin: Point { x: 0.0, y: 0.0 },
        resolution: 0.5,
    };
    assert!(
        outcome.placements.len() >= 40,
        "{}",
        outcome.placements.len()
    );
    validate_vector(&[print()], &outcome.placements, &rules).unwrap();
    validate_raster(&[print()], &outcome.placements, &rules, grid).unwrap();
    assert_eq!(outcome.validation.winner_rank, 0);
    let zones = plate_zones(&plate, None).unwrap();
    assert!(
        anchor_candidates(&tower(), &zones)
            .unwrap()
            .contains(&anchor)
    );

    let capped = PackOptions {
        max_objects: 3,
        ..options()
    };
    let outcome = pack(&[print()], &plate, Some(&tower()), &capped).unwrap();
    assert_eq!(outcome.placements.len(), 3);
}

#[test]
fn pack_reports_every_member_and_rejects_bad_input() {
    let plate = plate(vec![]);
    let outcome = pack(&[print()], &plate, None, &options()).unwrap();
    assert_eq!(outcome.tower_anchor, None);
    let best = outcome.member_counts.iter().map(|m| m.count).max();
    assert_eq!(best, Some(outcome.placements.len()));
    assert!(pack(&[], &plate, None, &options()).is_err());
    let bad = PackOptions {
        resolution: 0.0,
        ..options()
    };
    assert!(pack(&[print()], &plate, None, &bad).is_err());
}

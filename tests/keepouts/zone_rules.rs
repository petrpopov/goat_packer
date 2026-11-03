//! What each zone class allows, on placed objects in exact geometry (design §7.2): a soft
//! strip blocks only the landing, a restricted band tests part boxes with equality passing,
//! a hard zone tests part hulls with touching allowed and keeps every support out.
//!
//! Prints here are built for rotation 0 in the plate-aligned frame, so placing one is a pure
//! translation by its origin.

use goat_packer::colored_packer::bed_rules::{ZoneHit, zone_hits};
use goat_packer::keepout::{BodyRule, Keepout, KeepoutFrame, KeepoutShape, PlateRules};
use goat_packer::{KeepoutClass, ObjectPrint, Point, PrintKind, Rect};

fn ring(points: &[(f64, f64)]) -> Vec<Point> {
    points.iter().map(|&(x, y)| Point { x, y }).collect()
}

fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
    ring(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

/// A 20 mm cube with a support base 3 mm wider on every side and supports 2 mm wider in
/// band 0.
fn cube() -> ObjectPrint {
    let body = square(0.0, 0.0, 20.0, 20.0);
    ObjectPrint {
        silhouette: vec![body.clone()],
        part_hulls: vec![body.clone()],
        part_boxes: vec![Rect::new(0.0, 0.0, 20.0, 20.0)],
        landing: vec![body.clone()],
        body: vec![vec![body]],
        support_base: vec![square(-3.0, -3.0, 23.0, 23.0)],
        support: vec![vec![square(-2.0, -2.0, 22.0, 22.0)]],
        ..ObjectPrint::default()
    }
}

/// A part whose hull is the upper-right half of its 10 mm box: the box reaches corners the
/// hull does not.
fn wedge() -> ObjectPrint {
    let hull = ring(&[(10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    ObjectPrint {
        silhouette: vec![hull.clone()],
        part_hulls: vec![hull.clone()],
        part_boxes: vec![Rect::new(0.0, 0.0, 10.0, 10.0)],
        landing: vec![hull],
        ..ObjectPrint::default()
    }
}

fn rules(band: f64, reserve: f64, keepouts: Vec<Keepout>) -> PlateRules {
    PlateRules {
        printable_area: Rect::new(0.0, 0.0, 256.0, 256.0),
        body_edge_band_mm: band,
        print_edge_reserve_mm: reserve,
        support_clip_mm: 0.0,
        support_clip_sides: [true; 4],
        keepouts,
        body_rule: BodyRule::Silhouette,
    }
}

fn zone(id: &str, class: KeepoutClass, shape: KeepoutShape, z: Option<[f64; 2]>) -> Keepout {
    Keepout {
        id: id.to_owned(),
        class,
        shape,
        z_range_mm: z,
        frame: KeepoutFrame::Plate,
    }
}

fn hit(zone: &str, kind: PrintKind, band: Option<usize>) -> ZoneHit {
    ZoneHit {
        zone: zone.to_owned(),
        kind,
        band,
    }
}

#[test]
fn a_soft_strip_blocks_only_the_landing() {
    // A P1S-like load line in the plate frame: y = 3 ± 0.405, first layer only.
    let purge = zone(
        "purge",
        KeepoutClass::Soft,
        KeepoutShape::Strips(vec![[18.0, 3.0, 240.0, 3.0, 0.81]]),
        Some([0.0, 0.2]),
    );
    let plate = rules(0.0, 0.0, vec![purge]);
    // Support base and supports lie over the strip, the landing clears it: allowed.
    assert_eq!(zone_hits(&plate, &cube(), (100.0, 4.0)), vec![]);
    // The landing touching the strip's edge is still allowed.
    assert_eq!(zone_hits(&plate, &cube(), (100.0, 3.0 + 0.405)), vec![]);
    // The landing on the strip: one hit, the landing (the body above it is allowed).
    assert_eq!(
        zone_hits(&plate, &cube(), (100.0, 3.2)),
        vec![hit("purge", PrintKind::Landing, Some(0))]
    );
}

#[test]
fn a_purge_strip_holds_half_a_width_past_its_ends() {
    let purge = zone(
        "purge",
        KeepoutClass::Soft,
        KeepoutShape::Strips(vec![[50.0, 30.0, 150.0, 30.0, 0.8]]),
        None,
    );
    let plate = rules(0.0, 0.0, vec![purge]);
    assert_eq!(zone_hits(&plate, &cube(), (150.4, 20.0)), vec![]);
    assert_eq!(
        zone_hits(&plate, &cube(), (150.2, 20.0)),
        vec![hit("purge", PrintKind::Landing, Some(0))]
    );
}

#[test]
fn a_soft_zone_above_the_first_layer_does_not_meet_the_landing() {
    let raised = zone(
        "raised",
        KeepoutClass::Soft,
        KeepoutShape::Rect(Rect::new(90.0, 0.0, 130.0, 40.0)),
        Some([4.0, 6.0]),
    );
    assert_eq!(
        zone_hits(&rules(0.0, 0.0, vec![raised]), &cube(), (100.0, 10.0)),
        vec![]
    );
}

#[test]
fn the_restricted_body_band_tests_part_boxes_with_equality_passing() {
    let plate = rules(3.5, 0.0, vec![]);
    // Box exactly 3.5 mm from the left and bottom edges; the support base reaches 0.5 mm.
    assert_eq!(zone_hits(&plate, &cube(), (3.5, 3.5)), vec![]);
    assert_eq!(
        zone_hits(&plate, &cube(), (3.49, 3.5)),
        vec![hit("body_edge_band", PrintKind::Silhouette, None)]
    );
    // Right and top edges: box max exactly at 256 - 3.5.
    assert_eq!(zone_hits(&plate, &cube(), (232.5, 232.5)), vec![]);
    assert_eq!(
        zone_hits(&plate, &cube(), (232.5, 232.51)),
        vec![hit("body_edge_band", PrintKind::Silhouette, None)]
    );
}

#[test]
fn a_restricted_rect_keeps_out_part_boxes_but_not_supports() {
    let tower = zone(
        "tower",
        KeepoutClass::Restricted,
        KeepoutShape::Rect(Rect::new(100.0, 100.0, 125.0, 125.0)),
        None,
    );
    let plate = rules(0.0, 0.0, vec![tower]);
    // Box touching the rect's left side, supports inside it: allowed.
    assert_eq!(zone_hits(&plate, &cube(), (80.0, 100.0)), vec![]);
    assert_eq!(
        zone_hits(&plate, &cube(), (80.01, 100.0)),
        vec![hit("tower", PrintKind::Silhouette, None)]
    );
}

#[test]
fn a_hard_zone_tests_part_hulls_and_touching_passes() {
    let cutter = zone(
        "cutter",
        KeepoutClass::Hard,
        KeepoutShape::Rect(Rect::new(0.0, 0.0, 18.0, 28.0)),
        None,
    );
    let plate = rules(0.0, 0.0, vec![cutter]);
    // The wedge's box overlaps the cutter's corner, its hull only touches (18, 28): allowed.
    assert_eq!(zone_hits(&plate, &wedge(), (15.0, 21.0)), vec![]);
    assert_eq!(
        zone_hits(&plate, &wedge(), (14.9, 21.0)),
        vec![hit("cutter", PrintKind::Silhouette, None)]
    );
    // A square part touching the cutter's right side passes, a hair inside does not.
    let square_part = ObjectPrint {
        support_base: Vec::new(),
        support: Vec::new(),
        ..cube()
    };
    assert_eq!(zone_hits(&plate, &square_part, (18.0, 5.0)), vec![]);
    assert_eq!(
        zone_hits(&plate, &square_part, (17.99, 5.0)),
        vec![hit("cutter", PrintKind::Silhouette, None)]
    );
}

#[test]
fn a_hard_zone_keeps_every_support_out() {
    let cutter = zone(
        "cutter",
        KeepoutClass::Hard,
        KeepoutShape::Rect(Rect::new(0.0, 0.0, 18.0, 28.0)),
        None,
    );
    // The body clears the cutter by 1.5 mm, the supports reach into it.
    let hits = zone_hits(&rules(0.0, 0.0, vec![cutter]), &cube(), (19.5, 5.0));
    assert_eq!(
        hits,
        vec![
            hit("cutter", PrintKind::SupportBase, Some(0)),
            hit("cutter", PrintKind::SupportBand, Some(0)),
        ]
    );
    // A hard edge band (a macro purge) does the same along every edge.
    let band = zone(
        "macro_purge",
        KeepoutClass::Hard,
        KeepoutShape::EdgeBand(5.0),
        None,
    );
    let hits = zone_hits(&rules(0.0, 0.0, vec![band]), &cube(), (6.0, 100.0));
    assert_eq!(
        hits,
        vec![
            hit("macro_purge", PrintKind::SupportBase, Some(0)),
            hit("macro_purge", PrintKind::SupportBand, Some(0)),
        ]
    );
}

#[test]
fn a_support_clip_lets_support_cross_the_edge_but_not_the_body() {
    let clipped = |clip: f64| PlateRules {
        support_clip_mm: clip,
        support_clip_sides: [true; 4],
        ..rules(0.0, 0.0, vec![])
    };
    // The base reaches 3 mm past the body: body on the edge, support 3 mm over it.
    assert_eq!(
        zone_hits(&clipped(0.0), &cube(), (3.0, 100.0)),
        vec![]
    );
    assert_eq!(
        zone_hits(&clipped(0.0), &cube(), (1.0, 100.0)),
        vec![
            hit("printable_area", PrintKind::SupportBase, Some(0)),
            hit("printable_area", PrintKind::SupportBand, Some(0)),
        ]
    );
    // With a 2 mm clip the base may hang 2 mm over (origin 1), not 3 mm (origin 0).
    assert_eq!(zone_hits(&clipped(2.0), &cube(), (1.0, 100.0)), vec![]);
    assert_eq!(
        zone_hits(&clipped(2.0), &cube(), (0.0, 100.0)),
        vec![hit("printable_area", PrintKind::SupportBase, Some(0))]
    );
    // A body past the edge is never excused by the clip.
    assert_eq!(
        zone_hits(&clipped(4.0), &cube(), (-1.0, 100.0)),
        vec![hit("printable_area", PrintKind::Silhouette, None)]
    );
    // The reserve and the clip add: reserve 0.5, clip 2 lets the base reach 1.5 over.
    let both = PlateRules {
        print_edge_reserve_mm: 0.5,
        support_clip_mm: 2.0,
        support_clip_sides: [true; 4],
        ..rules(0.0, 0.0, vec![])
    };
    assert_eq!(zone_hits(&both, &cube(), (1.5, 100.0)), vec![]);
    assert_eq!(
        zone_hits(&both, &cube(), (1.0, 100.0)),
        vec![hit("printable_area", PrintKind::SupportBase, Some(0))]
    );
}

#[test]
fn supports_stay_inside_the_printable_area_less_the_reserve() {
    // Support base exactly on the edge: allowed without a reserve, not with one.
    assert_eq!(
        zone_hits(&rules(0.0, 0.0, vec![]), &cube(), (3.0, 100.0)),
        vec![]
    );
    assert_eq!(
        zone_hits(&rules(0.0, 0.5, vec![]), &cube(), (3.0, 100.0)),
        vec![hit("printable_area", PrintKind::SupportBase, Some(0))]
    );
    // Part boxes get the GUIs' 1e-4 mm tolerance at the printable edge, supports none.
    let bare = ObjectPrint {
        support_base: Vec::new(),
        support: Vec::new(),
        ..cube()
    };
    assert_eq!(
        zone_hits(&rules(0.0, 0.0, vec![]), &bare, (-5e-5, 100.0)),
        vec![]
    );
    assert_eq!(
        zone_hits(&rules(0.0, 0.0, vec![]), &bare, (-2e-4, 100.0)),
        vec![hit("printable_area", PrintKind::Silhouette, None)]
    );
}

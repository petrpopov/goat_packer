//! The permission matrix and the pair table (design 2026-11-02, §7.2 and §7.3) pinned cell by
//! cell, plus the plate rules around them: a profile's bed rules (`profile_rules`) and what
//! each zone class allows on placed objects (`zone_rules`).

#[path = "keepouts/profile_rules.rs"]
mod profile_rules;
#[path = "keepouts/zone_rules.rs"]
mod zone_rules;

use goat_packer::keepout::{
    BodyRule, Gap, Keepout, KeepoutFrame, KeepoutShape, Permission, PlateRules, ZoneClass,
    pair_gap, pair_rules, permission,
};
use goat_packer::{ForeignGaps, KeepoutClass, PackOptions, Point, PrintKind, Rect, SUPPORT_BODY_GAP_MM};

use Gap::{Clearance, Support, Zero};
use Permission::{Allowed, Forbidden, PartBox, PartHull};
use PrintKind::*;

fn forbidden(gap: Gap) -> Permission {
    Forbidden { gap }
}

/// §7.2 row by row, columns: boundary, hard, restricted, soft, tower body, tower brim, tower
/// brim the GUI checks.
#[test]
fn matrix_pins_every_cell_of_the_design_table() {
    let table = [
        // Bodies: boxes inside the bed, hulls clear of hard zones, boxes off restricted bands.
        (
            Silhouette,
            [
                PartBox,
                PartHull,
                PartBox,
                Allowed,
                forbidden(Clearance),
                forbidden(Clearance),
                forbidden(Clearance),
            ],
        ),
        (
            BodyBand,
            [
                PartBox,
                PartHull,
                PartBox,
                Allowed,
                forbidden(Clearance),
                forbidden(Clearance),
                forbidden(Clearance),
            ],
        ),
        // The landing is only checked on its own: soft zones, and the tower.
        (
            Landing,
            [
                Allowed,
                Allowed,
                Allowed,
                forbidden(Zero),
                forbidden(Clearance),
                forbidden(Clearance),
                forbidden(Clearance),
            ],
        ),
        (Brim, [Allowed; 7]),
        // Supports: inside the bed, off hard zones, away from the tower body, over its brim unless the GUI checks it.
        (
            SupportBase,
            [
                forbidden(Zero),
                forbidden(Zero),
                Allowed,
                Allowed,
                forbidden(Support),
                Allowed,
                forbidden(Zero),
            ],
        ),
        (
            SupportBand,
            [
                forbidden(Zero),
                forbidden(Zero),
                Allowed,
                Allowed,
                forbidden(Support),
                Allowed,
                forbidden(Zero),
            ],
        ),
        (
            InterfaceBand,
            [
                forbidden(Zero),
                forbidden(Zero),
                Allowed,
                Allowed,
                forbidden(Support),
                Allowed,
                forbidden(Zero),
            ],
        ),
        (
            TowerBody,
            [
                forbidden(Zero),
                forbidden(Zero),
                Allowed,
                forbidden(Zero),
                Allowed,
                Allowed,
                Allowed,
            ],
        ),
        (
            TowerBrim,
            [
                forbidden(Zero),
                forbidden(Zero),
                Allowed,
                Allowed,
                Allowed,
                Allowed,
                Allowed,
            ],
        ),
    ];
    assert_eq!(table.len(), PrintKind::ALL.len(), "one row per kind");
    for (kind, expected) in table {
        assert_eq!(expected.len(), ZoneClass::ALL.len());
        for (zone, want) in ZoneClass::ALL.into_iter().zip(expected) {
            assert_eq!(permission(kind, zone), want, "{kind:?} against {zone:?}");
        }
    }
}

#[test]
fn profile_classes_map_to_their_matrix_columns() {
    assert_eq!(ZoneClass::from(KeepoutClass::Hard), ZoneClass::Hard);
    assert_eq!(
        ZoneClass::from(KeepoutClass::Restricted),
        ZoneClass::Restricted
    );
    assert_eq!(ZoneClass::from(KeepoutClass::Soft), ZoneClass::Soft);
}

#[test]
fn gaps_resolve_to_the_job_clearance_and_the_support_gap() {
    assert_eq!(Zero.mm(0.7, ForeignGaps::default()), 0.0);
    assert_eq!(Clearance.mm(0.7, ForeignGaps::default()), 0.7);
    assert_eq!(Support.mm(0.7, ForeignGaps::default()), SUPPORT_BODY_GAP_MM);
    assert_eq!(SUPPORT_BODY_GAP_MM, 0.5);
    // A support base may touch a neighbour's landing (printed-OK plates).
    assert_eq!(Gap::SupportBase.mm(0.7, ForeignGaps::default()), 0.0);
}

/// §7.3: forbidden pairs with their gap, both directions.
#[test]
fn pair_table_forbids_exactly_the_listed_pairs() {
    let forbidden_pairs = [
        (Silhouette, Silhouette, Clearance),
        (SupportBase, Landing, Gap::SupportBase),
        (SupportBand, BodyBand, Support),
        (InterfaceBand, BodyBand, Support),
    ];
    for (a, b, gap) in forbidden_pairs {
        assert_eq!(
            pair_gap(a, b, BodyRule::Silhouette),
            Some(gap),
            "{a:?} / {b:?}"
        );
        assert_eq!(
            pair_gap(b, a, BodyRule::Silhouette),
            Some(gap),
            "{b:?} / {a:?}"
        );
    }
    assert_eq!(
        pair_rules(BodyRule::Silhouette).len(),
        forbidden_pairs.len()
    );
}

#[test]
fn pairs_not_in_the_table_are_allowed() {
    let supports = [SupportBase, SupportBand, InterfaceBand];
    for a in supports {
        for b in supports {
            assert_eq!(
                pair_gap(a, b, BodyRule::Silhouette),
                None,
                "supports overlap: {a:?} / {b:?}"
            );
        }
    }
    for kind in PrintKind::ALL {
        assert_eq!(
            pair_gap(Brim, kind, BodyRule::Silhouette),
            None,
            "brim / {kind:?}"
        );
        assert_eq!(
            pair_gap(Brim, kind, BodyRule::Bands),
            None,
            "brim / {kind:?}"
        );
    }
    // A body does not need to clear the neighbour's landing separately: the silhouette rule
    // covers it, and the tower is a zone, not a pair.
    assert_eq!(pair_gap(Silhouette, Landing, BodyRule::Silhouette), None);
    assert_eq!(pair_gap(TowerBody, Silhouette, BodyRule::Silhouette), None);
}

/// Q14: per-band body against body replaces the silhouette pair and nothing else.
#[test]
fn band_body_rule_swaps_only_the_body_pair() {
    assert_eq!(BodyRule::default(), BodyRule::Silhouette);
    assert_eq!(
        pair_gap(BodyBand, BodyBand, BodyRule::Bands),
        Some(Clearance)
    );
    assert_eq!(pair_gap(Silhouette, Silhouette, BodyRule::Bands), None);
    assert_eq!(pair_gap(BodyBand, BodyBand, BodyRule::Silhouette), None);
    let kept = [
        (SupportBase, Landing),
        (SupportBand, BodyBand),
        (InterfaceBand, BodyBand),
    ];
    for (a, b) in kept {
        assert_eq!(
            pair_gap(a, b, BodyRule::Bands),
            pair_gap(a, b, BodyRule::Silhouette),
            "{a:?} / {b:?}"
        );
    }
}

fn keepout(z_range_mm: Option<[f64; 2]>) -> Keepout {
    Keepout {
        id: "purge".to_owned(),
        class: KeepoutClass::Soft,
        shape: KeepoutShape::Strips(vec![[18.0, 1.0, 240.0, 1.0, 0.81]]),
        z_range_mm,
        frame: KeepoutFrame::Gcode,
    }
}

/// A zone is tested only in the bands its z range meets; a zone ending exactly at a band
/// start still holds the layer printed at that z.
#[test]
fn zone_z_range_selects_the_bands_it_is_tested_in() {
    let first_layer_only = keepout(Some([0.0, 0.2]));
    assert!(first_layer_only.meets_band(0));
    assert!(!first_layer_only.meets_band(1));

    let every_height = keepout(None);
    assert!((0..40).all(|band| every_height.meets_band(band)));

    let upper = keepout(Some([4.0, 6.0]));
    assert!(!upper.meets_band(1));
    assert!(upper.meets_band(2));
    assert!(upper.meets_band(3), "z = 6.0 is the first layer of band 3");
    assert!(!upper.meets_band(4));
}

fn options(body_bed: Option<Rect>) -> PackOptions {
    PackOptions {
        bed: Rect::new(0.5, 1.0, 270.5, 271.0),
        body_bed,
        exclusions: vec![Rect::new(0.0, 0.0, 22.0, 32.0)],
        body_exclusions: vec![Rect::new(100.0, 100.0, 125.0, 125.0)],
        resolution: 0.25,
        clearance: 0.7,
        angle_step_deg: 90.0,
        attempts: 1,
        candidates_per_step: 1,
        stall_attempts: 0,
        max_objects: 0,
        lock_orientation: false,
        seed: 1,
        support_overlap_mm: None,
        foreign_gaps: Default::default(),
        plate_rules: None,
    }
}

/// The legacy fields keep their meaning: the bed is the area, the inset of `body_bed` is the
/// body band, exclusions are hard and the tower keepout (body only) is restricted.
#[test]
fn legacy_bed_fields_convert_to_plate_rules() {
    let body_bed = Rect::new(4.5, 5.0, 266.5, 267.0);
    let rules = options(Some(body_bed)).effective_plate_rules();

    assert_eq!(rules.printable_area, Rect::new(0.5, 1.0, 270.5, 271.0));
    assert_eq!(rules.body_edge_band_mm, 4.0);
    assert_eq!(rules.print_edge_reserve_mm, 0.0);
    assert_eq!(rules.body_rule, BodyRule::Silhouette);
    assert_eq!(rules.keepouts.len(), 2);
    assert_eq!(rules.keepouts[0].class, KeepoutClass::Hard);
    assert_eq!(
        rules.keepouts[0].shape,
        KeepoutShape::Rect(Rect::new(0.0, 0.0, 22.0, 32.0))
    );
    assert_eq!(rules.keepouts[1].class, KeepoutClass::Restricted);
    assert_eq!(
        rules.keepouts[1].shape,
        KeepoutShape::Rect(Rect::new(100.0, 100.0, 125.0, 125.0))
    );
    assert_ne!(rules.keepouts[0].id, rules.keepouts[1].id);
    assert!(
        rules
            .keepouts
            .iter()
            .all(|k| k.z_range_mm.is_none() && k.frame == KeepoutFrame::Plate)
    );
}

#[test]
fn legacy_conversion_without_a_body_bed_has_no_edge_band() {
    assert_eq!(options(None).effective_plate_rules().body_edge_band_mm, 0.0);
}

/// An uneven body inset cannot be one band: the widest side wins, which is the safe side.
#[test]
fn legacy_conversion_takes_the_widest_body_inset() {
    // Insets: left 2, bottom 0, right 0, top 4.
    let uneven = Rect::new(2.5, 1.0, 270.5, 267.0);
    assert_eq!(
        options(Some(uneven))
            .effective_plate_rules()
            .body_edge_band_mm,
        4.0
    );
}

#[test]
fn explicit_plate_rules_win_over_the_legacy_fields() {
    let mut with_rules = options(None);
    let explicit = PlateRules {
        printable_area: Rect::new(0.0, 0.0, 256.0, 256.0),
        body_edge_band_mm: 3.5,
        print_edge_reserve_mm: 0.0,
        keepouts: Vec::new(),
        body_rule: BodyRule::Bands,
    };
    with_rules.plate_rules = Some(explicit.clone());
    assert_eq!(with_rules.effective_plate_rules(), explicit);
}

#[test]
fn plate_rules_survive_a_state_file_and_default_missing_fields() {
    let rules = PlateRules {
        printable_area: Rect::new(0.0, 0.0, 256.0, 256.0),
        body_edge_band_mm: 3.5,
        print_edge_reserve_mm: 0.7,
        keepouts: vec![
            keepout(Some([0.0, 0.2])),
            Keepout {
                id: "macro_purge".to_owned(),
                class: KeepoutClass::Hard,
                shape: KeepoutShape::EdgeBand(5.0),
                z_range_mm: None,
                frame: KeepoutFrame::Plate,
            },
            Keepout {
                id: "cutter".to_owned(),
                class: KeepoutClass::Hard,
                shape: KeepoutShape::Polygon(vec![
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 18.0, y: 0.0 },
                    Point { x: 18.0, y: 28.0 },
                    Point { x: 0.0, y: 0.0 },
                ]),
                z_range_mm: None,
                frame: KeepoutFrame::Plate,
            },
        ],
        body_rule: BodyRule::Bands,
    };
    let json = serde_json::to_string(&rules).unwrap();
    assert_eq!(serde_json::from_str::<PlateRules>(&json).unwrap(), rules);

    let minimal = r#"{"id": "cutter", "class": "hard", "shape": {"rect": {"min_x": 0.0, "min_y": 0.0, "max_x": 18.0, "max_y": 28.0}}}"#;
    let keepout: Keepout = serde_json::from_str(minimal).unwrap();
    assert_eq!(keepout.frame, KeepoutFrame::Plate);
    assert_eq!(keepout.z_range_mm, None);
}

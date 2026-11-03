//! `ForeignGaps` (support base to a neighbour's landing, support above layer 0 to a neighbour's
//! body) change what the checks and the search admit; the defaults are the former constants.
//! Generated prints only.

use goat_packer::keepout::{BodyRule, PlateRules};
use goat_packer::pack::pack;
use goat_packer::{
    ForeignGaps, LayoutRules, ObjectPrint, PackOptions, Point, PrintPlacement, Rect,
    validate_raster, validate_vector,
};
use goat_packer::kind_masks::OriginGrid;

const CLEARANCE: f64 = 0.7;

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Vec<Point>> {
    let p = |x, y| Point { x, y };
    vec![vec![p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1), p(x0, y0)]]
}

/// A 2 x 2 mm body with a 2 x 2 mm support to its right (x 1..3), as a first-layer base or as
/// a band-0 support. The opposite neighbour's body is `d - 4` mm from it.
fn print(base: bool) -> ObjectPrint {
    let body = rect(-1.0, -1.0, 1.0, 1.0);
    let patch = rect(1.0, -1.0, 3.0, 1.0);
    ObjectPrint {
        silhouette: body.clone(),
        part_hulls: body.clone(),
        part_boxes: vec![Rect::new(-1.0, -1.0, 1.0, 1.0)],
        landing: body.clone(),
        support_base: if base { patch.clone() } else { Vec::new() },
        body: vec![body],
        support: vec![if base { Vec::new() } else { patch }],
        interface: vec![Vec::new()],
        ..ObjectPrint::default()
    }
}

fn plate() -> PlateRules {
    PlateRules {
        printable_area: Rect::new(0.0, 0.0, 100.0, 100.0),
        body_edge_band_mm: 0.0,
        print_edge_reserve_mm: 0.0,
        keepouts: Vec::new(),
        body_rule: BodyRule::Silhouette,
    }
}

fn rules(foreign: ForeignGaps) -> LayoutRules {
    LayoutRules::new(&plate(), None, CLEARANCE, foreign, None).unwrap()
}

fn pair(distance: f64) -> Vec<PrintPlacement> {
    [(20.0, 50.0), (20.0 + distance, 50.0)]
        .map(|(x_mm, y_mm)| PrintPlacement { print: 0, x_mm, y_mm })
        .to_vec()
}

fn grid() -> OriginGrid {
    OriginGrid { origin: Point { x: 0.0, y: 0.0 }, resolution: 0.25 }
}

/// The pair at `4 + gap` passes, 0.05 mm closer fails, for either check.
fn assert_threshold(print: &ObjectPrint, foreign: ForeignGaps, gap: f64) {
    let rules = rules(foreign);
    let prints = std::slice::from_ref(print);
    assert!(validate_vector(prints, &pair(4.0 + gap), &rules).is_ok(), "vector {foreign:?}");
    assert!(validate_vector(prints, &pair(4.0 + gap - 0.05), &rules).is_err(), "vector {foreign:?}");
    // The raster check covers conservatively: a margin of a few pixels passes, a contact fails.
    assert!(validate_raster(prints, &pair(4.0 + gap + 0.5), &rules, grid()).is_ok(), "raster {foreign:?}");
    assert!(validate_raster(prints, &pair(4.0 + gap - 0.05), &rules, grid()).is_err(), "raster {foreign:?}");
}

#[test]
fn the_defaults_are_the_former_constants() {
    let defaults = ForeignGaps::default();
    assert_eq!(defaults.base_mm, 0.0);
    assert_eq!(defaults.band_mm, goat_packer::SUPPORT_BODY_GAP_MM);
    assert_eq!(defaults.band_mm, 0.5);
}

#[test]
fn the_base_gap_sets_how_close_a_support_base_may_come_to_a_neighbours_landing() {
    for base_mm in [0.0, 0.3, 1.0] {
        assert_threshold(&print(true), ForeignGaps { base_mm, band_mm: 0.5 }, base_mm);
    }
}

#[test]
fn the_band_gap_sets_how_close_a_support_may_come_to_a_neighbours_body() {
    for band_mm in [0.25, 0.5, 1.0] {
        assert_threshold(&print(false), ForeignGaps { base_mm: 0.0, band_mm }, band_mm);
    }
}

#[test]
fn a_base_gap_does_not_touch_the_band_rule_and_the_other_way_round() {
    // Bands only: the base gap is irrelevant, and vice versa.
    let strict_base = ForeignGaps { base_mm: 2.0, band_mm: 0.25 };
    let rules_ok = rules(strict_base);
    assert!(validate_vector(&[print(false)], &pair(4.25), &rules_ok).is_ok());
    let strict_band = ForeignGaps { base_mm: 0.0, band_mm: 2.0 };
    assert!(validate_vector(&[print(true)], &pair(4.0), &rules(strict_band)).is_ok());
}

#[test]
fn invalid_gaps_are_refused() {
    for bad in [-0.1, f64::NAN, f64::INFINITY] {
        assert!(LayoutRules::new(&plate(), None, CLEARANCE, ForeignGaps { base_mm: bad, band_mm: 0.5 }, None).is_err());
        assert!(LayoutRules::new(&plate(), None, CLEARANCE, ForeignGaps { base_mm: 0.0, band_mm: bad }, None).is_err());
    }
}

fn options(foreign_gaps: ForeignGaps) -> PackOptions {
    PackOptions {
        bed: Rect::new(0.0, 0.0, 100.0, 100.0),
        body_bed: None,
        exclusions: Vec::new(),
        body_exclusions: Vec::new(),
        resolution: 0.25,
        resolutions: Vec::new(),
        clearance: CLEARANCE,
        angle_step_deg: 90.0,
        attempts: 8,
        candidates_per_step: 0,
        stall_attempts: 0,
        max_objects: 0,
        lock_orientation: true,
        seed: 0,
        support_overlap_mm: None,
        foreign_gaps,
        plate_rules: None,
    }
}

#[test]
fn the_search_packs_by_the_gaps_it_is_given() {
    let count = |foreign: ForeignGaps| {
        let outcome = pack(&[print(true)], &plate(), None, &options(foreign)).unwrap();
        // The published layout passes the exact check under the same gaps.
        validate_vector(&[print(true)], &outcome.placements, &rules(foreign)).unwrap();
        outcome.placements.len()
    };
    let loose = count(ForeignGaps { base_mm: 0.0, band_mm: 0.5 });
    let strict = count(ForeignGaps { base_mm: 3.0, band_mm: 0.5 });
    assert!(loose > strict, "loose {loose}, strict {strict}");
    // The default options mean the explicit former values.
    assert_eq!(count(ForeignGaps::default()), loose);
}

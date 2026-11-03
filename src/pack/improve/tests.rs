//! Improvement on synthetic boxes: a 10 x 6 mm box on a 0.5 mm grid with 0.5 mm clearance. The
//! conservative raster adds a pixel on each side, so a row of boxes has a pitch of 22 pixels
//! (11 mm) and a column of 14; a box origin sits at its centre.

use super::*;
use crate::geometry::{Point, Rect};
use crate::keepout::PlateRules;
use crate::kind_masks::{KindShapes, MaskRules, OriginGrid, PlateZone, plate_zones};
use crate::object_print::ObjectPrint;
use crate::pack::robustness::layout_slack;

const RESOLUTION: f64 = 0.5;

fn box_print(w: f64, h: f64) -> ObjectPrint {
    let ring: Vec<Point> = [(-w, -h), (w, -h), (w, h), (-w, h), (-w, -h)]
        .iter()
        .map(|&(x, y)| Point {
            x: x / 2.0,
            y: y / 2.0,
        })
        .collect();
    ObjectPrint {
        silhouette: vec![ring.clone()],
        part_hulls: vec![ring.clone()],
        part_boxes: vec![Rect::of_points(ring.iter())],
        landing: vec![ring.clone()],
        support_base: vec![ring.clone()],
        body: vec![vec![ring]],
        support: vec![vec![]],
        interface: vec![vec![]],
        ..ObjectPrint::default()
    }
}

fn zones(width: f64, height: f64) -> Vec<PlateZone> {
    let rules = PlateRules {
        printable_area: Rect::new(0.0, 0.0, width, height),
        body_edge_band_mm: 0.0,
        print_edge_reserve_mm: 0.0,
        keepouts: Vec::new(),
        body_rule: Default::default(),
    };
    plate_zones(&rules, None).unwrap()
}

/// One class of 10 x 6 boxes on a plate of `zones`.
fn context(zones: &[PlateZone]) -> Ctx<'_> {
    let grid = OriginGrid {
        origin: Point { x: 0.0, y: 0.0 },
        resolution: RESOLUTION,
    };
    let rules = MaskRules {
        clearance_mm: 0.5,
        foreign: Default::default(),
        body_rule: Default::default(),
        support_overlap_mm: None,
    };
    let masks = vec![KindShapes::new(&box_print(10.0, 6.0), rules).rasterize(RESOLUTION)];
    Ctx::new(masks, vec![0], zones, grid)
}

fn row(xs: &[i32], y: i32) -> Vec<PixelPlacement> {
    xs.iter()
        .map(|&x| PixelPlacement { class: 0, x, y })
        .collect()
}

fn seed(ctx: &Ctx, layout: &[PixelPlacement]) -> Candidate {
    ctx.candidate(Member::Lattice, None, vec![0], layout)
}

fn assert_legal(ctx: &Ctx, layout: &[PixelPlacement]) {
    for (i, a) in layout.iter().enumerate() {
        assert!(
            ctx.origins(a.class).contains(a.x, a.y),
            "{a:?} on the plate"
        );
        for b in &layout[i + 1..] {
            let nfp = ctx.no_fit(a.class, b.class);
            assert!(!nfp.contains(b.x - a.x, b.y - a.y), "{a:?} overlaps {b:?}");
        }
    }
}

/// A 44 x 7 mm strip holds four boxes (origins 10..78 px, pitch 22), but the seed spreads
/// three of them out.
fn strip() -> Vec<PlateZone> {
    zones(44.0, 7.0)
}

#[test]
fn a_spread_row_gets_one_more_box() {
    let zones = strip();
    let ctx = context(&zones);
    let spread = row(&[10, 44, 78], 6);
    let improved = improve_candidates(&ctx, &[seed(&ctx, &spread)], 64, 0);
    let best = improved.iter().map(|c| c.key.count).max().unwrap_or(0);
    assert_eq!(best, 4, "the gap at the right edge is worth a box");
    for c in &improved {
        assert_eq!(c.key.tie.member, Member::Improve);
        assert_eq!(c.key.tie.detail[..2], [Member::Lattice as i64, 3]);
        assert_legal(&ctx, &c.placements);
    }
    assert_eq!(improved_counts(&improved)[0].count, 4);
}

#[test]
fn a_full_layout_comes_back_unchanged_and_the_cap_is_respected() {
    let zones = strip();
    let ctx = context(&zones);
    let full = row(&[10, 32, 54, 76], 6);
    assert!(improve_candidates(&ctx, &[seed(&ctx, &full)], 64, 0).is_empty());
    let spread = row(&[10, 44, 78], 6);
    assert!(improve_candidates(&ctx, &[seed(&ctx, &spread)], 64, 3).is_empty());
}

#[test]
fn growing_never_loses_a_copy_whatever_the_budget() {
    let zones = zones(60.0, 30.0);
    let ctx = context(&zones);
    let sparse = [
        (10, 6),
        (50, 6),
        (90, 6),
        (30, 30),
        (70, 30),
        (10, 54),
        (60, 54),
    ];
    let layout: Vec<PixelPlacement> = sparse
        .iter()
        .map(|&(x, y)| PixelPlacement { class: 0, x, y })
        .collect();
    assert_legal(&ctx, &layout);
    let depth = DepthTable::new(&ctx);
    let free = vec![ctx.free_map(0, None)];
    let field = Field {
        depth: &depth,
        free: &free,
    };
    let mut last = 0;
    for tries in [0, 1, 5, 40] {
        let grown = grow::grow(&ctx, &field, None, &layout, tries, 0);
        assert!(grown.len() >= layout.len(), "{tries} tries");
        assert!(grown.len() >= last, "more tries never lose a result");
        assert_legal(&ctx, &grown);
        last = grown.len();
    }
    assert!(last > layout.len(), "the sparse plate has room");
}

#[test]
fn effort_only_extends_the_tries() {
    let tries: Vec<usize> = (1..=70).map(tries_for).collect();
    assert!(tries.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(tries[0], 2);
    assert_eq!(tries[63], FULL_TRIES);
    assert_eq!(tries[69], FULL_TRIES);
}

#[test]
fn threads_do_not_change_the_result() {
    let zones = strip();
    let ctx = context(&zones);
    let sources = [
        seed(&ctx, &row(&[10, 44, 78], 6)),
        seed(&ctx, &row(&[10, 32, 60], 7)),
    ];
    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let improved = pool.install(|| improve_candidates(&ctx, &sources, 64, 0));
        improved
            .iter()
            .map(|c| (c.key.clone(), c.placements.clone()))
            .collect::<Vec<_>>()
    };
    let (one, eight) = (run(1), run(8));
    assert!(!one.is_empty());
    assert_eq!(one.len(), eight.len());
    for (a, b) in one.iter().zip(&eight) {
        assert_eq!(a.0, b.0);
        assert_eq!(a.1, b.1);
    }
}

#[test]
fn spare_room_becomes_slack() {
    let zones = strip();
    let ctx = context(&zones);
    // Three boxes touching at the pitch, 22 pixels of room at the right.
    let tight = row(&[10, 32, 54], 6);
    let before = layout_slack(&ctx, None, &tight);
    assert_eq!(before, 0);
    let source = seed(&ctx, &tight);
    let polished = polish_candidates(&ctx, &[source]);
    assert_eq!(polished.len(), 1);
    let c = &polished[0];
    assert_eq!(c.key.count, 3);
    assert_eq!(c.key.tie.detail[..3], [Member::Lattice as i64, 3, -1]);
    assert_legal(&ctx, &c.placements);
    let slack = layout_slack(&ctx, None, &c.placements);
    assert_eq!(slack, crate::pack::robustness::cap_px(RESOLUTION));
    assert!(c.key.robustness_quarter_mm > 0);
    assert!(improved_counts(&polished).is_empty(), "no gain in count");
}

#[test]
fn sources_take_each_member_in_turn_and_skip_layouts_far_below_the_best() {
    let zones = strip();
    let ctx = context(&zones);
    let of = |member: Member, count: usize, detail: i64| {
        let layout = row(&vec![10; count], 6);
        ctx.candidate(member, None, vec![detail], &layout)
    };
    let candidates = vec![
        of(Member::Lattice, 10, 0),
        of(Member::Lattice, 10, 1),
        of(Member::Lattice, 10, 2),
        of(Member::Sweep, 9, 0),
        of(Member::Rows, 5, 0),
    ];
    let taken = top_sources(&candidates);
    let order: Vec<(Member, i64)> = taken
        .iter()
        .map(|c| (c.key.tie.member, c.key.tie.detail[0]))
        .collect();
    let expected = [
        (Member::Lattice, 0),
        (Member::Sweep, 0),
        (Member::Lattice, 1),
        (Member::Lattice, 2),
    ];
    assert_eq!(order, expected, "Rows is 5 below the best and is skipped");
}

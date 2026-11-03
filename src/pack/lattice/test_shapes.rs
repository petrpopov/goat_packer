//! Generated prints and a run context for the lattice members' unit tests (no real data).

use crate::geometry::{Point, Rect};
use crate::keepout::{BodyRule, PlateRules};
use crate::kind_masks::{KindShapes, MaskRules, OriginGrid, plate_zones};
use crate::layout::PixelPlacement;
use crate::object_print::ObjectPrint;
use crate::pack::context::Ctx;

/// Millimetres per pixel of the test context.
pub(super) const RESOLUTION: f64 = 0.5;

fn ring(points: &[(f64, f64)]) -> Vec<Point> {
    let mut ring: Vec<Point> = points.iter().map(|&(x, y)| Point { x, y }).collect();
    ring.push(ring[0]);
    ring
}

/// A right triangle (legs 20 and 10 mm) turned `half_turns` half turns: alone it only tiles
/// at half density with translations, its half-turn copy fills the other half.
pub(super) fn triangle(half_turns: u8) -> ObjectPrint {
    let sign = if half_turns.is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    let turned = |scale: f64| {
        let corners = [(-6.67, -3.33), (13.33, -3.33), (-6.67, 6.67)];
        ring(&corners.map(|(x, y)| (sign * scale * x, sign * scale * y)))
    };
    ObjectPrint {
        rotation_deg: 180.0 * f64::from(half_turns),
        silhouette: vec![turned(1.0)],
        part_hulls: vec![turned(1.0)],
        part_boxes: vec![Rect::of_points(turned(1.0).iter())],
        landing: vec![turned(1.0)],
        support_base: vec![turned(1.05)],
        body: vec![vec![turned(1.0)]],
        support: vec![vec![]],
        interface: vec![vec![]],
        ..ObjectPrint::default()
    }
}

/// A 60 x 40 mm plate without keepouts.
pub(super) fn plate() -> PlateRules {
    PlateRules {
        printable_area: Rect::new(0.0, 0.0, 60.0, 40.0),
        body_edge_band_mm: 0.0,
        print_edge_reserve_mm: 0.0,
        support_clip_mm: 0.0,
        support_clip_sides: [true; 4],
        keepouts: Vec::new(),
        body_rule: BodyRule::Silhouette,
    }
}

/// Runs `f` on the context of `prints` on `plate()`.
pub(super) fn with_ctx<R>(prints: &[ObjectPrint], f: impl FnOnce(&Ctx) -> R) -> R {
    let rules = plate();
    let zones = plate_zones(&rules, None).expect("plate zones");
    let grid = OriginGrid {
        origin: Point { x: 0.0, y: 0.0 },
        resolution: RESOLUTION,
    };
    let mask_rules = MaskRules {
        clearance_mm: 0.5,
        foreign: Default::default(),
        body_rule: rules.body_rule,
        support_overlap_mm: None,
    };
    let masks = prints
        .iter()
        .map(|p| KindShapes::new(p, mask_rules).rasterize(RESOLUTION))
        .collect();
    let ctx = Ctx::new(masks, (0..prints.len()).collect(), &zones, grid);
    f(&ctx)
}

/// Every placement is on a feasible origin and every pair of copies is outside the no-fit
/// set, checked pair by pair (nothing shared with the lattice arithmetic).
pub(super) fn assert_legal(ctx: &Ctx, layout: &[PixelPlacement]) {
    for (i, a) in layout.iter().enumerate() {
        assert!(
            ctx.origins(a.class).contains(a.x, a.y),
            "{a:?} off the plate"
        );
        for b in &layout[i + 1..] {
            let shift = (b.x - a.x, b.y - a.y);
            assert!(
                !ctx.no_fit(a.class, b.class).contains(shift.0, shift.1),
                "{a:?} and {b:?} collide"
            );
        }
    }
}

//! The vector stage of the masks: every kind that takes part in a pair rule, grown by half
//! its rule's gap, and the support core. Growing is the costly step, so it runs once per
//! rotation class; rasterizing (`KindShapes::rasterize`) is cheap and can follow a move.

use super::buffer::{grown, shrunk};
use crate::geometry::{Point, Rect};
use crate::keepout::{BodyRule, Gap, pair_rules};
use crate::object_print::{Loops, ObjectPrint, PrintKind};
use rayon::prelude::*;

/// What decides the masks of a job besides the print: the gaps and which pairs apply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskRules {
    /// `PackOptions::clearance`: what `Gap::Clearance` means.
    pub clearance_mm: f64,
    pub body_rule: BodyRule,
    /// `PackOptions::support_overlap_mm`: support bases of two objects may overlap this deep.
    pub support_overlap_mm: Option<f64>,
}

/// One mask of a pair rule: `kind` in absolute band `band` (0 for kinds without bands),
/// grown by half of `gap`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskKey {
    pub kind: PrintKind,
    pub band: usize,
    pub gap: Gap,
}

/// The masks of one print in vector form, in the plate-aligned frame of its rotation class.
#[derive(Clone, Debug)]
pub struct KindShapes {
    /// The print the shapes were built from, moved with them: plate zones test its kinds.
    pub print: ObjectPrint,
    pub rules: MaskRules,
    /// Every mask a pair rule needs, in `pair_rules` order, without duplicates.
    pub keys: Vec<MaskKey>,
    pub grown: Vec<Loops>,
    /// Pairs of indices into `keys`: this object's mask `.0` against another object's `.1`.
    pub slots: Vec<(usize, usize)>,
    /// The support base shrunk by half of `support_overlap_mm`; `None` without the option or
    /// without a support base (nothing to overlap).
    pub core: Option<Loops>,
    /// Material that must stay on the printable area although no kind carries it: a legacy
    /// `Footprint`'s bed extent (thin-tip overshoot, support edge reserve). Empty for prints
    /// that come from the support model.
    pub bed_extent: Loops,
}

impl KindShapes {
    pub fn new(print: &ObjectPrint, rules: MaskRules) -> KindShapes {
        assert!(rules.clearance_mm.is_finite() && rules.clearance_mm >= 0.0);
        let (keys, slots) = mask_layout(rules.body_rule, print.band_count());
        // Growing is i_overlay's outline, ~10 µs per vertex; masks are independent, and an
        // indexed parallel collect keeps their order.
        let grown = keys
            .par_iter()
            .map(|key| {
                let radius = key.gap.mm(rules.clearance_mm) * 0.5;
                grown(print.loops_of(key.kind, key.band), radius)
            })
            .collect();
        let core = rules
            .support_overlap_mm
            .filter(|_| !print.support_base.is_empty())
            .map(|overlap| {
                assert!(
                    overlap.is_finite() && overlap >= 0.0,
                    "support overlap {overlap}"
                );
                shrunk(&print.support_base, overlap * 0.5)
            });
        KindShapes {
            print: print.clone(),
            rules,
            keys,
            grown,
            slots,
            core,
            bed_extent: Loops::new(),
        }
    }

    /// The same shapes with every point passed through `transform`, which must be rigid
    /// (a translation, or a rotation of a legacy footprint whose kinds turn with the model).
    /// Part boxes are rebuilt from the moved part hulls when there is one hull per box.
    pub fn mapped(&self, transform: impl Fn(Point) -> Point + Copy) -> KindShapes {
        let map = |loops: &Loops| map_loops(loops, transform);
        KindShapes {
            print: map_print(&self.print, transform),
            rules: self.rules,
            keys: self.keys.clone(),
            grown: self.grown.iter().map(map).collect(),
            slots: self.slots.clone(),
            core: self.core.as_ref().map(map),
            bed_extent: map(&self.bed_extent),
        }
    }

    /// Bounding box of everything the print puts on the plate (all kinds, every band, the
    /// bed extent); infinite when there is nothing.
    pub fn extent_bounds(&self) -> Rect {
        let p = &self.print;
        let flat = [
            &p.silhouette,
            &p.landing,
            &p.brim,
            &p.support_base,
            &self.bed_extent,
        ];
        let banded = p.body.iter().chain(&p.support).chain(&p.interface);
        Rect::of_points(flat.into_iter().chain(banded).flatten().flatten())
    }
}

/// The mask keys the pair rules need for a print with `bands` bands, and the slots that pair
/// them across two objects (both ways for two different kinds).
fn mask_layout(body_rule: BodyRule, bands: usize) -> (Vec<MaskKey>, Vec<(usize, usize)>) {
    let mut keys: Vec<MaskKey> = Vec::new();
    let mut index_of = |key: MaskKey| match keys.iter().position(|known| *known == key) {
        Some(index) => index,
        None => {
            keys.push(key);
            keys.len() - 1
        }
    };
    let mut slots = Vec::new();
    for rule in pair_rules(body_rule) {
        let banded = is_band_kind(rule.a) || is_band_kind(rule.b);
        for band in 0..if banded { bands } else { 1 } {
            let key = |kind| MaskKey {
                kind,
                band,
                gap: rule.gap,
            };
            let (a, b) = (index_of(key(rule.a)), index_of(key(rule.b)));
            slots.push((a, b));
            if a != b {
                slots.push((b, a));
            }
        }
    }
    (keys, slots)
}

/// Kinds that exist once per absolute height band.
pub(crate) fn is_band_kind(kind: PrintKind) -> bool {
    matches!(
        kind,
        PrintKind::BodyBand | PrintKind::SupportBand | PrintKind::InterfaceBand
    )
}

fn map_loops(loops: &Loops, transform: impl Fn(Point) -> Point) -> Loops {
    loops
        .iter()
        .map(|ring| ring.iter().map(|&p| transform(p)).collect())
        .collect()
}

fn map_print(print: &ObjectPrint, transform: impl Fn(Point) -> Point + Copy) -> ObjectPrint {
    let map = |loops: &Loops| map_loops(loops, transform);
    let bands = |bands: &[Loops]| bands.iter().map(map).collect();
    let part_hulls = map(&print.part_hulls);
    let part_boxes = if part_hulls.len() == print.part_boxes.len() {
        part_hulls
            .iter()
            .map(|hull| Rect::of_points(hull.iter()))
            .collect()
    } else {
        print
            .part_boxes
            .iter()
            .map(|b| moved_box(*b, transform))
            .collect()
    };
    ObjectPrint {
        rotation_deg: print.rotation_deg,
        silhouette: map(&print.silhouette),
        part_hulls,
        part_boxes,
        landing: map(&print.landing),
        brim: map(&print.brim),
        body: bands(&print.body),
        support_base: map(&print.support_base),
        support: bands(&print.support),
        interface: bands(&print.interface),
        provenance: print.provenance.clone(),
    }
}

/// The box around a moved box's corners: a superset of the moved part's box.
fn moved_box(b: Rect, transform: impl Fn(Point) -> Point) -> Rect {
    let corners = [
        Point {
            x: b.min_x,
            y: b.min_y,
        },
        Point {
            x: b.max_x,
            y: b.min_y,
        },
        Point {
            x: b.max_x,
            y: b.max_y,
        },
        Point {
            x: b.min_x,
            y: b.max_y,
        },
    ]
    .map(transform);
    Rect::of_points(corners.iter())
}

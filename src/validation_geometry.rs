//! Vector-space publication check, independent of rasterization and no-fit maps: the exact
//! gaps of `keepout::pair_rules` between placed kinds, and the exact class rules of
//! `keepout::permission` against the plate zones (hull ∩ zone, part box distances, material
//! against strips and the tower). This validates the supplied envelope, not supports a
//! future slicer might generate. Support cores (`support_overlap_mm`) are raster-only.
//!
//! Cost model: the shapes of a rotation class are built once in the print frame (`class`)
//! and every copy is a translation of them, so no polygon is rebuilt per placement. Pairs are
//! culled by their boxes and checked in parallel, and the first conflict in placement order
//! is reported whatever the thread count.

mod class;
mod edge_grid;
mod region;
mod zones;

use crate::footprint::Footprint;
use crate::geometry::{Point, Rect, rotate_loops};
use crate::keepout::{BodyRule, ForeignGaps, pair_rules};
use crate::kind_masks::{join_sag_mm, plate_zones};
use crate::object_print::{Loops, ObjectPrint, PrintKind};
use crate::options::{PackOptions, PackResult, PackedPlacement};
use crate::validation::{
    LayoutRules, PrintPlacement, check_print_placements, check_prints, first_failure, meeting_pairs,
};
use class::{ClassShapes, rotated_print};
use rayon::prelude::*;
use region::{Region, closer_than, negated, overlaps};
use zones::{ZoneShape, breaks_zone};

/// Float slack of the exact tests, mm.
const GAP_EPS: f64 = 1e-7;

/// The exact check of layouts of fixed prints and rules: the shapes are built once, so a
/// caller re-validating (the published winner, then `PackPlan::build`) pays for them once.
pub struct VectorValidator {
    classes: Vec<ClassShapes>,
    zones: Vec<ZoneShape>,
    body_rule: BodyRule,
    clearance_mm: f64,
    foreign: ForeignGaps,
}

/// A broken pair rule: `own` of one placement too close to `foreign` of the other.
struct PairBreak {
    own: PrintKind,
    foreign: PrintKind,
    band: Option<usize>,
    gap: f64,
    /// The later placement holds `own`.
    flipped: bool,
}

impl VectorValidator {
    /// `prints[i]` is rotation class `i`, already in its plate-aligned frame (a placement is
    /// a pure translation of it).
    pub fn new(prints: &[ObjectPrint], rules: &LayoutRules) -> Result<VectorValidator, String> {
        rules.check_values()?;
        check_prints(prints)?;
        let classes = prints
            .par_iter()
            .map(|print| ClassShapes::new(print.clone(), None))
            .collect();
        Ok(VectorValidator::from_classes(classes, rules))
    }

    fn from_classes(classes: Vec<ClassShapes>, rules: &LayoutRules) -> VectorValidator {
        VectorValidator {
            classes,
            zones: rules.zones.iter().map(ZoneShape::new).collect(),
            body_rule: rules.body_rule,
            clearance_mm: rules.clearance_mm,
            foreign: rules.foreign,
        }
    }

    /// The first broken rule, by placement order: a placement's zones before its pairs with
    /// earlier placements.
    pub fn check(&self, placements: &[PrintPlacement]) -> Result<(), String> {
        check_print_placements(placements, self.classes.len())?;
        let at: Vec<Point> = placements.iter().map(PrintPlacement::origin).collect();
        let zone_failure = placements
            .par_iter()
            .enumerate()
            .find_map_first(|(index, p)| {
                let class = &self.classes[p.print];
                self.zones
                    .iter()
                    .find(|zone| breaks_zone(class, at[index], zone, self.clearance_mm, self.foreign))
                    .map(|zone| (index, zone_message(index, zone)))
            });
        // Boxes grown by half the widest gap meet whenever two copies can break a pair rule.
        let reach = self.widest_gap() * 0.5;
        let boxes: Vec<Rect> = placements
            .iter()
            .zip(&at)
            .map(|(p, &at)| self.classes[p.print].bounds_at(at).grown(reach))
            .collect();
        let pair_failure = meeting_pairs(&boxes)
            .par_iter()
            .find_map_first(|&(previous, index)| {
                let (a, b) = (&placements[previous], &placements[index]);
                let shift = Point {
                    x: at[index].x - at[previous].x,
                    y: at[index].y - at[previous].y,
                };
                self.pair_break(&self.classes[a.print], &self.classes[b.print], shift)
                    .map(|found| (index, pair_message(previous, index, &found)))
            });
        first_failure(zone_failure, pair_failure)
    }

    fn widest_gap(&self) -> f64 {
        pair_rules(self.body_rule)
            .iter()
            .map(|rule| rule.gap.mm(self.clearance_mm, self.foreign))
            .fold(0.0, f64::max)
    }

    /// The first pair rule `a` and `b` (at `shift` from `a`) break, tested both ways.
    fn pair_break(&self, a: &ClassShapes, b: &ClassShapes, shift: Point) -> Option<PairBreak> {
        for rule in pair_rules(self.body_rule) {
            let banded = is_band_kind(rule.a) || is_band_kind(rule.b);
            let bands = if banded {
                a.band_count().max(b.band_count())
            } else {
                1
            };
            let gap = rule.gap.mm(self.clearance_mm, self.foreign);
            for band in 0..bands {
                let found = |flipped| PairBreak {
                    own: rule.a,
                    foreign: rule.b,
                    band: banded.then_some(band),
                    gap,
                    flipped,
                };
                if kinds_break((a, rule.a), (b, rule.b), band, shift, gap) {
                    return Some(found(false));
                }
                let mirrored = rule.a == rule.b;
                if !mirrored && kinds_break((b, rule.a), (a, rule.b), band, negated(shift), gap) {
                    return Some(found(true));
                }
            }
        }
        None
    }
}

/// `own`'s kind against `foreign`'s kind in `band`, `foreign` at `shift` from `own`.
fn kinds_break(
    (own, own_kind): (&ClassShapes, PrintKind),
    (foreign, foreign_kind): (&ClassShapes, PrintKind),
    band: usize,
    shift: Point,
    gap: f64,
) -> bool {
    match (
        own.region(own_kind, band),
        foreign.region(foreign_kind, band),
    ) {
        (Some(a), Some(b)) => breaks_gap(a, b, shift, gap),
        _ => false,
    }
}

/// The exact layout check of the slicer-free core (design §8.5): run it on the published
/// winner only. Building a `VectorValidator` once is cheaper when the same prints are checked
/// again.
pub fn validate_vector(
    prints: &[ObjectPrint],
    placements: &[PrintPlacement],
    rules: &LayoutRules,
) -> Result<(), String> {
    VectorValidator::new(prints, rules)?.check(placements)
}

/// `a` and `b` moved by `shift` break a rule that keeps them `gap` apart. The masks draw
/// round joins as inscribed chords, so the search may place two kinds up to the chord sag
/// closer than the gap (`join_sag_mm`, under a micron): that much is tolerated.
fn breaks_gap(a: &Region, b: &Region, shift: Point, gap: f64) -> bool {
    let tolerance = GAP_EPS + join_sag_mm(gap);
    if gap > tolerance {
        closer_than(a, b, shift, gap - tolerance)
    } else {
        overlaps(a, b, shift)
    }
}

fn is_band_kind(kind: PrintKind) -> bool {
    matches!(
        kind,
        PrintKind::BodyBand | PrintKind::SupportBand | PrintKind::InterfaceBand
    )
}

fn zone_message(index: usize, zone: &ZoneShape) -> String {
    format!(
        "placement {index} breaks the {:?} zone '{}' (vector check)",
        zone.zone.class, zone.zone.id
    )
}

fn pair_message(previous: usize, index: usize, found: &PairBreak) -> String {
    let (own, foreign) = if found.flipped {
        (index, previous)
    } else {
        (previous, index)
    };
    let band = found
        .band
        .map_or(String::new(), |band| format!(" in band {band}"));
    format!(
        "placements {previous} and {index} overlap or violate clearance: {:?} of {own} within \
         {} mm of {:?} of {foreign}{band} (vector check)",
        found.own, found.gap, found.foreign
    )
}

/// The vector half of `validate_result`: a legacy footprint in original coordinates, one
/// class per distinct placement angle (its kinds turn rigidly with the model).
pub(crate) fn validate_geometry(
    fp: &Footprint,
    options: &PackOptions,
    result: &PackResult,
) -> Result<(), String> {
    let plate = options.effective_plate_rules();
    let rules = LayoutRules {
        zones: plate_zones(&plate, None)?,
        body_rule: plate.body_rule,
        clearance_mm: options.clearance,
        foreign: options.foreign_gaps,
        support_overlap_mm: options.support_overlap_mm,
    };
    let print = ObjectPrint::from_legacy(fp);
    // Legacy rules keep the legacy bed rule: everything printed stays on the bed.
    let extent = if options.plate_rules.is_none() {
        legacy_extent(fp)
    } else {
        Loops::new()
    };
    let (angles, placements) = angle_classes(&result.placements);
    let classes = angles
        .par_iter()
        .map(|&angle| {
            let extent = (!extent.is_empty())
                .then(|| Rect::of_points(rotate_loops(&extent, angle).iter().flatten()));
            ClassShapes::new(rotated_print(&print, angle), extent)
        })
        .collect();
    VectorValidator::from_classes(classes, &rules).check(&placements)
}

/// The distinct angles of `placements` in order of appearance, and every placement as a
/// translation of its angle's class.
pub(crate) fn angle_classes(placements: &[PackedPlacement]) -> (Vec<f64>, Vec<PrintPlacement>) {
    let mut angles: Vec<f64> = Vec::new();
    let placements = placements
        .iter()
        .map(|p| {
            let known = angles
                .iter()
                .position(|a| a.to_bits() == p.angle_deg.to_bits());
            let print = known.unwrap_or_else(|| {
                angles.push(p.angle_deg);
                angles.len() - 1
            });
            PrintPlacement {
                print,
                x_mm: p.x_mm,
                y_mm: p.y_mm,
            }
        })
        .collect();
    (angles, placements)
}

/// Every loop of the legacy footprint: the legacy bed rule keeps all of it on the bed.
fn legacy_extent(fp: &Footprint) -> Loops {
    let zones = fp
        .layer_zones
        .iter()
        .flat_map(|z| z.support_loops.iter().chain(&z.body_loops));
    fp.loops
        .iter()
        .chain(&fp.body_loops)
        .chain(&fp.support_landing_loops)
        .chain(fp.bed_loops.iter().flatten())
        .chain(zones)
        .cloned()
        .collect()
}

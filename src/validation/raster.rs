//! The raster check of a layout: every placement re-rasterized at its own sub-pixel phase
//! (never the search's no-fit maps), its plate zones tested at its pixel origin and every
//! pair of placements whose masks can meet tested mask by mask. Masks are cached by
//! (class, sub-pixel offset), so a lattice or grid layout rasterizes each class once.

use super::{
    LayoutRules, PrintPlacement, check_print_placements, check_prints, first_failure, meeting_pairs,
};
use crate::collision::{support_cores_overlap, zones_collide};
use crate::geometry::{Point, Rect};
use crate::kind_masks::{KindMasks, KindShapes, MaskRules, OriginGrid, PlateZone, ZoneChecker};
use crate::object_print::ObjectPrint;
use crate::raster_mask::LegacyPrint;
use rayon::prelude::*;

/// A placement this close to a pixel origin (mm) is rasterized as is, without shifting.
const ON_GRID_TOLERANCE_MM: f64 = 1e-7;

/// Where the masks of a class come from.
enum MaskSource<'a> {
    /// One print per rotation class, in its plate-aligned frame: copies are translations.
    Prints(Vec<KindShapes>),
    /// A legacy footprint turned rigidly to each class's angle.
    Legacy {
        print: &'a LegacyPrint,
        angles: Vec<f64>,
    },
}

impl MaskSource<'_> {
    fn classes(&self) -> usize {
        match self {
            MaskSource::Prints(shapes) => shapes.len(),
            MaskSource::Legacy { angles, .. } => angles.len(),
        }
    }

    fn masks(&self, class: usize, offset: Point, resolution: f64) -> KindMasks {
        match self {
            MaskSource::Prints(shapes) if offset == ORIGIN => shapes[class].rasterize(resolution),
            MaskSource::Prints(shapes) => shapes[class]
                .mapped(|p| Point {
                    x: p.x + offset.x,
                    y: p.y + offset.y,
                })
                .rasterize(resolution),
            MaskSource::Legacy { print, angles } => print.masks(angles[class], offset),
        }
    }
}

const ORIGIN: Point = Point { x: 0.0, y: 0.0 };

/// A class and the sub-pixel offset of a placement from its pixel origin, mm.
type Phase = (usize, Point);

/// The masks of one (class, sub-pixel offset) and its zone tests.
struct Cached<'a> {
    key: (usize, u64, u64),
    masks: KindMasks,
    zones: ZoneChecker<'a>,
    /// Inclusive pixel box of every mask (`None` when nothing is set), mask frame.
    bounds: Option<Rect>,
}

/// Zone tests a shared validator adds for one caller (one tower anchor), by cache slot.
pub(crate) struct ExtraZones<'z> {
    checkers: Vec<(usize, ZoneChecker<'z>)>,
}

impl ExtraZones<'_> {
    fn checker(&self, slot: usize) -> Option<&ZoneChecker<'_>> {
        self.checkers
            .binary_search_by_key(&slot, |(s, _)| *s)
            .ok()
            .map(|i| &self.checkers[i].1)
    }
}

/// The raster check of layouts of fixed prints and zones. Kind shapes are grown once and
/// masks are cached across `check` calls, so checking a shortlist of candidates costs one
/// rasterization per distinct (class, phase).
pub struct RasterValidator<'a> {
    zones: &'a [PlateZone],
    grid: OriginGrid,
    support_overlap_mm: Option<f64>,
    source: MaskSource<'a>,
    cache: Vec<Cached<'a>>,
}

impl<'a> RasterValidator<'a> {
    /// `prints[i]` is rotation class `i`, already in its plate-aligned frame. `grid` is the
    /// pixel grid of the masks; placements off it are rasterized at their sub-pixel phase.
    pub fn new(
        prints: &[ObjectPrint],
        rules: &'a LayoutRules,
        grid: OriginGrid,
    ) -> Result<RasterValidator<'a>, String> {
        rules.check_values()?;
        check_prints(prints)?;
        if !grid.resolution.is_finite() || grid.resolution <= 0.0 {
            return Err("invalid raster resolution".to_owned());
        }
        let mask_rules = MaskRules {
            clearance_mm: rules.clearance_mm,
            foreign: rules.foreign,
            body_rule: rules.body_rule,
            support_overlap_mm: rules.support_overlap_mm,
        };
        let shapes = prints
            .iter()
            .map(|print| KindShapes::new(print, mask_rules))
            .collect();
        Ok(RasterValidator {
            zones: &rules.zones,
            grid,
            support_overlap_mm: rules.support_overlap_mm,
            source: MaskSource::Prints(shapes),
            cache: Vec::new(),
        })
    }

    /// A legacy footprint's masks at `angles` (one class per angle), on its own zones and grid.
    pub(crate) fn legacy(
        print: &'a LegacyPrint,
        angles: Vec<f64>,
        support_overlap_mm: Option<f64>,
    ) -> RasterValidator<'a> {
        RasterValidator {
            zones: &print.zones,
            grid: print.grid,
            support_overlap_mm,
            source: MaskSource::Legacy { print, angles },
            cache: Vec::new(),
        }
    }

    /// The first broken rule, by placement order: a placement's zones before its pairs with
    /// earlier placements.
    pub fn check(&mut self, placements: &[PrintPlacement]) -> Result<(), String> {
        check_print_placements(placements, self.source.classes())?;
        let (centers, keys): (Vec<(i32, i32)>, Vec<Phase>) =
            placements.iter().map(|p| self.pixel(p)).unzip();
        let slots = self.cached(&keys);
        self.verdict(&centers, &slots, None)
    }

    /// Rasterizes every (class, phase) of `layouts` up front, so that `extra_zones` and
    /// `check_primed` can then run on one shared validator. A layout `check` refuses outright
    /// (an unknown class) is skipped: `check_primed` refuses it the same way.
    pub(crate) fn prime(&mut self, layouts: &[Vec<PrintPlacement>]) {
        let classes = self.source.classes();
        let keys: Vec<Phase> = layouts
            .iter()
            .filter(|layout| check_print_placements(layout, classes).is_ok())
            .flat_map(|layout| layout.iter().map(|p| self.pixel(p).1))
            .collect();
        self.cached(&keys);
    }

    /// The tests of `zones` (zones beyond the validator's own, e.g. one tower anchor's) for
    /// every primed phase of `layouts`. The masks and the tests of the validator's own zones
    /// serve every caller; only these are built per caller.
    pub(crate) fn extra_zones<'z>(
        &self,
        zones: &'z [PlateZone],
        layouts: &[&[PrintPlacement]],
    ) -> ExtraZones<'z> {
        let mut slots: Vec<usize> = layouts
            .iter()
            .flat_map(|layout| layout.iter())
            .filter_map(|p| self.slot(&self.pixel(p).1))
            .collect();
        slots.sort_unstable();
        slots.dedup();
        let checkers = slots
            .par_iter()
            .map(|&slot| (slot, ZoneChecker::new(&self.cache[slot].masks, zones, &self.grid)))
            .collect();
        ExtraZones { checkers }
    }

    /// `check` of a primed layout against the validator's zones followed by `extra`'s: the
    /// verdict of a validator built on the concatenated zone list, whose tests run zone by
    /// zone in list order.
    pub(crate) fn check_primed(
        &self,
        placements: &[PrintPlacement],
        extra: &ExtraZones<'_>,
    ) -> Result<(), String> {
        check_print_placements(placements, self.source.classes())?;
        let (centers, slots): (Vec<(i32, i32)>, Vec<usize>) = placements
            .iter()
            .map(|p| {
                let (center, key) = self.pixel(p);
                (center, self.slot(&key).expect("primed layout"))
            })
            .unzip();
        self.verdict(&centers, &slots, Some(extra))
    }

    /// The first broken rule of placements at pixel `centers` with cached masks `slots`.
    fn verdict(
        &self,
        centers: &[(i32, i32)],
        slots: &[usize],
        extra: Option<&ExtraZones<'_>>,
    ) -> Result<(), String> {
        let zone_failure = slots
            .par_iter()
            .enumerate()
            .find_map_first(|(index, &slot)| {
                let (x, y) = centers[index];
                let conflict = self.cache[slot].zones.conflict(x, y).or_else(|| {
                    extra
                        .and_then(|extra| extra.checker(slot))
                        .and_then(|checker| checker.conflict(x, y))
                })?;
                Some((index, format!("placement {index}: {conflict}")))
            });
        let boxes: Vec<Rect> = slots
            .iter()
            .zip(centers)
            .map(|(&slot, &(x, y))| {
                let empty = Rect::of_points(std::iter::empty());
                let moved = |b: Rect| b.translated(f64::from(x), f64::from(y));
                self.cache[slot].bounds.map_or(empty, moved)
            })
            .collect();
        let pair_failure = meeting_pairs(&boxes)
            .par_iter()
            .find_map_first(|&(previous, index)| {
                let message = self.pair_conflict(
                    (
                        previous,
                        &self.cache[slots[previous]].masks,
                        centers[previous],
                    ),
                    (index, &self.cache[slots[index]].masks, centers[index]),
                )?;
                Some((index, message))
            });
        first_failure(zone_failure, pair_failure)
    }

    /// The rounded pixel origin of a placement and its mask key (class, leftover offset).
    /// Keeping the fractional phase of imported or manual placements matters: rounding every
    /// origin independently can turn a real overlap into two merely touching masks.
    fn pixel(&self, p: &PrintPlacement) -> ((i32, i32), Phase) {
        let r = self.grid.resolution;
        let (fx, fy) = (p.x_mm - self.grid.origin.x, p.y_mm - self.grid.origin.y);
        let (x, y) = ((fx / r).round() as i32, (fy / r).round() as i32);
        let mut offset = Point {
            x: fx - f64::from(x) * r,
            y: fy - f64::from(y) * r,
        };
        if offset.x.abs() < ON_GRID_TOLERANCE_MM && offset.y.abs() < ON_GRID_TOLERANCE_MM {
            offset = ORIGIN;
        }
        ((x, y), (p.print, offset))
    }

    /// Cache slots of `keys`, rasterizing the missing ones in parallel (in key order).
    fn cached(&mut self, keys: &[Phase]) -> Vec<usize> {
        let bits = |&(class, offset): &Phase| (class, offset.x.to_bits(), offset.y.to_bits());
        let mut missing: Vec<Phase> = Vec::new();
        for key in keys {
            let known = self.cache.iter().any(|c| c.key == bits(key))
                || missing.iter().any(|m| bits(m) == bits(key));
            if !known {
                missing.push(*key);
            }
        }
        let (source, zones, grid) = (&self.source, self.zones, &self.grid);
        let built: Vec<Cached<'a>> = missing
            .par_iter()
            .map(|key @ &(class, offset)| {
                let masks = source.masks(class, offset, grid.resolution);
                Cached {
                    key: bits(key),
                    zones: ZoneChecker::new(&masks, zones, grid),
                    bounds: pixel_bounds(&masks),
                    masks,
                }
            })
            .collect();
        self.cache.extend(built);
        keys.iter()
            .map(|key| self.slot(key).expect("cached above"))
            .collect()
    }

    /// The cache slot of a (class, phase), if rasterized.
    fn slot(&self, &(class, offset): &Phase) -> Option<usize> {
        let key = (class, offset.x.to_bits(), offset.y.to_bits());
        self.cache.iter().position(|c| c.key == key)
    }

    /// Two placements (index, masks, pixel origin) break a pair rule or the support overlap.
    fn pair_conflict(
        &self,
        (previous, previous_masks, (previous_x, previous_y)): (usize, &KindMasks, (i32, i32)),
        (index, masks, (x, y)): (usize, &KindMasks, (i32, i32)),
    ) -> Option<String> {
        let (dx, dy) = (x - previous_x, y - previous_y);
        // Pair rules (`keepout::pair_rules`): silhouettes keep clearance, a support base keeps
        // its gap from a foreign landing, supports keep theirs from foreign bodies band by
        // band, supports may touch other supports: the predicate the packing enforces.
        if zones_collide(previous_masks, masks, dx, dy) {
            return Some(format!(
                "placements {previous} and {index} overlap: a body touches a foreign structure"
            ));
        }
        support_cores_overlap(previous_masks, masks, dx, dy).then(|| {
            format!(
                "placements {previous} and {index}: supports overlap by more than {} mm",
                self.support_overlap_mm.unwrap_or_default()
            )
        })
    }
}

/// The raster layout check of the slicer-free core (design §8.5), for every shortlisted
/// candidate. Keep a `RasterValidator` to check several candidates of the same prints.
pub fn validate_raster(
    prints: &[ObjectPrint],
    placements: &[PrintPlacement],
    rules: &LayoutRules,
    grid: OriginGrid,
) -> Result<(), String> {
    RasterValidator::new(prints, rules, grid)?.check(placements)
}

/// Inclusive pixel box of every set mask: two copies whose boxes miss cannot collide.
fn pixel_bounds(masks: &KindMasks) -> Option<Rect> {
    let set = masks.masks.iter().chain(&masks.support_core);
    let boxes = set.filter(|m| m.pixel_count > 0).map(|m| {
        let (x, y) = (f64::from(m.min_x), f64::from(m.min_y));
        Rect::new(x, y, x + m.width as f64 - 1.0, y + m.height as f64 - 1.0)
    });
    boxes.reduce(Rect::union)
}

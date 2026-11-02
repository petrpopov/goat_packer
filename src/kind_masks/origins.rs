//! Which origins a print may take on the plate (design 2026-11-02, §8.4): the search's base
//! map, removable layers for zones the search moves (the tower), and the check of a single
//! origin. All three evaluate the same tests (`zone_tests`), so they cannot drift.

use super::zone_tests::{
    PlateTest, Range, ZoneTest, clear_range, inside_range, intersect, plate_tests, rect_of,
};
use super::{KindMasks, PlateZone};
use crate::collision::overlap_shifts;
use crate::geometry::Point;
use crate::span_raster::SpanRaster;

/// Origins are `origin + (x, y) * resolution` for integer `(x, y)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OriginGrid {
    pub origin: Point,
    pub resolution: f64,
}

/// The origins a print may take: the search's base map before any other object is placed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeasibleOrigins {
    pub(crate) allowed: SpanRaster,
}

impl FeasibleOrigins {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.allowed.contains(x, y)
    }

    pub fn count(&self) -> usize {
        self.allowed.pixel_count()
    }

    /// Inclusive `(min_x, min_y, max_x, max_y)` of the allowed origins; `None` when empty.
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let rows = self.allowed.rows.iter().enumerate();
        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        for (index, row) in rows.filter(|(_, row)| !row.is_empty()) {
            let y = self.allowed.min_y + index as i32;
            let (first, last) = (row[0].0, row[row.len() - 1].1 - 1);
            bounds = Some(match bounds {
                None => (first, y, last, y),
                Some((x0, y0, x1, _)) => (x0.min(first), y0, x1.max(last), y),
            });
        }
        bounds
    }

    /// This map with the origins of `set` removed (a movable zone placed, e.g. the tower).
    pub fn without(&self, set: &OriginSet) -> FeasibleOrigins {
        FeasibleOrigins {
            allowed: self.allowed.difference(&set.spans),
        }
    }
}

/// Origins one group of zones forbids: a removable layer over a base map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OriginSet {
    pub(crate) spans: SpanRaster,
}

impl OriginSet {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.spans.contains(x, y)
    }

    pub fn count(&self) -> usize {
        self.spans.pixel_count()
    }
}

/// The origins where `masks` satisfies every zone. `zones` must hold the printable boundary
/// (`plate_zones` always does): it is what bounds the map.
pub fn feasible_origins(
    masks: &KindMasks,
    zones: &[PlateZone],
    grid: &OriginGrid,
) -> FeasibleOrigins {
    let (range, forbidden) = constraints(masks, zones, grid);
    let range = range.expect("plate zones without the printable boundary have no bounded origins");
    FeasibleOrigins {
        allowed: rect_of(range).difference(&forbidden),
    }
}

/// The origins of `within` that `zones` forbid: the removable layer of a zone group whose
/// position the search chooses (the prime tower's body and brim). Moving such a group by
/// whole pixels moves its set by the same amount, so a caller can build it once.
pub fn forbidden_origins(
    masks: &KindMasks,
    zones: &[PlateZone],
    grid: &OriginGrid,
    within: &FeasibleOrigins,
) -> OriginSet {
    let Some(bounds) = within.bounds() else {
        return OriginSet::default();
    };
    let (range, forbidden) = constraints(masks, zones, grid);
    let range = range.map_or(bounds, |range| intersect(range, bounds));
    let allowed = rect_of(range).difference(&forbidden);
    OriginSet {
        spans: within.allowed.difference(&allowed),
    }
}

/// The first zone rule `masks` breaks at origin `(x, y)`, as a message; `None` when it fits.
pub fn zone_conflict(
    masks: &KindMasks,
    zones: &[PlateZone],
    grid: &OriginGrid,
    x: i32,
    y: i32,
) -> Option<String> {
    ZoneChecker::new(masks, zones, grid).conflict(x, y)
}

/// The zone tests of one print, built once and evaluated at many origins (validation checks
/// every copy of a rotation class with one checker).
pub struct ZoneChecker<'a> {
    zones: &'a [PlateZone],
    grid: OriginGrid,
    tests: Vec<PlateTest>,
}

impl<'a> ZoneChecker<'a> {
    pub fn new(masks: &KindMasks, zones: &'a [PlateZone], grid: &OriginGrid) -> Self {
        ZoneChecker {
            zones,
            grid: *grid,
            tests: plate_tests(masks, zones, grid),
        }
    }

    /// The first zone rule broken at origin `(x, y)`, as a message.
    pub fn conflict(&self, x: i32, y: i32) -> Option<String> {
        self.tests
            .iter()
            .find(|test| test.test.violated_at(&self.grid, x, y))
            .map(|test| test.test.describe(&test.what, &self.zones[test.zone]))
    }
}

/// The bounding range all `Outside` zones leave (`None` without any) and the origins the
/// other zones forbid.
fn constraints(
    masks: &KindMasks,
    zones: &[PlateZone],
    grid: &OriginGrid,
) -> (Option<Range>, SpanRaster) {
    let mut range: Option<Range> = None;
    let mut forbidden = SpanRaster::default();
    for PlateTest { test, .. } in plate_tests(masks, zones, grid) {
        match test {
            ZoneTest::BoxesInside { boxes, inside } => {
                for b in boxes {
                    let fits = inside_range(b, inside, grid);
                    range = Some(range.map_or(fits, |range| intersect(range, fits)));
                }
            }
            ZoneTest::BoxesClear { boxes, rect } => {
                for b in boxes {
                    forbidden = forbidden.union(&rect_of(clear_range(b, rect, grid)));
                }
            }
            ZoneTest::Raster { material, zone } => {
                forbidden = forbidden.union(&overlap_shifts(&[(&zone, &material)]));
            }
        }
    }
    (range, forbidden)
}

//! The permission matrix against plate zones as tests (design 2026-11-02, §7.2, §8.4):
//! every (zone, kind) entry of `keepout::permission` that is not `Allowed` becomes one test.
//! Box tests against rectangles are exact origin ranges; everything else is the conservative
//! cover of the material against the cover of the zone.

use super::buffer::grown;
use super::origins::OriginGrid;
use super::{KindMasks, PlateZone, ZoneArea, cover};
use crate::geometry::{Point, Rect};
use crate::keepout::{Gap, Permission, ZoneClass, permission};
use crate::object_print::{Loops, ObjectPrint, PrintKind};
use crate::pixel_mask::{PixelMask, masks_overlap};
use crate::span_raster::SpanRaster;

/// Inclusive origin range `(min_x, min_y, max_x, max_y)`.
pub(super) type Range = (i32, i32, i32, i32);

/// Pixel slack of the exact range tests: equality up to float noise passes.
const RANGE_SLACK_PX: f64 = 1e-9;

/// One entry of the permission matrix against one zone, ready to evaluate.
pub(super) enum ZoneTest {
    /// Every box (print frame) stays inside `inside`; equality passes.
    BoxesInside { boxes: Vec<Rect>, inside: Rect },
    /// No box overlaps `rect`; equality passes.
    BoxesClear { boxes: Vec<Rect>, rect: Rect },
    /// `material` (print-frame pixels) must not overlap `zone` (grid pixels).
    Raster {
        material: PixelMask,
        zone: PixelMask,
    },
}

impl ZoneTest {
    pub(super) fn violated_at(&self, grid: &OriginGrid, x: i32, y: i32) -> bool {
        match self {
            ZoneTest::BoxesInside { boxes, inside } => boxes
                .iter()
                .any(|b| !in_range(inside_range(*b, *inside, grid), x, y)),
            ZoneTest::BoxesClear { boxes, rect } => boxes
                .iter()
                .any(|b| in_range(clear_range(*b, *rect, grid), x, y)),
            ZoneTest::Raster { material, zone } => masks_overlap(zone, 0, 0, material, x, y),
        }
    }

    pub(super) fn describe(&self, what: &str, zone: &PlateZone) -> String {
        let how = match self {
            ZoneTest::BoxesInside { .. } => "leaves the area inside",
            ZoneTest::BoxesClear { .. } | ZoneTest::Raster { .. } => "enters",
        };
        format!("{what} {how} {:?} zone '{}'", zone.class, zone.id)
    }
}

/// The kinds of an object (the tower kinds belong to the plate).
const OBJECT_KINDS: [PrintKind; 7] = [
    PrintKind::Silhouette,
    PrintKind::BodyBand,
    PrintKind::Landing,
    PrintKind::Brim,
    PrintKind::SupportBase,
    PrintKind::SupportBand,
    PrintKind::InterfaceBand,
];

/// Gaps in a fixed order, so material tests come out in the same order every run.
const GAPS: [Gap; 3] = [Gap::Zero, Gap::Clearance, Gap::Support];

/// One test of one zone, with what it is about (for messages).
pub(super) struct PlateTest {
    pub(super) zone: usize,
    pub(super) test: ZoneTest,
    pub(super) what: String,
}

/// Material loops one row of the matrix puts against a zone, keyed by kind and band so
/// zones with the same rows share one cover.
type Materials<'a> = Vec<(PrintKind, usize, &'a Loops)>;

/// One forbidden row's material against a zone: its gap, kind, band and loops.
type RowMaterial<'a> = (Gap, PrintKind, usize, &'a Loops);

/// Material covers already built, by gap and (kind, band) list.
type CoverCache = Vec<((Gap, Vec<(PrintKind, usize)>), PixelMask)>;

/// Every test `zones` put on `masks`, zone by zone. Material covers are built once per
/// distinct (gap, material) and shared between zones.
pub(super) fn plate_tests(
    masks: &KindMasks,
    zones: &[PlateZone],
    grid: &OriginGrid,
) -> Vec<PlateTest> {
    let mut covers = CoverCache::new();
    let mut tests = Vec::new();
    for (index, zone) in zones.iter().enumerate() {
        let (parts, materials) = zone_rows(&masks.print, zone);
        if let Some(hulls) = parts {
            let test = part_test(masks, zone, grid, hulls);
            tests.push(PlateTest {
                zone: index,
                test,
                what: "body".to_owned(),
            });
        }
        for gap in GAPS {
            let group: Materials = materials
                .iter()
                .filter(|(g, ..)| *g == gap)
                .map(|&(_, kind, band, loops)| (kind, band, loops))
                .collect();
            if group.is_empty() {
                continue;
            }
            let what = material_label(group.iter().map(|(kind, ..)| *kind));
            let test = material_test(masks, &group, gap, zone, grid, &mut covers);
            tests.push(PlateTest {
                zone: index,
                test,
                what,
            });
        }
        if let Some(test) = bed_extent_test(masks, zone) {
            tests.push(PlateTest {
                zone: index,
                test,
                what: "printed extent".to_owned(),
            });
        }
    }
    tests
}

/// The matrix rows against `zone`: `Some(hulls)` when the parts are tested (by convex hull
/// when no row asks for the stricter box), and the material of every forbidden row.
fn zone_rows<'a>(print: &'a ObjectPrint, zone: &PlateZone) -> (Option<bool>, Vec<RowMaterial<'a>>) {
    let (mut boxes, mut hulls) = (false, false);
    let mut materials = Vec::new();
    for kind in OBJECT_KINDS {
        match permission(kind, zone.class) {
            Permission::Allowed => {}
            Permission::PartBox => boxes = true,
            Permission::PartHull => hulls = true,
            Permission::Forbidden { gap } => materials.extend(
                zone_material(print, kind, zone)
                    .into_iter()
                    .map(|(band, loops)| (gap, kind, band, loops)),
            ),
        }
    }
    ((boxes || hulls).then_some(!boxes), materials)
}

/// A legacy footprint's bed extent: the whole printed extent stays on the plate.
fn bed_extent_test(masks: &KindMasks, zone: &PlateZone) -> Option<ZoneTest> {
    let ZoneArea::Outside { inside, .. } = zone.area else {
        return None;
    };
    (zone.class == ZoneClass::Boundary && !masks.bed_extent.is_empty()).then(|| {
        let extent = Rect::of_points(masks.bed_extent.iter().flatten());
        ZoneTest::BoxesInside {
            boxes: vec![extent],
            inside,
        }
    })
}

/// The parts against a zone: their boxes (restricted, boundary) or their convex hulls (hard
/// zones; touching passes). Against an `Outside` zone a hull fits exactly when its box does.
fn part_test(masks: &KindMasks, zone: &PlateZone, grid: &OriginGrid, hulls: bool) -> ZoneTest {
    let print = &masks.print;
    let boxes = part_boxes(masks);
    match &zone.area {
        ZoneArea::Outside { inside, .. } => ZoneTest::BoxesInside {
            boxes,
            inside: *inside,
        },
        ZoneArea::Rect(rect) if !hulls => ZoneTest::BoxesClear { boxes, rect: *rect },
        area => {
            let rings: Vec<Loops> = if hulls && !print.part_hulls.is_empty() {
                print
                    .part_hulls
                    .iter()
                    .map(|hull| vec![hull.clone()])
                    .collect()
            } else {
                boxes.iter().map(|b| vec![rect_ring(*b)]).collect()
            };
            let refs: Vec<&Loops> = rings.iter().collect();
            ZoneTest::Raster {
                material: union_cover(&refs, 0.0, masks.resolution),
                zone: zone_mask(area, grid),
            }
        }
    }
}

/// Part boxes, or the print's whole extent when it carries none (no part may go unchecked).
fn part_boxes(masks: &KindMasks) -> Vec<Rect> {
    let boxes: Vec<Rect> = masks
        .print
        .part_boxes
        .iter()
        .copied()
        .filter(is_finite)
        .collect();
    if boxes.is_empty() && is_finite(&masks.extent_bounds) {
        return vec![masks.extent_bounds];
    }
    boxes
}

/// Material grown by the full `gap` against a zone: its box against an `Outside` zone (the
/// box of a disc-grown shape is its box grown by the radius), its cover otherwise.
fn material_test(
    masks: &KindMasks,
    group: &Materials,
    gap: Gap,
    zone: &PlateZone,
    grid: &OriginGrid,
    covers: &mut CoverCache,
) -> ZoneTest {
    let gap_mm = gap.mm(masks.rules.clearance_mm, masks.rules.foreign);
    match &zone.area {
        ZoneArea::Outside {
            inside,
            material_inset_mm,
        } => {
            let points = group.iter().flat_map(|(.., loops)| loops.iter().flatten());
            ZoneTest::BoxesInside {
                boxes: vec![Rect::of_points(points).grown(gap_mm)],
                inside: inside.grown(-material_inset_mm),
            }
        }
        area => {
            let key = (
                gap,
                group.iter().map(|&(kind, band, _)| (kind, band)).collect(),
            );
            let material = match covers.iter().find(|(known, _)| *known == key) {
                Some((_, mask)) => mask.clone(),
                None => {
                    let loops: Vec<&Loops> = group.iter().map(|(.., loops)| *loops).collect();
                    let mask = union_cover(&loops, gap_mm, masks.resolution);
                    covers.push((key, mask.clone()));
                    mask
                }
            };
            ZoneTest::Raster {
                material,
                zone: zone_mask(area, grid),
            }
        }
    }
}

/// Which loops of `kind` can meet `zone`, with their band (0 for kinds without bands). The
/// silhouette answers for zones at every height and the body bands for zones with a z range;
/// without body bands the silhouette answers for both. First-layer kinds are band 0.
pub(crate) fn zone_material<'a>(
    print: &'a ObjectPrint,
    kind: PrintKind,
    zone: &PlateZone,
) -> Vec<(usize, &'a Loops)> {
    let no_bands = print.body.iter().all(Vec::is_empty);
    let banded = |bands: &'a [Loops]| -> Vec<(usize, &'a Loops)> {
        bands
            .iter()
            .enumerate()
            .filter(|(band, _)| zone.meets_band(*band))
            .collect()
    };
    let first_layer = |loops: &'a Loops| {
        if zone.meets_band(0) {
            vec![(0, loops)]
        } else {
            vec![]
        }
    };
    let found = match kind {
        PrintKind::Silhouette if zone.z_range_mm.is_none() || no_bands => {
            vec![(0, &print.silhouette)]
        }
        PrintKind::BodyBand if zone.z_range_mm.is_some() => banded(&print.body),
        PrintKind::Landing => first_layer(&print.landing),
        PrintKind::Brim => first_layer(&print.brim),
        PrintKind::SupportBase => first_layer(&print.support_base),
        PrintKind::SupportBand => banded(&print.support),
        PrintKind::InterfaceBand => banded(&print.interface),
        _ => vec![],
    };
    found
        .into_iter()
        .filter(|(_, loops)| !loops.is_empty())
        .collect()
}

fn material_label(kinds: impl Iterator<Item = PrintKind>) -> String {
    let mut names: Vec<String> = Vec::new();
    for kind in kinds {
        let name = format!("{kind:?}");
        if !names.contains(&name) {
            names.push(name);
        }
    }
    format!("{} material", names.join("/"))
}

/// The union of each loop set's cover after growing it by `radius`: sets are unioned, never
/// concatenated (concatenated even-odd rings of two bands would cancel where they overlap).
fn union_cover(loops: &[&Loops], radius: f64, resolution: f64) -> PixelMask {
    let union = loops.iter().fold(SpanRaster::default(), |union, loops| {
        union.union(&cover(&grown(loops, radius), resolution))
    });
    PixelMask::from_spans(&union)
}

/// A zone's area in grid pixels.
fn zone_mask(area: &ZoneArea, grid: &OriginGrid) -> PixelMask {
    let loops = match area {
        ZoneArea::Rect(rect) => vec![rect_ring(*rect)],
        ZoneArea::Loops(loops) => loops.clone(),
        ZoneArea::Outside { .. } => unreachable!("outside zones are range tests"),
    };
    let shifted: Loops = loops
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|p| Point {
                    x: p.x - grid.origin.x,
                    y: p.y - grid.origin.y,
                })
                .collect()
        })
        .collect();
    PixelMask::from_spans(&cover(&shifted, grid.resolution))
}

fn rect_ring(r: Rect) -> Vec<Point> {
    [
        (r.min_x, r.min_y),
        (r.max_x, r.min_y),
        (r.max_x, r.max_y),
        (r.min_x, r.max_y),
        (r.min_x, r.min_y),
    ]
    .iter()
    .map(|&(x, y)| Point { x, y })
    .collect()
}

fn is_finite(r: &Rect) -> bool {
    [r.min_x, r.min_y, r.max_x, r.max_y]
        .iter()
        .all(|v| v.is_finite())
}

/// Origins where box `b` lies inside `inside` (equality passes).
pub(super) fn inside_range(b: Rect, inside: Rect, grid: &OriginGrid) -> Range {
    let (r, o) = (grid.resolution, grid.origin);
    (
        ((inside.min_x - o.x - b.min_x) / r - RANGE_SLACK_PX).ceil() as i32,
        ((inside.min_y - o.y - b.min_y) / r - RANGE_SLACK_PX).ceil() as i32,
        ((inside.max_x - o.x - b.max_x) / r + RANGE_SLACK_PX).floor() as i32,
        ((inside.max_y - o.y - b.max_y) / r + RANGE_SLACK_PX).floor() as i32,
    )
}

/// Origins where box `b` overlaps `rect` with positive area (touching passes).
pub(super) fn clear_range(b: Rect, rect: Rect, grid: &OriginGrid) -> Range {
    let (r, o) = (grid.resolution, grid.origin);
    (
        ((rect.min_x - o.x - b.max_x) / r + RANGE_SLACK_PX).floor() as i32 + 1,
        ((rect.min_y - o.y - b.max_y) / r + RANGE_SLACK_PX).floor() as i32 + 1,
        ((rect.max_x - o.x - b.min_x) / r - RANGE_SLACK_PX).ceil() as i32 - 1,
        ((rect.max_y - o.y - b.min_y) / r - RANGE_SLACK_PX).ceil() as i32 - 1,
    )
}

fn in_range((x0, y0, x1, y1): Range, x: i32, y: i32) -> bool {
    x0 <= x && x <= x1 && y0 <= y && y <= y1
}

pub(super) fn intersect(a: Range, b: Range) -> Range {
    (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3))
}

pub(super) fn rect_of((x0, y0, x1, y1): Range) -> SpanRaster {
    SpanRaster::rect(x0, y0, x1, y1)
}

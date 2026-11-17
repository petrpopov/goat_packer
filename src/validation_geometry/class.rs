//! One rotation class, prepared once for the vector check: every kind (and band) as a
//! `Region` in the print's own frame, plus the part hulls and boxes. A copy of the class is
//! the same shapes at a translation, so nothing is rebuilt per placement.

use super::region::{Region, moved_box};
use crate::geometry::{Point, Rect};
use crate::object_print::{ObjectPrint, PrintKind};
use rayon::prelude::*;

/// Kinds without bands, in the order `flat` stores them.
const FLAT_KINDS: [PrintKind; 4] = [
    PrintKind::Silhouette,
    PrintKind::Landing,
    PrintKind::Brim,
    PrintKind::SupportBase,
];

/// Kinds with one entry per absolute band, in the order `bands` stores them.
const BAND_KINDS: [PrintKind; 3] = [
    PrintKind::BodyBand,
    PrintKind::SupportBand,
    PrintKind::InterfaceBand,
];

pub(super) struct ClassShapes {
    /// The print in its plate-aligned frame (for `zone_material`, which picks the bands).
    pub(super) print: ObjectPrint,
    flat: Vec<Region>,
    bands: Vec<Vec<Region>>,
    pub(super) hulls: Vec<Region>,
    pub(super) boxes: Vec<Rect>,
    /// A legacy footprint's bed extent: its box stays inside the printable area.
    pub(super) extent: Option<Rect>,
    /// Box around everything above, print frame.
    pub(super) bounds: Rect,
}

impl ClassShapes {
    /// `print` is already in the plate-aligned frame of its class: copies are translations.
    pub(super) fn new(print: ObjectPrint, extent: Option<Rect>) -> ClassShapes {
        let band_count = print.band_count();
        let source = &print;
        let banded = BAND_KINDS
            .iter()
            .flat_map(|&kind| (0..band_count).map(move |band| source.loops_of(kind, band)));
        let sources: Vec<&[Vec<Point>]> = FLAT_KINDS
            .iter()
            .map(|&kind| source.loops_of(kind, 0))
            .chain(banded)
            .collect();
        // Regions are independent; an indexed parallel collect keeps their order.
        let regions: Vec<Region> = sources.par_iter().map(|loops| Region::new(loops)).collect();
        let hulls: Vec<Region> = print
            .part_hulls
            .par_iter()
            .map(|hull| Region::new(std::slice::from_ref(hull)))
            .collect();
        let bounds = regions
            .iter()
            .chain(&hulls)
            .map(Region::bounds)
            .chain(print.part_boxes.iter().copied())
            .chain(extent)
            .fold(Rect::of_points(std::iter::empty()), Rect::union);
        let mut regions = regions.into_iter();
        let flat = regions.by_ref().take(FLAT_KINDS.len()).collect();
        let bands = BAND_KINDS
            .iter()
            .map(|_| regions.by_ref().take(band_count).collect())
            .collect();
        ClassShapes {
            boxes: print.part_boxes.clone(),
            print,
            flat,
            bands,
            hulls,
            extent,
            bounds,
        }
    }

    /// The region of `kind` in `band` (0 for kinds without bands); `None` when it is empty.
    pub(super) fn region(&self, kind: PrintKind, band: usize) -> Option<&Region> {
        let region = if let Some(i) = FLAT_KINDS.iter().position(|&k| k == kind) {
            self.flat.get(i)
        } else {
            let i = BAND_KINDS.iter().position(|&k| k == kind)?;
            self.bands.get(i)?.get(band)
        };
        region.filter(|r| !r.is_empty())
    }

    pub(super) fn band_count(&self) -> usize {
        self.print.band_count()
    }

    /// Everything the class prints, at translation `at`.
    pub(super) fn bounds_at(&self, at: Point) -> Rect {
        moved_box(self.bounds, at)
    }
}

/// A legacy footprint's print turned rigidly by `angle_deg` about its origin.
pub(super) fn rotated_print(print: &ObjectPrint, angle_deg: f64) -> ObjectPrint {
    print.turned(angle_deg)
}

pub(super) fn rect_ring(r: Rect) -> Vec<Point> {
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

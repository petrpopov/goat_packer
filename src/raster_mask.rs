//! Today's `Footprint` + `PackOptions` as the slicer-free core sees them: the footprint
//! becomes an `ObjectPrint` (`ObjectPrint::from_legacy`), the bed fields become plate zones
//! (`PackOptions::effective_plate_rules`), and the masks come from `kind_masks`. The search,
//! the grid and `validate_result` all go through `LegacyPrint`, so the three keep one rule.

use crate::footprint::Footprint;
use crate::geometry::{Point, rotate_point};
use crate::keepout::{BodyRule, ForeignGaps};
use crate::kind_masks::{KindMasks, KindShapes, MaskRules, OriginGrid, PlateZone, plate_zones};
use crate::object_print::{Loops, ObjectPrint};
use crate::options::PackOptions;
use geo::{ConvexHull, MultiPoint};

/// The masks of one legacy footprint at one angle (the name the public API has always used).
pub type RasterMask = KindMasks;

/// A legacy footprint prepared once per packing run: kinds grown in the footprint frame,
/// turned rigidly for each angle the search tries. Rigid turning is the legacy contract (a
/// rotatable footprint is a mesh silhouette; locked footprints stay at 0°).
pub(crate) struct LegacyPrint {
    shapes: KindShapes,
    pub(crate) zones: Vec<PlateZone>,
    pub(crate) grid: OriginGrid,
    resolution: f64,
}

impl LegacyPrint {
    pub(crate) fn new(footprint: &Footprint, options: &PackOptions) -> Result<Self, String> {
        let rules = options.effective_plate_rules();
        let zones = plate_zones(&rules, None)?;
        let mask_rules = MaskRules {
            clearance_mm: options.clearance,
            foreign: options.foreign_gaps,
            body_rule: rules.body_rule,
            support_overlap_mm: options.support_overlap_mm,
        };
        // Legacy rules keep the legacy bed rule: everything printed, `bed_loops` included,
        // stays on the bed. Explicit plate rules say it themselves (`print_edge_reserve_mm`).
        let legacy_bed = options.plate_rules.is_none();
        Ok(LegacyPrint {
            shapes: legacy_shapes(footprint, mask_rules, legacy_bed),
            zones,
            grid: OriginGrid {
                origin: Point {
                    x: options.bed.min_x,
                    y: options.bed.min_y,
                },
                resolution: options.resolution,
            },
            resolution: options.resolution,
        })
    }

    /// The masks at `angle_deg`, shifted by `offset` mm after turning (a placement off the
    /// pixel grid keeps its sub-pixel phase).
    pub(crate) fn masks(&self, angle_deg: f64, offset: Point) -> KindMasks {
        moved(&self.shapes, angle_deg, offset).rasterize(self.resolution)
    }

    pub(crate) fn masks_at(&self, angle_deg: f64) -> KindMasks {
        self.masks(angle_deg, Point { x: 0.0, y: 0.0 })
    }
}

/// `footprint` as kind shapes in its own frame. `legacy_bed` adds the convex hull of every
/// loop as the bed extent: only its bounding box is ever tested, at any angle.
fn legacy_shapes(footprint: &Footprint, rules: MaskRules, legacy_bed: bool) -> KindShapes {
    let mut shapes = KindShapes::new(&ObjectPrint::from_legacy(footprint), rules);
    if legacy_bed {
        shapes.bed_extent = extent_hull(footprint);
    }
    shapes
}

fn extent_hull(footprint: &Footprint) -> Loops {
    let points: Vec<(f64, f64)> = footprint
        .loops
        .iter()
        .chain(&footprint.body_loops)
        .chain(&footprint.support_landing_loops)
        .chain(footprint.bed_loops.iter().flatten())
        .chain(
            footprint
                .layer_zones
                .iter()
                .flat_map(|zone| zone.support_loops.iter().chain(&zone.body_loops)),
        )
        .flatten()
        .map(|p| (p.x, p.y))
        .collect();
    let hull = MultiPoint::from(points).convex_hull();
    let ring: Vec<Point> = hull
        .exterior()
        .coords()
        .map(|c| Point { x: c.x, y: c.y })
        .collect();
    if ring.is_empty() {
        Loops::new()
    } else {
        vec![ring]
    }
}

/// `shapes` turned by `angle_deg` about the footprint origin, then shifted by `offset`.
fn moved(shapes: &KindShapes, angle_deg: f64, offset: Point) -> KindShapes {
    let mut moved = shapes.mapped(|p| {
        let q = rotate_point(p, angle_deg);
        Point {
            x: q.x + offset.x,
            y: q.y + offset.y,
        }
    });
    moved.print.rotation_deg = shapes.print.rotation_deg + angle_deg;
    moved
}

pub fn rasterize_footprint(
    footprint: &Footprint,
    angle_deg: f64,
    resolution: f64,
    clearance: f64,
) -> RasterMask {
    rasterize_footprint_with_support(footprint, angle_deg, resolution, clearance, None)
}

/// The masks of `footprint` at `angle_deg` under the legacy bed rule and the silhouette body
/// rule, with the support core of `support_overlap_mm`.
pub fn rasterize_footprint_with_support(
    footprint: &Footprint,
    angle_deg: f64,
    resolution: f64,
    clearance: f64,
    support_overlap_mm: Option<f64>,
) -> RasterMask {
    assert!(resolution.is_finite() && resolution > 0.0);
    assert!(angle_deg.is_finite());
    let rules = MaskRules {
        clearance_mm: clearance,
        foreign: ForeignGaps::default(),
        body_rule: BodyRule::Silhouette,
        support_overlap_mm,
    };
    let shapes = legacy_shapes(footprint, rules, true);
    moved(&shapes, angle_deg, Point { x: 0.0, y: 0.0 }).rasterize(resolution)
}

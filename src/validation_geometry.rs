//! Vector-space publication check, independent of rasterization and no-fit maps.
//! This validates the supplied envelope, not supports a future slicer might generate.

use geo::{Area, BooleanOps, BoundingRect, Distance, Euclidean, MultiPolygon};

use crate::footprint::{Footprint, LAYER_ZONE_GAP_MM};
use crate::geometry::{Point, Rect, polygon_from_loop};
use crate::options::{PackOptions, PackResult, PackedPlacement};

const GAP_EPS: f64 = 1e-7;
const AREA_EPS: f64 = 1e-9;

struct Zone {
    polygon: MultiPolygon,
    bounds: Option<geo::Rect>,
}

impl Zone {
    fn new(loops: &[Vec<Point>], p: &PackedPlacement) -> Self {
        // Footprints use even-odd rings, not necessarily one exterior followed by holes.
        // XOR preserves holes, islands and winding without sharing raster fill logic.
        let mut polygon = MultiPolygon::new(vec![]);
        for ring in p.place_loops(loops) {
            polygon = polygon.xor(&MultiPolygon::new(vec![polygon_from_loop(&ring)]));
        }
        let bounds = polygon.bounding_rect();
        Self { polygon, bounds }
    }

    fn conflicts(&self, other: &Self, gap: f64) -> bool {
        let (Some(a), Some(b)) = (self.bounds, other.bounds) else {
            return false;
        };
        // Almost every pair is far apart, including most height-band pairs.
        if a.max().x + gap < b.min().x
            || b.max().x + gap < a.min().x
            || a.max().y + gap < b.min().y
            || b.max().y + gap < a.min().y
        {
            return false;
        }
        if gap > GAP_EPS && Euclidean.distance(&self.polygon, &other.polygon) < gap - GAP_EPS {
            return true;
        }
        self.polygon.intersection(&other.polygon).unsigned_area() > AREA_EPS
    }
}

struct Placed {
    patch: Zone,
    body: Zone,
    landing: Zone,
    layers: Vec<(Zone, Zone)>,
}

pub(crate) fn validate_geometry(
    fp: &Footprint,
    options: &PackOptions,
    result: &PackResult,
) -> Result<(), String> {
    let body = if fp.body_loops.is_empty() {
        &fp.loops
    } else {
        &fp.body_loops
    };
    let landing = if fp.support_landing_loops.is_empty() {
        body
    } else {
        &fp.support_landing_loops
    };
    let mut placed: Vec<Placed> = Vec::with_capacity(result.placements.len());
    for (index, p) in result.placements.iter().enumerate() {
        // Original coordinates, not the rounded canonical frame used by the raster search.
        let all = std::iter::once(&fp.loops)
            .chain(std::iter::once(body))
            .chain(std::iter::once(landing))
            .chain(fp.bed_loops.iter())
            .chain(
                fp.layer_zones
                    .iter()
                    .flat_map(|z| [&z.support_loops, &z.body_loops]),
            );
        let mut extent = Rect::new(
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for loops in all {
            extent = extent.union(Rect::of_points(p.place_loops(loops).iter().flatten()));
        }
        let body_extent = Rect::of_points(p.place_loops(body).iter().flatten());
        if extent.sticks_out_of(options.bed, GAP_EPS)
            || options
                .body_bed
                .is_some_and(|bed| body_extent.sticks_out_of(bed, GAP_EPS))
        {
            return Err(format!("placement {index} leaves bed (vector check)"));
        }
        // Independent rectangle predicate; same documented bbox semantics as slicers.
        let hits = |bounds: Rect, exclusions: &[Rect]| {
            exclusions.iter().any(|r| {
                bounds.min_x - options.clearance < r.max_x - GAP_EPS
                    && bounds.max_x + options.clearance > r.min_x + GAP_EPS
                    && bounds.min_y - options.clearance < r.max_y - GAP_EPS
                    && bounds.max_y + options.clearance > r.min_y + GAP_EPS
            })
        };
        if hits(extent, &options.exclusions) || hits(body_extent, &options.body_exclusions) {
            return Err(format!(
                "placement {index} intersects exclusion (vector check)"
            ));
        }
        let current = Placed {
            patch: Zone::new(&fp.loops, p),
            body: Zone::new(body, p),
            landing: Zone::new(landing, p),
            layers: fp
                .layer_zones
                .iter()
                .map(|z| (Zone::new(&z.support_loops, p), Zone::new(&z.body_loops, p)))
                .collect(),
        };
        for (previous, other) in placed.iter().enumerate() {
            let conflict = current.body.conflicts(&other.body, options.clearance)
                || current.patch.conflicts(&other.landing, options.clearance)
                || current.landing.conflicts(&other.patch, options.clearance)
                || current
                    .layers
                    .iter()
                    .zip(&other.layers)
                    .any(|((sa, ba), (sb, bb))| {
                        sa.conflicts(bb, LAYER_ZONE_GAP_MM) || ba.conflicts(sb, LAYER_ZONE_GAP_MM)
                    });
            if conflict {
                return Err(format!(
                    "placements {previous} and {index} overlap or violate clearance (vector check)"
                ));
            }
        }
        placed.push(current);
    }
    Ok(())
}

//! What one object occupies on the bed, and how it compares to its mesh silhouette.

use crate::geometry::{
    Point, Rect, closed_loop, drop_specks, filled_exterior_loops, point_in_loops,
    polygon_from_loop, sort_largest_first,
};
use crate::object_print::{Loops, ObjectPrint, Provenance};
use geo::{
    BooleanOps, ConvexHull, MultiPolygon,
    orient::{Direction, Orient},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One height band of `Footprint::layer_zones`, loops in the footprint frame.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct LayerZone {
    pub support_loops: Vec<Vec<Point>>,
    pub body_loops: Vec<Vec<Point>>,
}

/// Distance a neighbour's body keeps from this object's supports within a height band: the
/// deviation between a sample slice of a few objects and the full plate (≤ 0.5 mm measured on
/// a 43-goat P1S plate, 2026-11-01).
pub const SUPPORT_BODY_GAP_MM: f64 = 0.5;

/// Former name of `SUPPORT_BODY_GAP_MM`, kept until the packing core migrates to the new rules.
pub const LAYER_ZONE_GAP_MM: f64 = SUPPORT_BODY_GAP_MM;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Footprint {
    /// XY loops (even-odd: outer rings and holes), centered on the mesh bbox. Built either
    /// from the whole model's top-down projection or — better, when a probe slice exists —
    /// from the slice's actual first layer, which already includes tree supports wandering
    /// several mm past the body and any brim: the real patch two packed neighbours must not
    /// overlap with.
    pub loops: Vec<Vec<Point>>,
    /// The model-body subset of `loops` (everything printed by non-support extrusions, plus
    /// brim). Two-zone rule: a body must keep clearance from anything foreign, while supports
    /// may overlap other supports. Empty = the whole footprint is body (mesh-projection
    /// branch, or a first layer without supports).
    pub body_loops: Vec<Vec<Point>>,
    /// Where a FOREIGN object's patch must not land: the probe's first-layer body (walls and
    /// bottom of the model itself, no supports). Empty = `body_loops` (the plain two-zone rule).
    /// BambuStudio builds tree supports per object and never trims them around neighbours; a
    /// hand-made 40-goat P1S plate printed 15 neighbour pairs with one goat's support base on
    /// the bed under the other's silhouette (up to 21 mm²) while no printed lines met — the
    /// silhouette rule had cost that plate 3 goats. Silhouettes still keep clearance from each
    /// other (3D bodies); supports only must not land on what the neighbour lays on the bed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub support_landing_loops: Vec<Vec<Point>>,
    /// What the bed bounds and exclusions are checked against: the first-layer patch UNION the
    /// whole-model silhouette. The slicer's bed-collision warnings measure the full model
    /// extent, which hangs below the printed patch on models with overhangs (a lying animal's
    /// tail), silently eating the edge margin otherwise. `None` = use `loops` as before.
    pub bed_loops: Option<Vec<Vec<Point>>>,
    /// Height bands of a sliced sample (`gcode_collisions::layer_zones_from_sample`): per band
    /// what this object prints as support and as body. A neighbour's body must keep
    /// `LAYER_ZONE_GAP_MM` from these supports in the same band and vice versa — the slicer's
    /// real 3D supports (interfaces under the ears, trunks) instead of their first-layer shadow.
    /// Indices are absolute height bands: empty and one-sided bands must NOT be removed.
    /// Empty = no such pairs (the zone rule as before).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_zones: Vec<LayerZone>,
    pub width: f64,
    pub height: f64,
    pub min_z: f64,
    pub max_z: f64,
    pub triangle_count: usize,
}

impl Footprint {
    /// All known printed geometry projected onto XY, for bed/exclusion reservations only.
    /// Different height bands must be unioned, never even-odd concatenated.
    pub fn printed_extent_loops(&self) -> Vec<Vec<Point>> {
        let polygons = self
            .loops
            .iter()
            .chain(&self.body_loops)
            .chain(&self.support_landing_loops)
            .chain(self.bed_loops.iter().flatten())
            .chain(
                self.layer_zones
                    .iter()
                    .flat_map(|z| z.support_loops.iter().chain(&z.body_loops)),
            )
            .map(|ring| crate::geometry::polygon_from_loop(ring))
            .collect::<Vec<_>>();
        crate::geometry::union_exterior_loops(&polygons)
    }

    /// Retain the union of probe AND sample realizations at each absolute height band.
    pub fn merge_layer_zones(&mut self, zones: &[LayerZone]) {
        self.layer_zones
            .resize_with(self.layer_zones.len().max(zones.len()), LayerZone::default);
        let union = |a: &[Vec<Point>], b: &[Vec<Point>]| {
            let polygons = a
                .iter()
                .chain(b)
                .map(|ring| crate::geometry::polygon_from_loop(ring))
                .collect::<Vec<_>>();
            crate::geometry::union_exterior_loops(&polygons)
        };
        for (existing, added) in self.layer_zones.iter_mut().zip(zones) {
            existing.support_loops = union(&existing.support_loops, &added.support_loops);
            existing.body_loops = union(&existing.body_loops, &added.body_loops);
        }
    }

    /// A copy with every set of loops (patch, body, landing, bed, layer zones) passed through
    /// `transform`; sizes and heights are kept as they are.
    pub(crate) fn map_loops(
        &self,
        transform: impl Fn(&[Vec<Point>]) -> Vec<Vec<Point>>,
    ) -> Footprint {
        let mut mapped = self.clone();
        mapped.loops = transform(&self.loops);
        mapped.body_loops = transform(&self.body_loops);
        mapped.support_landing_loops = transform(&self.support_landing_loops);
        mapped.bed_loops = self.bed_loops.as_ref().map(|loops| transform(loops));
        mapped.layer_zones = self
            .layer_zones
            .iter()
            .map(|zone| LayerZone {
                support_loops: transform(&zone.support_loops),
                body_loops: transform(&zone.body_loops),
            })
            .collect();
        mapped
    }
}

/// `Provenance::flavour` of a print that came from a slicer-learned `Footprint`.
const LEGACY_FLAVOUR: &str = "legacy-footprint";

impl ObjectPrint {
    /// A slicer-learned (or mesh-projected) `Footprint` as one rotation class at 0°, so old
    /// `--finish-state` files load and learned footprints can feed the new core.
    ///
    /// The silhouette is the body zone (the whole patch when the footprint has no separate
    /// body). The support base is the patch minus the body. An empty landing means the body,
    /// as in the old two-zone rule. `layer_zones` become body and support bands with the same
    /// absolute indices; a legacy band cannot tell interfaces from other supports, so the
    /// interface bands stay empty. The brim is not separate (it is in the body zone), and the
    /// footprint has no parts, so one hull and one box cover the whole silhouette.
    pub fn from_legacy(footprint: &Footprint) -> ObjectPrint {
        let silhouette = if footprint.body_loops.is_empty() {
            footprint.loops.clone()
        } else {
            footprint.body_loops.clone()
        };
        let landing = if footprint.support_landing_loops.is_empty() {
            silhouette.clone()
        } else {
            footprint.support_landing_loops.clone()
        };
        let support_base = if footprint.body_loops.is_empty() {
            Loops::new()
        } else {
            let patch = even_odd_area(&footprint.loops);
            area_loops(&patch.difference(&even_odd_area(&footprint.body_loops)))
        };
        let bands = &footprint.layer_zones;
        ObjectPrint {
            rotation_deg: 0.0,
            part_hulls: convex_hull_of(&silhouette).into_iter().collect(),
            part_boxes: (!silhouette.is_empty())
                .then(|| Rect::of_points(silhouette.iter().flatten()))
                .into_iter()
                .collect(),
            silhouette,
            landing,
            brim: Loops::new(),
            body: bands.iter().map(|zone| zone.body_loops.clone()).collect(),
            support_base,
            support: bands
                .iter()
                .map(|zone| zone.support_loops.clone())
                .collect(),
            interface: vec![Loops::new(); bands.len()],
            provenance: Provenance {
                flavour: LEGACY_FLAVOUR.to_owned(),
                ..Provenance::default()
            },
        }
    }
}

/// Even-odd loops as one area: outer rings and holes XOR together. Every ring is oriented
/// counter-clockwise first, as `union_exterior_loops` does for geo's fill rule.
fn even_odd_area(loops: &[Vec<Point>]) -> MultiPolygon {
    loops
        .iter()
        .fold(MultiPolygon::new(Vec::new()), |area, ring| {
            let polygon = polygon_from_loop(ring).orient(Direction::Default);
            area.xor(&MultiPolygon::new(vec![polygon]))
        })
}

/// An area as loops (outer rings and holes), largest first, slivers dropped.
fn area_loops(area: &MultiPolygon) -> Loops {
    let mut loops: Loops = area
        .0
        .iter()
        .flat_map(|polygon| std::iter::once(polygon.exterior()).chain(polygon.interiors()))
        .filter_map(|ring| closed_loop(ring.coords(), (0.0, 0.0)))
        .collect();
    drop_specks(&mut loops);
    sort_largest_first(&mut loops);
    loops
}

/// The convex hull of all `loops` as one ring; `None` for no points.
fn convex_hull_of(loops: &[Vec<Point>]) -> Option<Vec<Point>> {
    let polygons = loops.iter().map(|ring| polygon_from_loop(ring)).collect();
    let hull = MultiPolygon::new(polygons).convex_hull();
    closed_loop(hull.exterior().coords(), (0.0, 0.0))
}

/// The pixel grid on which a probe-slice patch and a mesh silhouette are compared: one
/// range of pixel centres covering both, and the hole-filled silhouette loops.
struct ComparisonGrid<'a> {
    patch_loops: &'a [Vec<Point>],
    silhouette_loops: Vec<Vec<Point>>,
    resolution: f64,
    /// Inclusive pixel ranges `(min_ix, max_ix, min_iy, max_iy)`.
    range: (i32, i32, i32, i32),
}

impl<'a> ComparisonGrid<'a> {
    fn new(patch: &'a Footprint, silhouette: &Footprint, resolution: f64) -> Self {
        assert!(resolution > 0.0, "resolution must be positive");
        // Fill the silhouette's holes: each loop becomes a hole-free polygon, and their union
        // covers the holes (a hole ring's own polygon sits inside the body's).
        let silhouette_loops = filled_exterior_loops(&silhouette.loops);
        let patch_bounds = Rect::of_points(patch.loops.iter().flatten());
        let silhouette_bounds = Rect::of_points(silhouette_loops.iter().flatten());
        let range = (
            ((patch_bounds.min_x.min(silhouette_bounds.min_x)) / resolution - 0.5).floor() as i32,
            ((patch_bounds.max_x.max(silhouette_bounds.max_x)) / resolution - 0.5).ceil() as i32,
            ((patch_bounds.min_y.min(silhouette_bounds.min_y)) / resolution - 0.5).floor() as i32,
            ((patch_bounds.max_y.max(silhouette_bounds.max_y)) / resolution - 0.5).ceil() as i32,
        );
        Self {
            patch_loops: &patch.loops,
            silhouette_loops,
            resolution,
            range,
        }
    }

    /// Pixels whose centre lies inside `loops` (even-odd).
    fn pixels_inside(&self, loops: &[Vec<Point>]) -> HashSet<(i32, i32)> {
        let (min_ix, max_ix, min_iy, max_iy) = self.range;
        let mut pixels = HashSet::new();
        for iy in min_iy..=max_iy {
            let y = (f64::from(iy) + 0.5) * self.resolution;
            for ix in min_ix..=max_ix {
                let x = (f64::from(ix) + 0.5) * self.resolution;
                if point_in_loops(Point { x, y }, loops) {
                    pixels.insert((ix, iy));
                }
            }
        }
        // A valid sub-pixel silhouette may contain no cell centre. Returning an empty
        // set makes patch_overshoot_mm grow an empty set forever. Preserve its boundary
        // cells rather than silently erasing the model from this comparison.
        if pixels.is_empty() && !loops.is_empty() {
            let raster = crate::span_raster::SpanRaster::cover(
                loops,
                self.resolution,
                (min_ix, max_ix, min_iy, max_iy),
            );
            for (row, spans) in raster.rows.iter().enumerate() {
                for &(start, end) in spans {
                    pixels.extend((start..end).map(|x| (x, raster.min_y + row as i32)));
                }
            }
        }
        pixels
    }

    fn silhouette_pixels(&self) -> HashSet<(i32, i32)> {
        self.pixels_inside(&self.silhouette_loops)
    }

    fn patch_pixels(&self) -> HashSet<(i32, i32)> {
        self.pixels_inside(self.patch_loops)
    }
}

/// Whether every printed pixel of the first-layer `patch` lies within the mesh `silhouette`
/// plus `tolerance_mm`. A prismatic model's walls run along its outline, so its patch only
/// bulges half a line width (~0.25 mm) past it — up to ~1 mm on sharp convex tips, where the
/// nozzle bead miter extends width/2 / sin(θ/2) — while supports and brim wander millimetres
/// beyond. The silhouette's own holes (a keyring hole) don't count as sticking out: the
/// patch's collision mask deliberately fills them, so the comparison runs against the
/// hole-filled silhouette. Contained ⇒ the silhouette is the honest footprint AND rotations
/// stay allowed (packing a locked patch once cost a flat goat plate a full object: 29 instead
/// of 30); sticking out ⇒ only the patch keeps neighbours off the printed supports.
pub fn patch_within_silhouette(
    patch: &Footprint,
    silhouette: &Footprint,
    resolution: f64,
    tolerance_mm: f64,
) -> bool {
    let grid = ComparisonGrid::new(patch, silhouette, resolution);
    let radius = (tolerance_mm / resolution).round() as i32;
    let silhouette_pixels = grid.silhouette_pixels();
    let mut tolerated: HashSet<(i32, i32)> = silhouette_pixels.clone();
    if radius > 0 {
        for &(x, y) in &silhouette_pixels {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    tolerated.insert((x + dx, y + dy));
                }
            }
        }
    }
    grid.patch_pixels().is_subset(&tolerated)
}

/// The eight neighbours of a pixel, for growing a pixel set one Chebyshev ring at a time.
const NEIGHBOURS: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// How far the probe-slice `patch` extends past the mesh `silhouette` at thin tips, in mm
/// (0.0 when the patch stays inside). Bed bounds and exclusion keepouts are checked against
/// the silhouette dilated by exactly this amount: the wall-bead miter and Arachne's
/// flow-preserving spikes print past the mesh outline (measured 1.66 mm at a P1S goat's leg
/// tips), and nothing routes the prime line or the bed edge around them. Both footprints
/// must share the model center, which the caller guarantees. Distances grow in square
/// (Chebyshev) rings quantized to `resolution` — the same kernel `patch_within_silhouette`
/// tolerates.
pub fn patch_overshoot_mm(patch: &Footprint, silhouette: &Footprint, resolution: f64) -> f64 {
    let grid = ComparisonGrid::new(patch, silhouette, resolution);
    let silhouette_pixels = grid.silhouette_pixels();
    assert!(
        !silhouette_pixels.is_empty(),
        "silhouette must have nonzero area"
    );
    // Ring-grow the silhouette one Chebyshev pixel at a time until it covers every patch
    // pixel: the ring count at which the last spike is swallowed is the overshoot.
    let mut uncovered: HashSet<(i32, i32)> = grid
        .patch_pixels()
        .difference(&silhouette_pixels)
        .copied()
        .collect();
    if uncovered.is_empty() {
        return 0.0;
    }
    let mut covered = silhouette_pixels;
    let mut rings = 0u32;
    while !uncovered.is_empty() {
        rings += 1;
        let mut grown: HashSet<(i32, i32)> = covered.clone();
        for &(x, y) in &covered {
            for (dx, dy) in NEIGHBOURS {
                grown.insert((x + dx, y + dy));
            }
        }
        covered = grown;
        uncovered.retain(|pixel| !covered.contains(pixel));
    }
    f64::from(rings) * resolution
}

//! Body per absolute height band (design §4.2 `body[k]`): the projection of every normal
//! part's material inside the band's slab. The projection of a solid cut by a slab is the
//! union of its slab-clipped surface triangles and its two cut faces (caps), so no section is
//! sampled and nothing between samples is missed.
//!
//! Slab of band k: `[k·band − pad, (k+1)·band + pad]`. A printed layer occupies
//! `(z_prev, z]` and belongs to every band that interval meets (lead decision on bands,
//! 00-review fix 9), but it prints the section at its mid-height. With `pad` = half the
//! largest layer height every such mid-height lies inside the slab, so the band contains
//! the printed body of every layer that reaches into it. Measured on the owner goats
//! (P1S/A2L/U1, 8 rotations, corpus run 1): without the pad, body lines stick out of their
//! band by up to 0.99 mm; with it, by at most 0.033 mm.
//!
//! That remainder is the bead itself: the slicer spaces walls by `w − h(1 − π/4)` and puts
//! the outer wall's spacing edge on the outline, so the w-wide bead bulges `h(1 − π/4)/2`
//! (0.030 mm at h = 0.28) past the mesh. `bulge` grows every band by that much, because the
//! kinds are material extents with the line width included (design §4.2).

use i_overlay::core::fill_rule::FillRule;
use rayon::prelude::*;

use super::area::{
    IntRings, NM_PER_MM, counter_clockwise, doubled_area, int_point, loops_from_rings, resolve,
};
use super::offset::offset_area;
use super::section::raw_rings;
use crate::embedded_mesh::{ObjectMesh, Part};
use crate::object_print::LAYER_BAND_MM;
use crate::object_print::Loops;

/// Mitre limit of the bulge growth. Any limit keeps the growth conservative: a mitre or its
/// square cut always covers the round offset of the vertex. 1.5 still mitres a right angle
/// (a box grows by exactly the bulge on every side) and keeps the reach of the sharpest cut
/// at √(1.5² + 1) = 1.80 bulges (`SILHOUETTE_REACH`); 3 reached 3.16, which the silhouette
/// would have to pay along its whole outline.
const BULGE_MITER_LIMIT: f64 = 1.5;

/// How far, in bulges, a mitred growth with limit `BULGE_MITER_LIMIT` reaches from the
/// vertex it grows: the tip of a mitre lies within the limit, the corners of a square cut at
/// most √(limit² + 1) away (the supremum, approached by the sharpest corners).
const SILHOUETTE_REACH: f64 = 1.802_775_637_731_995; // √(1.5² + 1)

/// Grid slack of the silhouette growth: every offset point is rounded to the 1 nm grid.
const SILHOUETTE_SLACK_MM: f64 = 2.0e-6;

/// How the body is cut into height bands and how far printed walls reach past the mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyBands {
    /// Band height (`LAYER_BAND_MM`).
    pub band_mm: f64,
    /// Extra height below and above each band (see the module comment).
    pub pad_mm: f64,
    /// Outward growth of every band for the wall bead (see the module comment).
    pub bulge_mm: f64,
}

impl BodyBands {
    /// The bands of a print whose largest layer height is `max_layer_height_mm`.
    pub fn printed(max_layer_height_mm: f64) -> BodyBands {
        BodyBands {
            band_mm: LAYER_BAND_MM,
            pad_mm: max_layer_height_mm * 0.5,
            bulge_mm: wall_bulge_mm(max_layer_height_mm),
        }
    }

    /// Mesh geometry only: band k covers exactly `[k·band, (k+1)·band]`, no bead.
    pub fn mesh_only() -> BodyBands {
        BodyBands {
            band_mm: LAYER_BAND_MM,
            pad_mm: 0.0,
            bulge_mm: 0.0,
        }
    }

    /// `(lo, hi)` of band `band`'s slab, never below the bed.
    pub fn slab(&self, band: usize) -> (f64, f64) {
        let lo = band as f64 * self.band_mm - self.pad_mm;
        let hi = (band + 1) as f64 * self.band_mm + self.pad_mm;
        (lo.max(0.0), hi)
    }

    /// Number of bands whose slab starts below `height` (0 for a non-positive band height).
    pub fn count(&self, height: f64) -> usize {
        if self.band_mm <= 0.0 {
            return 0;
        }
        let mut count = 0;
        while self.slab(count).0 < height {
            count += 1;
        }
        count
    }
}

/// How far an outer wall bead of layer height `layer_height_mm` reaches past the outline:
/// half the difference between its width and its spacing, `h(1 − π/4)/2`.
pub fn wall_bulge_mm(layer_height_mm: f64) -> f64 {
    layer_height_mm * (1.0 - std::f64::consts::FRAC_PI_4) * 0.5
}

/// `body[k]` for every band up to the object's top, in the mesh's frame. Bands are
/// independent and each is built single-threaded, so running them in parallel keeps the bits.
pub fn body_bands(mesh: &ObjectMesh, bands: &BodyBands) -> Vec<Loops> {
    let height = mesh.height();
    let parts: Vec<PartSpans> = mesh.normal_parts().map(PartSpans::new).collect();
    (0..bands.count(height))
        .into_par_iter()
        .map(|band| {
            let (lo, hi) = bands.slab(band);
            let area = without_filled_holes(slab_area(&parts, lo, hi, height), bands.bulge_mm);
            loops_from_rings(&offset_area(&area, bands.bulge_mm, BULGE_MITER_LIMIT))
        })
        .collect()
}

/// `area` without the holes a growth by `grow_mm` fills anyway: a hole of area below
/// π·grow² has an inscribed radius below `grow`, so every point of it lies within `grow` of
/// its rim and the mitred growth (which covers the round one) closes it. Same result, but
/// the union of a slab's triangles leaves hundreds of such cracks (10⁻⁷ mm², seeoptics
/// keychain: ~200 per band) and each one, grown, turns into a self-crossing star: the
/// resolve took 0.5 s per band instead of milliseconds.
fn without_filled_holes(area: IntRings, grow_mm: f64) -> IntRings {
    if grow_mm <= 0.0 {
        return area;
    }
    let grow_nm = grow_mm * NM_PER_MM;
    let filled_doubled = 2.0 * std::f64::consts::PI * grow_nm * grow_nm;
    area.into_iter()
        .filter(|ring| {
            let doubled = doubled_area(ring);
            doubled > 0 || (-doubled) as f64 >= filled_doubled
        })
        .collect()
}

/// The silhouette: the projection of every normal part (the union of its triangles' shadows,
/// which holds every slab of it) grown by `SILHOUETTE_REACH` bulges, so it contains each
/// body band: a band is its slab grown by the bulge, and no point of that growth lies
/// farther than the reach from the slab.
///
/// Not the union of the bands: their outlines run along the same walls a few nm apart, and
/// their union is a sawtooth (seeoptics keychain: 57 528 vertices against 1 750, 1.5 s per
/// rotation against 2 ms) that every later exact check pays for.
pub fn silhouette(mesh: &ObjectMesh, bands: &BodyBands) -> Loops {
    let mut shadows = IntRings::new();
    for part in mesh.normal_parts() {
        for index in 0..part.triangles.len() {
            let ring = part
                .corners(index)
                .iter()
                .map(|corner| int_point(corner[0], corner[1]))
                .collect();
            shadows.extend(counter_clockwise(ring));
        }
    }
    if bands.bulge_mm <= 0.0 {
        return loops_from_rings(&resolve(&shadows, FillRule::Positive));
    }
    let grow_mm = bands.bulge_mm * SILHOUETTE_REACH + SILHOUETTE_SLACK_MM;
    let projection = without_filled_holes(resolve(&shadows, FillRule::Positive), grow_mm);
    loops_from_rings(&offset_area(&projection, grow_mm, BULGE_MITER_LIMIT))
}

/// The XY outline of the part of a triangle with `lo <= z <= hi`; empty when the triangle
/// misses the slab. Up to five points, in the triangle's own winding.
pub fn slab_polygon(corners: &[[f64; 3]; 3], lo: f64, hi: f64) -> Vec<[f64; 2]> {
    let above_lo = clip_by_height(corners, lo, true);
    let inside = clip_by_height(&above_lo, hi, false);
    inside.iter().map(|point| [point[0], point[1]]).collect()
}

/// A normal part with each triangle's z span, so a slab only visits triangles it meets.
struct PartSpans<'a> {
    part: &'a Part,
    spans: Vec<(f64, f64)>,
}

impl<'a> PartSpans<'a> {
    fn new(part: &'a Part) -> Self {
        let spans = (0..part.triangles.len())
            .map(|index| {
                let corners = part.corners(index);
                let zs = corners.map(|corner| corner[2]);
                (zs[0].min(zs[1]).min(zs[2]), zs[0].max(zs[1]).max(zs[2]))
            })
            .collect();
        PartSpans { part, spans }
    }
}

fn slab_area(parts: &[PartSpans<'_>], lo: f64, hi: f64, height: f64) -> IntRings {
    let mut rings = IntRings::new();
    for spans in parts {
        for (index, &(low, high)) in spans.spans.iter().enumerate() {
            if high < lo || low > hi {
                continue;
            }
            let polygon = slab_polygon(&spans.part.corners(index), lo, hi);
            let ring = polygon
                .iter()
                .map(|point| int_point(point[0], point[1]))
                .collect();
            rings.extend(counter_clockwise(ring));
        }
        for cap in [lo, hi] {
            if cap > 0.0 && cap < height {
                rings.extend(resolve(&raw_rings(spans.part, cap), FillRule::NonZero));
            }
        }
    }
    resolve(&rings, FillRule::Positive)
}

/// Sutherland-Hodgman against one horizontal plane: keeps `z >= level` when `keep_above`,
/// else `z <= level`.
fn clip_by_height(polygon: &[[f64; 3]], level: f64, keep_above: bool) -> Vec<[f64; 3]> {
    let inside = |point: &[f64; 3]| {
        if keep_above {
            point[2] >= level
        } else {
            point[2] <= level
        }
    };
    let mut out = Vec::with_capacity(polygon.len() + 2);
    for (index, current) in polygon.iter().enumerate() {
        let next = &polygon[(index + 1) % polygon.len()];
        if inside(current) {
            out.push(*current);
        }
        if inside(current) != inside(next) {
            let t = (level - current[2]) / (next[2] - current[2]);
            out.push([
                current[0] + (next[0] - current[0]) * t,
                current[1] + (next[1] - current[1]) * t,
                level,
            ]);
        }
    }
    out
}

//! Stage A2: what the support generator sees of the object on every layer (spec 01 §1.3-§1.6),
//! on the flavour grid:
//!
//! - `area` L_k: per volume the chained section loops resolved non-zero, closed by the slice
//!   closing radius (grow then shrink, mitre 3) and Douglas-Peucker'd at 2.5 µm; normal parts
//!   united and negative parts subtracted in volume order. With several regions the union is
//!   taken after a 10-unit safety grow (`ClipperSafetyOffset`), except on the elephant-foot
//!   layers, where the slicer restores the merged uncompensated regions closed by 1.5·r_c.
//! - `extrudable` X_k = L_k ∩ ((L_k ⊖ w/2) ⊕ w): features an erosion by half a line width
//!   deletes drop out (spec 01 §1.6).
//! - `outline` O_l = S(resolution)(L_l), the clearance volumes' input (spec 02 §2.1).
//!
//! The section loops come from `mesh_section` (S1); every boolean here runs on the flavour
//! grid through `polygon_ops`.

use super::SupportError;
use super::flavour::Flavour;
use super::layers::LayerGrid;
use super::settings::{Derived, SupportSettings};
use crate::embedded_mesh::{ObjectMesh, Part, PartKind};
use crate::mesh_section::raw_section_loops;
use crate::polygon_ops::{
    DEFAULT_MITER_LIMIT, FillRule, Grid, IntPoint, Join, Rings, difference, douglas_peucker,
    intersection, offset, resolve, simplify,
};

/// Every section ring is Douglas-Peucker'd with this tolerance whenever `resolution` exceeds
/// 0.001 mm; the resolution value itself is not used (O `PrintObjectSlice.cpp:170-172`).
const SECTION_SIMPLIFY_MM: f64 = 0.0025;

/// The resolution above which sections are simplified at all.
const SIMPLIFY_RESOLUTION_MM: f64 = 0.001;

/// Regions are united after growing each by this many grid units (`ClipperSafetyOffset`).
const REGION_SAFETY_UNITS: f64 = 10.0;

/// The elephant-foot restore closes merged regions by this multiple of the closing radius.
const ELEPHANT_RESTORE_FACTOR: f64 = 1.5;

/// The object as the support generator sees it, one entry per layer of the grid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerSections {
    /// L_k.
    pub area: Vec<Rings>,
    /// X_k.
    pub extrudable: Vec<Rings>,
    /// O_l = S(res)(L_l).
    pub outline: Vec<Rings>,
}

/// Sections of every layer of `grid`. The mesh must be in the slicer's object frame. Trailing
/// layers whose section is empty are dropped from the result (spec 01 §1.2 rule 7); callers
/// truncate the grid to `area.len()`.
pub fn layer_sections(
    mesh: &ObjectMesh,
    grid: &LayerGrid,
    settings: &SupportSettings,
    derived: &Derived,
) -> Result<LayerSections, SupportError> {
    let flavour = derived.flavour;
    let units = derived.grid;
    let multi_region = has_several_regions(mesh);
    let elephant_layers = elephant_foot_layers(settings, flavour);
    let mut area = Vec::with_capacity(grid.len());
    for (index, layer) in grid.layers.iter().enumerate() {
        check_disjoint_parts(mesh, layer.slice_z, flavour)?;
        let restore = multi_region && index < elephant_layers;
        area.push(object_area(
            mesh,
            layer.slice_z,
            settings,
            units,
            multi_region,
            restore,
        ));
    }
    while area.last().is_some_and(Vec::is_empty) {
        area.pop();
    }
    let width = units.scaled(extrusion_width(settings, flavour));
    let extrudable = area
        .iter()
        .map(|rings| extrudable_part(rings, width))
        .collect();
    let outline = area
        .iter()
        .map(|rings| simplify(rings, derived.resolution as f64, flavour.strictly_simple()))
        .collect();
    Ok(LayerSections {
        area,
        extrudable,
        outline,
    })
}

/// Sections of the support enforcer or blocker volumes at every slice plane (resolved
/// non-zero, no closing): the start of E_k / K_k before paint is appended (spec 01 §2.6
/// step 7).
pub fn volume_layers(
    mesh: &ObjectMesh,
    kind: PartKind,
    grid: &LayerGrid,
    units: Grid,
) -> Vec<Rings> {
    grid.layers
        .iter()
        .map(|layer| {
            let rings: Rings = mesh
                .parts
                .iter()
                .filter(|part| part.kind == kind)
                .flat_map(|part| units.rings(&raw_section_loops(part, layer.slice_z)))
                .collect();
            resolve(&rings, FillRule::NonZero)
        })
        .collect()
}

/// The line width w of the extrudable filter in mm: B reads the raw `line_width` number (a
/// percentage would be misread), Orca resolves it against nozzle 0 (spec 01 §1.6).
pub fn extrusion_width(settings: &SupportSettings, flavour: Flavour) -> f64 {
    if flavour.raw_line_width() {
        settings.line_width.value
    } else {
        settings.line_width.absolute(settings.nozzle_diameter)
    }
}

/// X = L ∩ ((L ⊖ w/2) ⊕ w), mitred joins with limit 3.
pub fn extrudable_part(area: &Rings, width_units: f64) -> Rings {
    if area.is_empty() {
        return Rings::new();
    }
    let eroded = offset(area, -width_units / 2.0, Join::Miter(DEFAULT_MITER_LIMIT));
    let regrown = offset(&eroded, width_units, Join::Miter(DEFAULT_MITER_LIMIT));
    intersection(area, &regrown)
}

/// More than one printing region: several normal parts, or colour paint (which splits a part
/// into per-filament regions). Approximation: parts on one extruder with equal settings
/// would form one region; the 10-unit safety grow it decides is far below every margin.
fn has_several_regions(mesh: &ObjectMesh) -> bool {
    let normal: Vec<&Part> = mesh.normal_parts().collect();
    normal.len() > 1 || normal.iter().any(|part| !part.paint_color.is_empty())
}

/// How many bottom layers get the elephant-foot restore (0 when compensation is off).
fn elephant_foot_layers(settings: &SupportSettings, flavour: Flavour) -> usize {
    if settings.elephant_foot_compensation <= 0.0 {
        0
    } else if flavour.elephant_foot_layer_setting() {
        settings.elephant_foot_layers.max(0) as usize
    } else {
        1
    }
}

/// L at plane `z`: per-volume sections combined in volume order (a negative part removes
/// what the parts before it put there, spec 01 §1.4).
fn object_area(
    mesh: &ObjectMesh,
    z: f64,
    settings: &SupportSettings,
    units: Grid,
    multi_region: bool,
    restore: bool,
) -> Rings {
    let mut regions: Vec<Rings> = Vec::new();
    for part in &mesh.parts {
        match part.kind {
            PartKind::Normal => regions.push(part_area(part, z, settings, units)),
            PartKind::Negative => {
                let cut = part_area(part, z, settings, units);
                regions = regions
                    .iter()
                    .map(|region| difference(region, &cut))
                    .collect();
            }
            _ => {}
        }
    }
    let all: Rings = regions.into_iter().flatten().collect();
    if !multi_region {
        return resolve(&all, FillRule::NonZero);
    }
    if restore {
        let epsilon = units.scaled(ELEPHANT_RESTORE_FACTOR * settings.slice_closing_radius);
        let grown = offset(&all, epsilon, Join::Miter(DEFAULT_MITER_LIMIT));
        return offset(&grown, -epsilon, Join::Miter(DEFAULT_MITER_LIMIT));
    }
    offset(&all, REGION_SAFETY_UNITS, Join::Miter(DEFAULT_MITER_LIMIT))
}

/// One volume's section: resolved non-zero, closed, simplified (spec 01 §1.3 steps 3-5).
fn part_area(part: &Part, z: f64, settings: &SupportSettings, units: Grid) -> Rings {
    let raw = units.rings(&raw_section_loops(part, z));
    let mut area = resolve(&raw, FillRule::NonZero);
    if settings.slice_closing_radius > 0.0 {
        let radius = units.scaled(settings.slice_closing_radius);
        let grown = offset(&area, radius, Join::Miter(DEFAULT_MITER_LIMIT));
        area = offset(&grown, -radius, Join::Miter(DEFAULT_MITER_LIMIT));
    }
    if settings.resolution > SIMPLIFY_RESOLUTION_MM {
        let tolerance = units.scaled(SECTION_SIMPLIFY_MM);
        area = area
            .iter()
            .map(|ring| douglas_peucker_ring(ring, tolerance))
            .filter(|ring| ring.len() >= 3)
            .collect();
    }
    area
}

/// Douglas-Peucker on a closed ring run as the open path back to its first vertex.
fn douglas_peucker_ring(ring: &[IntPoint], tolerance: f64) -> Vec<IntPoint> {
    let mut path = ring.to_vec();
    path.push(ring[0]);
    let mut simplified = douglas_peucker(&path, tolerance);
    simplified.pop();
    simplified
}

/// O/R/S keep only the first part's slice when printable parts sharing a plane have pairwise
/// disjoint XY boxes (spec 01 §1.4, Q5); unverified, so refused (design §5.7).
fn check_disjoint_parts(mesh: &ObjectMesh, z: f64, flavour: Flavour) -> Result<(), SupportError> {
    if flavour.keeps_disjoint_parts() {
        return Ok(());
    }
    let spanning: Vec<([f64; 3], [f64; 3])> = mesh
        .normal_parts()
        .filter_map(Part::bounds)
        .filter(|(min, max)| min[2] <= z && z <= max[2])
        .collect();
    if spanning.len() < 2 {
        return Ok(());
    }
    let overlap = |a: &([f64; 3], [f64; 3]), b: &([f64; 3], [f64; 3])| {
        a.0[0] <= b.1[0] && b.0[0] <= a.1[0] && a.0[1] <= b.1[1] && b.0[1] <= a.1[1]
    };
    let any_overlap = (0..spanning.len())
        .any(|i| (i + 1..spanning.len()).any(|j| overlap(&spanning[i], &spanning[j])));
    if any_overlap {
        Ok(())
    } else {
        Err(SupportError::unsupported(
            "parts",
            "disjoint",
            "Orca keeps only the first of parts with disjoint boxes on a layer (spec 01 §1.4 Q5)",
        ))
    }
}

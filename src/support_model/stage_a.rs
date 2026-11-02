//! Stage A of the support model (design §5.2 A1-A5) for one (mesh, paint, settings, flavour,
//! rotation class): deterministic, single-threaded, run once and shared by every ensemble
//! member of stage B.

use super::SupportError;
use super::clearance::{BedPlacement, Clearance, bed_obstacle};
use super::flavour::Flavour;
use super::layers::LayerGrid;
use super::overhangs::{Overhangs, PaintInput, detect_overhangs};
use super::sections::{LayerSections, layer_sections};
use super::settings::{Derived, SupportSettings, check_supported};
use crate::embedded_mesh::ObjectMesh;
use crate::polygon_ops::Rings;

/// The object must stand on the bed: its lowest normal vertex within this of z = 0 (mm).
const ON_BED_TOLERANCE_MM: f64 = 1e-6;

/// What stage A needs.
pub struct StageAInput<'a> {
    /// All parts with paint, in the plate-aligned frame of the rotation class
    /// (`ObjectMesh::rotated`): placing the result is a pure translation.
    pub mesh: &'a ObjectMesh,
    pub settings: &'a SupportSettings,
    pub flavour: Flavour,
    /// B only: the bed and where the slicing copy's frame origin (the input frame's origin)
    /// stands on the plate. `None` builds no bed obstacle.
    pub bed: Option<&'a BedPlacement>,
}

/// Stage A results, in the slicer's own object frame.
pub struct StageA {
    pub flavour: Flavour,
    pub derived: Derived,
    pub grid: LayerGrid,
    /// The slicer centres a rotated object on its normal parts' XY box: slicer frame =
    /// input frame − `frame_shift`. Stage-B geometry is moved back by + `frame_shift`.
    pub frame_shift: [f64; 2],
    pub sections: LayerSections,
    pub paint: PaintInput,
    pub overhangs: Overhangs,
    pub clearance: Clearance,
}

/// Runs A1-A5. Unsupported settings are refused before any geometry is computed (§5.7).
pub fn stage_a(input: &StageAInput) -> Result<StageA, SupportError> {
    let settings = input.settings;
    let flavour = input.flavour;
    check_supported(settings, flavour)?;
    let (mesh, frame_shift) = slicer_frame(input.mesh);
    let (min, _) = mesh.normal_bounds();
    if min[2].abs() > ON_BED_TOLERANCE_MM {
        return Err(SupportError::Invalid(format!(
            "the object does not stand on the bed (lowest normal vertex at z = {})",
            min[2]
        )));
    }
    let derived = Derived::new(settings, flavour);
    let mut grid = LayerGrid::new(settings, mesh.height())?;
    let sections = layer_sections(&mesh, &grid, settings, &derived)?;
    grid.truncate(sections.area.len());
    let paint = PaintInput::from_mesh(&mesh, &grid, flavour)?;
    let overhangs = detect_overhangs(&grid, &sections, &paint, settings, flavour)?;
    let border = match input.bed {
        Some(bed) if flavour.bed_is_obstacle() => {
            let placement = BedPlacement {
                object_origin: [
                    bed.object_origin[0] + frame_shift[0],
                    bed.object_origin[1] + frame_shift[1],
                ],
                ..bed.clone()
            };
            bed_obstacle(&placement, flavour.grid())
        }
        _ => Rings::new(),
    };
    let clearance = Clearance::new(&sections.outline, border, &derived);
    Ok(StageA {
        flavour,
        derived,
        grid,
        frame_shift,
        sections,
        paint,
        overhangs,
        clearance,
    })
}

/// The mesh re-centred on its normal parts' XY box, and the shift that was removed.
fn slicer_frame(mesh: &ObjectMesh) -> (ObjectMesh, [f64; 2]) {
    let (min, max) = mesh.normal_bounds();
    let shift = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
    let mut centred = mesh.clone();
    for part in &mut centred.parts {
        for vertex in &mut part.vertices {
            vertex[0] -= shift[0];
            vertex[1] -= shift[1];
        }
    }
    (centred, shift)
}

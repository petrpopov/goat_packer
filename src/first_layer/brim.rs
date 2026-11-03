//! The object brim (spec 03 §5): per volume group and island, a ring area from
//! `brim_object_gap` to gap + width around the first-layer contour (and inside holes for inner
//! brims), width from `brim_width` or the auto rule; the object's own no-brim set removed
//! (closed by one brim spacing in BambuStudio and Snapmaker Orca, 03 §5.3); then grown to the
//! loops' material. The plate's trimming (foreign objects, supports, tower, exclusions) is
//! not applied, so the kind is a superset of what prints; the matrix lets brims overlap
//! anything (design §7.3), so it serves reports and the soft class only.

use super::brim_groups::{group_material, volume_groups};
use super::brim_width::{auto_width, island_width, max_print_speed, quantised};
use super::grow::{boundary_band, grow_round, intersect, ring_between, subtract};
use super::islands::Island;
use super::landing::first_layer_height;
use super::project::{ProjectSettings, SlicerBuild, Unsupported, bead_spacing};
use crate::embedded_mesh::ObjectMesh;
use crate::geometry::Point;
use crate::mesh_section::area::union_of_areas;
use crate::object_print::Loops;

/// Extra growth of the loops' material for the slicer's chordal round offsets: the spire
/// probes' innermost loop reaches up to 0.024 mm closer to the object than an ideal ring
/// (corpus, all three printers and both rotations).
const LOOP_CHORD_ALLOWANCE_MM: f64 = 0.03;

/// The brim of one rotation class.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Brim {
    /// Material extent, even-odd loops; empty when nothing prints.
    pub loops: Loops,
    /// Width per island in group order, 0 where the island gets none.
    pub island_widths_mm: Vec<f64>,
}

impl Brim {
    pub fn max_width_mm(&self) -> f64 {
        self.island_widths_mm.iter().copied().fold(0.0, f64::max)
    }
}

/// Which sides of an island get a brim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Sides {
    None,
    Auto,
    Outer,
    Inner,
    Both,
}

/// The brim of `mesh` turned by `rotation_deg`. `part_filaments` gives each part's 1-based
/// filament in `mesh.parts` order (for adhesion and thermal length); empty means unknown,
/// which takes the strongest values of the project's filament types.
pub fn brim(
    mesh: &ObjectMesh,
    part_filaments: &[u32],
    settings: &ProjectSettings,
    build: SlicerBuild,
    rotation_deg: f64,
) -> Result<Brim, Unsupported> {
    let sides = brim_sides(settings)?;
    if sides == Sides::None {
        return Ok(Brim::default());
    }
    let first_layer = first_layer_height(settings)?;
    let nozzle = settings.number("nozzle_diameter").unwrap_or(0.4);
    let line_width = settings
        .length_or_percent("initial_layer_line_width", nozzle)
        .unwrap_or(1.25 * nozzle);
    let spacing = bead_spacing(line_width, first_layer);
    let rotated = mesh.rotated(rotation_deg);
    let groups = volume_groups(&rotated, settings, first_layer / 2.0);
    let gap = settings.number("brim_object_gap").unwrap_or(0.0);
    // The loops' material passes the brim area by half the width-spacing difference
    // (03 §7 table, Brim row): the first loop runs half a spacing inside the area's edge.
    let bead_excess = (line_width - spacing) / 2.0;
    let fixed_width = quantised(settings.number("brim_width").unwrap_or(0.0), spacing);
    let speed = max_print_speed(settings);
    let mut areas = Vec::new();
    let mut island_widths_mm = Vec::new();
    for group in &groups {
        let group_width = match sides {
            Sides::Auto => {
                let material = group_material(&group.parts, part_filaments, settings);
                quantised(auto_width(&group.shape, material, speed, build), spacing)
            }
            _ => fixed_width,
        };
        for island in &group.islands {
            let width = match sides {
                Sides::Auto => island_width(
                    group_width,
                    group.shape.height,
                    &island.shape(),
                    spacing,
                    build,
                ),
                _ => group_width,
            };
            island_widths_mm.push(width);
            if width > 0.0 {
                areas.extend(island_areas(island, sides, gap, width));
            }
        }
    }
    if areas.is_empty() {
        return Ok(Brim {
            loops: Loops::new(),
            island_widths_mm,
        });
    }
    let islands_area: Loops = groups
        .iter()
        .flat_map(|group| group.area.iter().cloned())
        .collect();
    let area = subtract(
        &union_of_areas(areas.iter()),
        &no_brim_set(&islands_area, gap, spacing, build),
    );
    Ok(Brim {
        loops: grow_round(&area, bead_excess + LOOP_CHORD_ALLOWANCE_MM),
        island_widths_mm,
    })
}

/// The object's own part of the no-brim set: its first-layer islands grown by the gap. B and
/// S close it by one brim spacing, so no brim squeezes between islands closer than about two
/// spacings; Orca 2.4/2.5 have the closing commented out (03 §5.3).
fn no_brim_set(islands_area: &[Vec<Point>], gap: f64, spacing: f64, build: SlicerBuild) -> Loops {
    let set = grow_round(islands_area, gap);
    if !matches!(build, SlicerBuild::BambuStudio | SlicerBuild::SnapmakerOrca) {
        return set;
    }
    let grown = grow_round(&set, spacing);
    subtract(&grown, &boundary_band(&grown, spacing))
}

fn brim_sides(settings: &ProjectSettings) -> Result<Sides, Unsupported> {
    if let Some(rafts) = settings.number("raft_layers").filter(|rafts| *rafts > 0.0) {
        return Err(Unsupported::new(
            "raft_layers",
            rafts,
            "objects on a raft get no brim of this kind",
        ));
    }
    match settings.text("brim_type").unwrap_or("auto_brim") {
        "no_brim" => Ok(Sides::None),
        "auto_brim" => Ok(Sides::Auto),
        "outer_only" => Ok(Sides::Outer),
        "inner_only" => Ok(Sides::Inner),
        "outer_and_inner" => Ok(Sides::Both),
        other => Err(Unsupported::new(
            "brim_type",
            other,
            "brim ears and painted brims are not described (03 §5)",
        )),
    }
}

/// The brim areas of one island: outer rings from the gap to gap + width, inner bands along
/// its holes.
fn island_areas(island: &Island, sides: Sides, gap: f64, width: f64) -> Vec<Loops> {
    let mut areas = Vec::new();
    if matches!(sides, Sides::Auto | Sides::Outer | Sides::Both) {
        areas.push(ring_between(
            std::slice::from_ref(&island.contour),
            gap,
            gap + width,
        ));
    }
    if matches!(sides, Sides::Inner | Sides::Both) {
        for hole in &island.holes {
            let hole_area = std::slice::from_ref(hole);
            let band = intersect(hole_area, &boundary_band(hole_area, gap + width));
            areas.push(subtract(&band, &boundary_band(hole_area, gap)));
        }
    }
    areas
}

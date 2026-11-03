//! First-layer volume groups (O `PrintObjectSlice.cpp:517-590`): the unit the auto-brim
//! width is sized for. Each normal part's section at the middle of layer 0, negative parts
//! later in volume order removed; parts whose sections touch after growing each by 0.05 mm
//! form one group.

use super::brim_width::{AreaMoments, GroupShape, Material, adhesion_of, thermal_length_of};
use super::grow::{grow_round, intersect, subtract};
use super::islands::{Island, islands};
use super::project::ProjectSettings;
use crate::embedded_mesh::{ObjectMesh, PartKind};
use crate::geometry::{Point, Rect};
use crate::mesh_section::area::union_of_areas;
use crate::mesh_section::{SectionOptions, part_section};
use crate::object_print::Loops;

/// Volumes whose first-layer slices come this close (grown by it each) form one group.
const GROUP_TOUCH_MM: f64 = 0.05;
/// `slice_closing_radius` of every owner project, used when a project omits it.
const DEFAULT_CLOSING_RADIUS_MM: f64 = 0.049;

/// One volume group: the normal parts whose first-layer slices touch.
pub struct VolumeGroup {
    pub parts: Vec<usize>,
    pub area: Loops,
    pub islands: Vec<Island>,
    pub shape: GroupShape,
}

/// The volume groups of `mesh` at the slice plane `slice_z`.
pub fn volume_groups(
    mesh: &ObjectMesh,
    settings: &ProjectSettings,
    slice_z: f64,
) -> Vec<VolumeGroup> {
    let closing = settings
        .number("slice_closing_radius")
        .unwrap_or(DEFAULT_CLOSING_RADIUS_MM);
    let options = SectionOptions::slicer(closing);
    let sections: Vec<(usize, Loops)> = mesh
        .parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.kind == PartKind::Normal)
        .map(|(index, part)| {
            let mut section = part_section(part, slice_z, &options);
            for later in mesh.parts[index + 1..]
                .iter()
                .filter(|p| p.kind == PartKind::Negative)
            {
                section = subtract(&section, &part_section(later, slice_z, &options));
            }
            (index, section)
        })
        .filter(|(_, section)| !section.is_empty())
        .collect();
    touching_groups(&sections)
        .into_iter()
        .map(|members| {
            let parts: Vec<usize> = members.iter().map(|&m| sections[m].0).collect();
            let area = union_of_areas(members.iter().map(|&m| &sections[m].1));
            group_of(mesh, parts, area)
        })
        .collect()
}

/// Indices of `sections` grouped by touch, each group in index order.
fn touching_groups(sections: &[(usize, Loops)]) -> Vec<Vec<usize>> {
    let grown: Vec<Loops> = sections
        .iter()
        .map(|(_, section)| grow_round(section, GROUP_TOUCH_MM))
        .collect();
    let mut group_of: Vec<usize> = (0..sections.len()).collect();
    for i in 0..sections.len() {
        for j in i + 1..sections.len() {
            if !intersect(&grown[i], &grown[j]).is_empty() {
                let (keep, drop) = (group_of[i].min(group_of[j]), group_of[i].max(group_of[j]));
                group_of
                    .iter_mut()
                    .filter(|g| **g == drop)
                    .for_each(|g| *g = keep);
            }
        }
    }
    let mut ids: Vec<usize> = group_of.clone();
    ids.sort_unstable();
    ids.dedup();
    ids.iter()
        .map(|id| {
            (0..sections.len())
                .filter(|m| group_of[*m] == *id)
                .collect()
        })
        .collect()
}

fn group_of(mesh: &ObjectMesh, parts: Vec<usize>, area: Loops) -> VolumeGroup {
    let (low, high) = parts
        .iter()
        .filter_map(|&index| mesh.parts[index].bounds())
        .fold(
            (f64::INFINITY, f64::NEG_INFINITY),
            |(low, high), (min, max)| (low.min(min[2]), high.max(max[2])),
        );
    let islands = islands(&area);
    let contours: Vec<Point> = islands
        .iter()
        .flat_map(|i| i.contour.iter().copied())
        .collect();
    let bounds = Rect::of_points(contours.iter());
    let moments = AreaMoments::of_rings(&area);
    VolumeGroup {
        parts,
        shape: GroupShape {
            height: (high - low).max(0.0),
            central_second: moments.central_second(),
            size: (bounds.max_x - bounds.min_x, bounds.max_y - bounds.min_y),
        },
        islands,
        area,
    }
}

/// The group's adhesion (the strongest of its filaments, the safe side for a brim) and
/// thermal length (the shortest of its volumes, 03 §5.2).
pub fn group_material(
    parts: &[usize],
    part_filaments: &[u32],
    settings: &ProjectSettings,
) -> Material {
    let types = settings.texts("filament_type");
    let known: Vec<&str> = parts
        .iter()
        .filter_map(|&part| part_filaments.get(part))
        .filter_map(|&filament| types.get((filament as usize).checked_sub(1)?).copied())
        .collect();
    let used = if known.is_empty() { types } else { known };
    if used.is_empty() {
        return Material {
            adhesion: 1.0,
            thermal_length: thermal_length_of(""),
        };
    }
    Material {
        adhesion: used.iter().map(|t| adhesion_of(t)).fold(f64::MIN, f64::max),
        thermal_length: used
            .iter()
            .map(|t| thermal_length_of(t))
            .fold(f64::MAX, f64::min),
    }
}

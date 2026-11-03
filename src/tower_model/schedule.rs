//! Filaments per layer, tool changes per layer and the tower top (spec 04 §4.1-§4.2), from
//! the mesh alone: every triangle reports the colour states of its paint tree on every layer
//! whose slice plane its z-extent spans; unpainted means the part's filament, replaced by a
//! layer range's. The sets are dilated by one layer, which covers the ±1-layer disagreement of
//! the slicer's paint segmentation and range edges (04 §8.4: real k_max/H_t 2 / 41.36-41.41,
//! the dilated rule 2 / 41.92 on all three goats).

use std::collections::BTreeSet;

use super::changes::{LayerOrdering, changes_per_layer};
use super::sources::FilamentSources;
use crate::embedded_mesh::{ObjectMesh, Part};
use crate::first_layer::{ProjectSettings, SlicerBuild, Unsupported, first_layer_height};
use crate::paint::decode_leaves;

/// Colour states are filament numbers; no printer has this many.
const MAX_STATE: u32 = 63;

/// The layer grid of a fixed layer height: layer 0 has the first-layer height, every later
/// layer `height`; the slice plane is the middle of the layer (01 §1.2 rule 4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerGrid {
    pub first_height: f64,
    pub height: f64,
    pub count: usize,
}

impl LayerGrid {
    /// Every layer that starts below the object's top.
    pub fn for_object(first_height: f64, height: f64, object_height: f64) -> LayerGrid {
        let above_first = ((object_height - first_height) / height).ceil().max(0.0) as usize;
        let mut grid = LayerGrid {
            first_height,
            height,
            count: 1 + above_first,
        };
        while grid.count > 1 && grid.bottom(grid.count - 1) >= object_height {
            grid.count -= 1;
        }
        grid
    }

    pub fn top(&self, layer: usize) -> f64 {
        self.first_height + layer as f64 * self.height
    }

    pub fn bottom(&self, layer: usize) -> f64 {
        if layer == 0 { 0.0 } else { self.top(layer - 1) }
    }

    pub fn plane(&self, layer: usize) -> f64 {
        (self.bottom(layer) + self.top(layer)) / 2.0
    }

    pub fn layer_height(&self, layer: usize) -> f64 {
        self.top(layer) - self.bottom(layer)
    }
}

/// One printed layer of the object as the tower sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolLayer {
    pub z_bottom: f64,
    pub z_top: f64,
    /// Filaments of this layer after the ±1-layer dilation, ascending.
    pub filaments: Vec<u32>,
    /// Tool changes the slicer may plan on this layer (upper bound).
    pub changes: usize,
}

/// Tool changes over the whole object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolSchedule {
    pub layers: Vec<ToolLayer>,
    /// Every filament the object prints (undilated), ascending: the plate's filament count the
    /// GUI estimates use.
    pub used: Vec<u32>,
}

impl ToolSchedule {
    /// The schedule of per-layer filament sets `sets` on `grid` (the slicer's sets, before
    /// dilation), with the change bound of the build's `ordering` (`changes`).
    pub fn from_sets(
        grid: &LayerGrid,
        sets: &[BTreeSet<u32>],
        ordering: LayerOrdering,
    ) -> ToolSchedule {
        let dilated: Vec<BTreeSet<u32>> = (0..sets.len())
            .map(|layer| {
                let low = layer.saturating_sub(1);
                let high = (layer + 2).min(sets.len());
                sets[low..high].iter().flatten().copied().collect()
            })
            .collect();
        let changes = changes_per_layer(&dilated, ordering);
        let layers = dilated
            .iter()
            .zip(changes)
            .enumerate()
            .map(|(layer, (set, changes))| ToolLayer {
                z_bottom: grid.bottom(layer),
                z_top: grid.top(layer),
                filaments: set.iter().copied().collect(),
                changes,
            })
            .collect();
        let used: BTreeSet<u32> = sets.iter().flatten().copied().collect();
        ToolSchedule {
            layers,
            used: used.into_iter().collect(),
        }
    }

    /// The layers the tower prints on: every layer up to the last one with a tool change
    /// (B `GCode/ToolOrdering.cpp:948-967`); empty when nothing changes.
    pub fn tower_layers(&self) -> &[ToolLayer] {
        let last = self.layers.iter().rposition(|layer| layer.changes > 0);
        last.map_or(&[], |last| &self.layers[..=last])
    }

    /// z of the tower top H_t, `None` without a tower.
    pub fn tower_top(&self) -> Option<f64> {
        self.tower_layers().last().map(|layer| layer.z_top)
    }

    pub fn max_changes(&self) -> usize {
        self.layers
            .iter()
            .map(|layer| layer.changes)
            .max()
            .unwrap_or(0)
    }
}

/// The schedule of `mesh` with its filament sources (04 §4.1).
pub fn tool_schedule(
    mesh: &ObjectMesh,
    sources: &FilamentSources,
    settings: &ProjectSettings,
    build: SlicerBuild,
) -> Result<ToolSchedule, Unsupported> {
    let grid = fixed_layer_grid(mesh, sources, settings)?;
    let planes: Vec<f64> = (0..grid.count).map(|layer| grid.plane(layer)).collect();
    let mut sets = vec![BTreeSet::new(); grid.count];
    for part in &mesh.parts {
        let Some(filament) = sources.part_filament(part) else {
            continue;
        };
        if part.kind == crate::embedded_mesh::PartKind::Modifier {
            add_modifier(part, filament, &planes, &mut sets);
            continue;
        }
        let states = layer_states(part, &planes, build)?;
        for (layer, mask) in states.iter().enumerate() {
            for state in (0..=MAX_STATE).filter(|state| mask >> state & 1 == 1) {
                let own = sources.range_filament_at(planes[layer]).unwrap_or(filament);
                sets[layer].insert(if state == 0 { own } else { state });
            }
        }
    }
    Ok(ToolSchedule::from_sets(
        &grid,
        &sets,
        LayerOrdering::of(build),
    ))
}

/// The grid of the project's fixed layer height. A painted height profile or a range with
/// its own height makes the grid variable: B switches to Tree Hybrid and Orca refuses
/// (design §5.7), and the tower z-schedule changes with it.
fn fixed_layer_grid(
    mesh: &ObjectMesh,
    sources: &FilamentSources,
    settings: &ProjectSettings,
) -> Result<LayerGrid, Unsupported> {
    let first = first_layer_height(settings)?;
    let height = settings
        .number("layer_height")
        .filter(|height| *height > 0.0)
        .ok_or_else(|| {
            Unsupported::new(
                "layer_height",
                "missing",
                "a positive layer height is required",
            )
        })?;
    if sources.variable_layer_height {
        return Err(Unsupported::new(
            "layer_heights_profile",
            "present",
            "variable layer height changes the layer grid",
        ));
    }
    if let Some(range) = sources.layer_ranges.iter().find(|range| {
        range
            .layer_height
            .is_some_and(|h| (h - height).abs() > 1e-6)
    }) {
        return Err(Unsupported::new(
            "layer_config_ranges.layer_height",
            range.layer_height.unwrap_or_default(),
            "a range with its own layer height makes the grid variable",
        ));
    }
    Ok(LayerGrid::for_object(first, height, mesh.height()))
}

/// Per layer, the bit set of colour states of `part`'s triangles that span the layer's plane
/// (bit 0 = unpainted). A triangle reports every state of its paint tree over its whole
/// z-extent: wider than the painted leaf, so it errs towards more filaments (04 §4.1).
fn layer_states(part: &Part, planes: &[f64], build: SlicerBuild) -> Result<Vec<u64>, Unsupported> {
    let mut masks = vec![0_u64; planes.len()];
    for index in 0..part.triangles.len() {
        let corners = part.corners(index);
        let mask = triangle_states(part, index, corners, build)?;
        let low = corners.iter().map(|c| c[2]).fold(f64::INFINITY, f64::min);
        let high = corners
            .iter()
            .map(|c| c[2])
            .fold(f64::NEG_INFINITY, f64::max);
        let first = planes.partition_point(|plane| *plane < low);
        let last = planes.partition_point(|plane| *plane <= high);
        for slot in &mut masks[first..last.max(first)] {
            *slot |= mask;
        }
    }
    Ok(masks)
}

fn triangle_states(
    part: &Part,
    index: usize,
    corners: [[f64; 3]; 3],
    build: SlicerBuild,
) -> Result<u64, Unsupported> {
    let Some(code) = part.paint_color.get(&(index as u32)) else {
        return Ok(1);
    };
    let leaves = decode_leaves(code, corners, build.colour_coding())
        .map_err(|error| Unsupported::new("paint_color", code, &error.to_string()))?;
    let mut mask = 0_u64;
    for leaf in leaves {
        let state = leaf.state.raw();
        if state > MAX_STATE {
            return Err(Unsupported::new(
                "paint_color",
                state,
                "more colour states than filaments",
            ));
        }
        mask |= 1 << state;
    }
    Ok(mask)
}

/// A modifier with its own extruder may print that filament on every layer it spans.
fn add_modifier(part: &Part, filament: u32, planes: &[f64], sets: &mut [BTreeSet<u32>]) {
    let Some((min, max)) = part.bounds() else {
        return;
    };
    let first = planes.partition_point(|plane| *plane < min[2]);
    let last = planes.partition_point(|plane| *plane <= max[2]);
    for set in &mut sets[first..last.max(first)] {
        set.insert(filament);
    }
}

//! Stage B8: the member's printed layers as one `Realization` (design §4.1 band rule): the
//! base on layer 0, and per absolute 2 mm band the union of the support material of every
//! layer whose interval (z_prev, z] meets the band, split into what lies above the base and
//! the interface part. Holes are filled, as in a realization read from G-code: the gaps between
//! support lines are no room for a neighbour.

use super::base::first_layer_base;
use super::graph::BranchGraph;
use super::smooth::smooth;
use super::stage_a::StageA;
use super::{draw::draw, tips::RoofPiece};
use crate::geometry::Point;
use crate::object_print::{Loops, MemberId, Realization, layer_bands};
use crate::polygon_ops::{Grid, Rings, islands, union_all};

/// Smooths, draws and bands the graph of one member.
pub fn realization(stage: &StageA, graph: &BranchGraph, member: MemberId) -> Realization {
    let smoothed = smooth(stage, &graph.layers);
    let tubes = draw(stage, &graph.layers, &smoothed.centres);
    layers_to_realization(stage, &tubes, &graph.roofs, member)
}

/// The realization from drawn tube slices and roof pieces (`tubes[0]` = the roots).
pub fn layers_to_realization(
    stage: &StageA,
    tubes: &[Rings],
    roofs: &[RoofPiece],
    member: MemberId,
) -> Realization {
    layers_with_base(stage, tubes, roofs, member, &Rings::new())
}

/// One member's first-layer base: its roots (`draw::finish_layer` of its layer-0 slices)
/// grown by `first_layer_base`. The growth spreads through the gap around the model, so a
/// 0.03 mm difference of the roots can move the base by 1.5 mm: a union keeps the members'
/// own bases (`union::UnionSlices`) instead of growing the union's roots.
pub fn member_base(stage: &StageA, layer0: Vec<crate::polygon_ops::Ring>) -> Rings {
    first_layer_base(stage, &super::draw::finish_layer(stage, 0, layer0))
}

/// `layers_to_realization` with `extra_base` (object frame, stage units) added to the base.
pub fn layers_with_base(
    stage: &StageA,
    tubes: &[Rings],
    roofs: &[RoofPiece],
    member: MemberId,
    extra_base: &Rings,
) -> Realization {
    let count = stage.grid.len();
    let mut roof_layers = vec![Rings::new(); count];
    for piece in roofs {
        if let Some(layer) = roof_layers.get_mut(piece.layer) {
            *layer = union_all([&*layer, &piece.rings]);
        }
    }
    let empty = Rings::new();
    let root_tubes = tubes.first().unwrap_or(&empty);
    let base = union_all([
        &first_layer_base(stage, root_tubes),
        &roof_layers[0],
        extra_base,
    ]);
    let mut support: Vec<Vec<Rings>> = Vec::new();
    let mut above: Vec<Vec<Rings>> = Vec::new();
    let mut interface: Vec<Vec<Rings>> = Vec::new();
    for (index, layer) in stage.grid.layers.iter().enumerate() {
        let material = if index == 0 {
            base.clone()
        } else {
            union_all([tubes.get(index).unwrap_or(&empty), &roof_layers[index]])
        };
        for band in layer_bands(layer.bottom, layer.top) {
            for list in [&mut support, &mut above, &mut interface] {
                if list.len() <= band {
                    list.resize(band + 1, Vec::new());
                }
            }
            support[band].push(material.clone());
            if index > 0 {
                above[band].push(material.clone());
            }
            interface[band].push(roof_layers[index].clone());
        }
    }
    let grid = stage.derived.grid;
    let shift = stage.frame_shift;
    let loops = |rings: &Rings| to_loops(&exterior(rings), grid, shift);
    let union_bands = |bands: Vec<Vec<Rings>>| -> Vec<Loops> {
        bands
            .iter()
            .map(|layers| loops(&union_all(layers.iter())))
            .collect()
    };
    Realization {
        member,
        landing: Loops::new(),
        base: loops(&base),
        above_base: union_bands(above),
        support: union_bands(support),
        interface: union_bands(interface),
    }
}

/// The outer ring of every island: holes are filled.
fn exterior(rings: &Rings) -> Rings {
    islands(rings)
        .into_iter()
        .filter_map(|island| island.into_iter().next())
        .collect()
}

/// Closed mm loops in the plate-aligned frame (the object frame plus the frame shift).
fn to_loops(rings: &Rings, grid: Grid, shift: [f64; 2]) -> Loops {
    grid.loops(rings)
        .into_iter()
        .map(|ring| {
            ring.into_iter()
                .map(|point| Point {
                    x: point.x + shift[0],
                    y: point.y + shift[1],
                })
                .collect()
        })
        .collect()
}

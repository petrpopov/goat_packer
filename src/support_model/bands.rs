//! Stage B8: the member's printed layers as one `Realization` (design §4.1 band rule): the
//! base on layer 0, and per absolute 2 mm band the union of the support material of every
//! layer whose interval (z_prev, z] meets the band, split into what lies above the base and
//! the interface part. Holes are filled, as in a realization read from G-code: the gaps between
//! support lines are no room for a neighbour.

use super::base::first_layer_base;
use super::graph::BranchGraph;
use super::smooth::smooth;
use super::stage_a::StageA;
use super::node::Node;
use super::{draw::{draw, draw_union}, tips::RoofPiece};
use crate::geometry::Point;
use crate::object_print::{Loops, MemberId, Realization, layer_bands};
use crate::polygon_ops::{Grid, IntPoint, Rings, islands, union_all};

/// Smooths, draws and bands the graph of one member.
pub fn realization(stage: &StageA, graph: &BranchGraph, member: MemberId) -> Realization {
    let smoothed = smooth(stage, &graph.layers);
    let tubes = draw(stage, &graph.layers, &smoothed.centres);
    layers_to_realization(stage, &tubes, &graph.roofs, member)
}

/// The realization of several graphs at once (the ensemble's union): each graph smoothed on
/// its own (in parallel), all drawn as one layer set (`draw::draw_union`), the roofs of every
/// arc family once. It covers the union of the graphs' own realizations.
pub fn union_realization(stage: &StageA, graphs: &[&BranchGraph], member: MemberId) -> Realization {
    use rayon::prelude::*;
    let smoothed: Vec<Vec<Vec<IntPoint>>> = graphs
        .par_iter()
        .map(|graph| smooth(stage, &graph.layers).centres)
        .collect();
    let drawn: Vec<(&[Vec<Node>], &[Vec<IntPoint>])> = graphs
        .iter()
        .zip(&smoothed)
        .map(|(graph, centres)| (graph.layers.as_slice(), centres.as_slice()))
        .collect();
    let drawing = draw_union(stage, &drawn);
    let mut roofs: Vec<RoofPiece> = Vec::new();
    for piece in graphs.iter().flat_map(|graph| &graph.roofs) {
        if !roofs.contains(piece) {
            roofs.push(piece.clone());
        }
    }
    // Every root set grows its own flange, as each graph alone would.
    let flanges: Vec<Rings> = drawing
        .roots
        .par_iter()
        .map(|roots| first_layer_base(stage, roots))
        .collect();
    realization_of(stage, &drawing.layers, union_all(flanges.iter()), &roofs, member)
}

/// The realization from drawn tube slices and roof pieces (`tubes[0]` = the roots).
pub fn layers_to_realization(
    stage: &StageA,
    tubes: &[Rings],
    roofs: &[RoofPiece],
    member: MemberId,
) -> Realization {
    let empty = Rings::new();
    let flange = first_layer_base(stage, tubes.first().unwrap_or(&empty));
    realization_of(stage, tubes, flange, roofs, member)
}

/// The realization from drawn tube slices, the grown first-layer flange and roof pieces.
fn realization_of(
    stage: &StageA,
    tubes: &[Rings],
    flange: Rings,
    roofs: &[RoofPiece],
    member: MemberId,
) -> Realization {
    let count = stage.grid.len();
    let mut roof_layers = vec![Rings::new(); count];
    for piece in roofs {
        if let Some(layer) = roof_layers.get_mut(piece.layer) {
            *layer = union_all([&*layer, &piece.rings]);
        }
    }
    let empty = Rings::new();
    let base = union_all([&flange, &roof_layers[0]]);
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

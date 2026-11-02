//! The per-layer overhangs H_k and what the tree generator receives (spec 01 §3.7-§3.8).

use super::clusters::{Clustered, emitted};
use super::context::{
    Context, ENFORCER_GROW_MM, TAIL_HANDOFF_GROW_MM, VERTICAL_DISC_ERROR_UNITS, VERTICAL_DISC_MM,
};
use super::pass::LayerPass;
use super::tails::{LayerTails, supports};
use super::{OverhangFlags, TypedOverhang};
use crate::polygon_ops::{
    IntPoint, Island, Ring, Rings, difference, difference_islands, intersection,
    intersection_islands, islands, overlaps,
};
use crate::support_model::flavour::CriticalOnly;
use crate::support_model::settings::Derived;

/// H_k for every layer.
pub(super) fn typed_layers(
    context: &Context,
    passes: &[LayerPass],
    tails: &[LayerTails],
    clustered: &Clustered,
) -> Vec<Vec<TypedOverhang>> {
    let detected = emitted(context, clustered);
    (0..context.layers())
        .map(|layer| match context.flavour.critical_only() {
            CriticalOnly::Bambu => bambu_layer(context, passes, tails, &detected[layer], layer),
            CriticalOnly::Orca => orca_layer(context, passes, tails, &detected[layer], layer),
        })
        .collect()
}

/// B (§3.7): critical-only keeps the cantilevers, except on enforcer layers, where cantilever
/// and sharp-tail polygons stay whole, the rest is cut to the grown enforcers and big flats
/// are united; then the tail supports.
fn bambu_layer(
    context: &Context,
    passes: &[LayerPass],
    tails: &[LayerTails],
    detected: &[TypedOverhang],
    layer: usize,
) -> Vec<TypedOverhang> {
    let mut found = if !(context.auto && context.critical_only) {
        detected.to_vec()
    } else if layer == 0 || layer >= context.enforcer_count || context.enforcer(layer).is_empty() {
        typed(passes[layer].cantilevers.clone(), cantilever())
    } else {
        cut_to_enforcers(context, detected, layer)
    };
    found.extend(tail_supports(context, tails, layer));
    found
}

fn cut_to_enforcers(
    context: &Context,
    detected: &[TypedOverhang],
    layer: usize,
) -> Vec<TypedOverhang> {
    let reach = context.grow(context.enforcer(layer), ENFORCER_GROW_MM);
    let mut found = Vec::new();
    let mut big_flat = Rings::new();
    for overhang in detected {
        let pieces = if overhang.flags.cantilever || overhang.flags.sharp_tail {
            vec![overhang.island.clone()]
        } else if overlaps(&reach, &overhang.island) {
            intersection_islands(&reach, &overhang.island)
        } else {
            Vec::new()
        };
        if overhang.flags.big_flat {
            big_flat.extend(pieces.into_iter().flatten());
        } else {
            found.extend(typed(pieces, overhang.flags));
        }
    }
    let flags = OverhangFlags {
        big_flat: true,
        ..OverhangFlags::default()
    };
    found.extend(typed(islands(&big_flat), flags));
    found
}

/// O/R/S (§3.7): tail supports; grown blockers removed; critical-only replaces the detected
/// overhangs by the cantilevers; then the enforced overhang and the tail supports.
fn orca_layer(
    context: &Context,
    passes: &[LayerPass],
    tails: &[LayerTails],
    detected: &[TypedOverhang],
    layer: usize,
) -> Vec<TypedOverhang> {
    let mut tail_pieces = tail_supports(context, tails, layer);
    let mut overhangs = detected.to_vec();
    let mut cantilevers = typed(passes[layer].cantilevers.clone(), cantilever());
    let blocker = context.grown_blocker(layer);
    if !blocker.is_empty() {
        overhangs = without(overhangs, &blocker);
        cantilevers = without(cantilevers, &blocker);
        tail_pieces = without(tail_pieces, &blocker);
    }
    if context.auto && context.critical_only {
        overhangs = cantilevers;
    }
    if layer >= 1 {
        overhangs.extend(enforced(context, layer));
    }
    overhangs.extend(tail_pieces);
    overhangs
}

/// ((X_k \ X_{k−1}) ∩ E_k) ⊕ e_E \ X_{k−1}, typed enforced.
fn enforced(context: &Context, layer: usize) -> Vec<TypedOverhang> {
    let enforcer = context.enforcer(layer);
    if enforcer.is_empty() {
        return Vec::new();
    }
    let lower = &context.extrudable[layer - 1];
    let fresh = intersection(&difference(&context.extrudable[layer], lower), enforcer);
    if fresh.is_empty() {
        return Vec::new();
    }
    let grown = context.grow(&fresh, ENFORCER_GROW_MM);
    let flags = OverhangFlags {
        enforced: true,
        ..OverhangFlags::default()
    };
    typed(difference_islands(&grown, lower), flags)
}

fn tail_supports(context: &Context, tails: &[LayerTails], layer: usize) -> Vec<TypedOverhang> {
    supports(context, tails, layer)
        .into_iter()
        .map(|(island, height)| TypedOverhang {
            island,
            flags: OverhangFlags {
                sharp_tail: true,
                lowest_tail: height < 1e-4,
                ..OverhangFlags::default()
            },
        })
        .collect()
}

fn without(overhangs: Vec<TypedOverhang>, blocker: &Rings) -> Vec<TypedOverhang> {
    overhangs
        .into_iter()
        .flat_map(|overhang| {
            difference_islands(&overhang.island, blocker)
                .into_iter()
                .map(move |island| TypedOverhang {
                    island,
                    flags: overhang.flags,
                })
        })
        .collect()
}

fn typed(islands: Vec<Island>, flags: OverhangFlags) -> Vec<TypedOverhang> {
    islands
        .into_iter()
        .map(|island| TypedOverhang { island, flags })
        .collect()
}

fn cantilever() -> OverhangFlags {
    OverhangFlags {
        cantilever: true,
        ..OverhangFlags::default()
    }
}

/// The hand-off polygons (§3.8): every H_k ring, sharp-tail ones grown by 0.2 mm, then the
/// vertical-enforcer discs on the first layer whose plane is at or above the point (never
/// layer 0).
pub(super) fn handoff(context: &Context, layers: &[Vec<TypedOverhang>]) -> Vec<Rings> {
    let mut handoff: Vec<Rings> = layers
        .iter()
        .map(|overhangs| {
            overhangs
                .iter()
                .flat_map(|overhang| {
                    if overhang.flags.sharp_tail {
                        context.grow(&overhang.island, TAIL_HANDOFF_GROW_MM)
                    } else {
                        overhang.island.clone()
                    }
                })
                .collect()
        })
        .collect();
    for point in context.vertical_points {
        let z = point[2] as f32;
        let layer = context.planes.partition_point(|&plane| plane < z);
        if layer > 0 && layer < context.layers() {
            handoff[layer].push(disc(context, point));
        }
    }
    handoff
}

/// A 0.5 mm disc around a vertical enforcer point, vertices truncated toward zero.
fn disc(context: &Context, point: &[f64; 3]) -> Ring {
    let radius = context.units(VERTICAL_DISC_MM);
    let count = Derived::circle_vertices(radius, VERTICAL_DISC_ERROR_UNITS);
    let step = 2.0 * std::f64::consts::PI / count as f64;
    let (cx, cy) = (
        context.units(point[0]) as i64,
        context.units(point[1]) as i64,
    );
    (0..count)
        .map(|index| {
            let angle = step * index as f64;
            IntPoint::new(
                (cx + (libm::cos(angle) * radius) as i64) as i32,
                (cy + (libm::sin(angle) * radius) as i64) as i32,
            )
        })
        .collect()
}

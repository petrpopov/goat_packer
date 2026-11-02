//! Step 1 of detection, one layer at a time (spec 01 §3.1-§3.3): the overhang Ov_k, the
//! sharp-tail seeds and the cantilevers. Upstream runs these layers in parallel; nothing here
//! reads another layer's result, so ascending order is exact.

use super::GUARD_ISLANDS;
use super::context::{
    BIG_OVERHANG_MM, CANTILEVER_MM, Context, ENFORCER_GROW_MM, SMALL_OVERHANG_RADIUS_WIDTHS,
    WELL_SUPPORTED_MM,
};
use crate::polygon_ops::{
    IntBox, Island, Join, Rings, area, bbox, contains, difference, difference_islands,
    distance_to_boundary, flatten, intersection, islands, offset, overlaps, union_islands,
};
use crate::support_model::SupportError;

/// What step 1 finds on one layer.
#[derive(Clone, Debug, Default)]
pub(super) struct LayerPass {
    /// Ov_k.
    pub overhangs: Vec<Island>,
    pub cantilevers: Vec<Island>,
    /// Sharp-tail seeds with their accumulated height (f32, as upstream sums it).
    pub seeds: Vec<(Island, f32)>,
}

/// Step 1 on every layer. The overhang guard's island trigger (§3.9) would switch sharp-tail
/// detection off from a racy layer onward; that realization set is not modelled.
pub(super) fn run(context: &Context) -> Result<Vec<LayerPass>, SupportError> {
    let mut passes = Vec::with_capacity(context.layers());
    for layer in 0..context.layers() {
        let pass = if !detects(context, layer) {
            LayerPass::default()
        } else if layer == 0 {
            first_layer(context)
        } else {
            upper_layer(context, layer)
        };
        if pass.overhangs.len() > GUARD_ISLANDS {
            return Err(SupportError::unsupported(
                "overhang_islands",
                &format!("{} on layer {layer}", pass.overhangs.len()),
                "the overhang guard (S3) would stop sharp-tail detection from a racy layer (spec 01 §3.9)",
            ));
        }
        passes.push(pass);
    }
    Ok(passes)
}

/// Which layers the pass visits: every layer for automatic supports; with manual supports
/// O/R/S only the `enforce_support_layers` bottom layers, B also every layer with enforcers.
fn detects(context: &Context, layer: usize) -> bool {
    if context.auto {
        return true;
    }
    if context.flavour.paint_in_detection() {
        (context.enforce_layers > 0 && layer >= context.enforce_layers)
            || (layer < context.enforcer_count && !context.enforcer(layer).is_empty())
    } else {
        layer <= context.enforce_layers
    }
}

/// Layer 0 has no overhang; its small islands seed sharp tails with the layer height. The box
/// must be ≤ 6 mm in both axes in B, in at least one axis in O/R/S.
fn first_layer(context: &Context) -> LayerPass {
    let mut area = context.extrudable[0].clone();
    if context.flavour.paint_in_detection() && context.blocker_count > 0 {
        let blocker = context.grown_blocker(0);
        if !blocker.is_empty() {
            area = difference(&area, &blocker);
        }
    }
    let limit = context.units(WELL_SUPPORTED_MM);
    let both = context.flavour.first_layer_seed_needs_both_axes();
    let seeds = islands(&area)
        .into_iter()
        .filter(|island| {
            let (width, height) = size(island);
            let (wide, tall) = (width > limit, height > limit);
            if both {
                !(wide || tall)
            } else {
                !(wide && tall)
            }
        })
        .map(|island| (island, context.heights[0]))
        .collect();
    LayerPass {
        seeds,
        ..LayerPass::default()
    }
}

fn upper_layer(context: &Context, layer: usize) -> LayerPass {
    let lower = &context.extrudable[layer - 1];
    let step_mm = context.step_mm(layer);
    let lower_offset = offset(lower, context.units(step_mm), Join::Square);
    let (overhangs, blocker, enforced) = if context.flavour.paint_in_detection() {
        bambu_overhangs(context, layer, &lower_offset)
    } else {
        let plain = difference_islands(&context.extrudable[layer], &lower_offset);
        (plain, Rings::new(), Rings::new())
    };
    let seeds = if context.auto {
        seeds(context, layer, &blocker, &enforced)
    } else {
        Vec::new()
    };
    let cantilevers = cantilevers(context, layer, &overhangs, &lower_offset, step_mm);
    LayerPass {
        overhangs,
        cantilevers,
        seeds,
    }
}

/// B (§3.2 steps 1-2): Ov = (P \ (K ⊕ e_K)) ∪ Eh with Eh = ((X_k \ X_{k−1}) ∩ E_k) ⊕ e_E \
/// X_{k−1}. Returns Ov, the grown blocker and Eh. Manual supports keep only Eh.
fn bambu_overhangs(
    context: &Context,
    layer: usize,
    lower_offset: &Rings,
) -> (Vec<Island>, Rings, Rings) {
    let current = &context.extrudable[layer];
    let lower = &context.extrudable[layer - 1];
    let raw_enforced = if layer < context.enforcer_count {
        intersection(&difference(current, lower), context.enforcer(layer))
    } else {
        Rings::new()
    };
    let enforced = if raw_enforced.is_empty() {
        Rings::new()
    } else {
        difference(&context.grow(&raw_enforced, ENFORCER_GROW_MM), lower)
    };
    if !context.auto {
        return (islands(&enforced), Rings::new(), enforced);
    }
    let mut overhangs = difference_islands(current, lower_offset);
    let blocker = if layer < context.blocker_count {
        context.grown_blocker(layer)
    } else {
        Rings::new()
    };
    if !blocker.is_empty() {
        overhangs = difference_islands(&flatten(overhangs), &blocker);
    }
    if !enforced.is_empty() {
        overhangs = union_islands(&flatten(overhangs), &enforced);
    }
    (overhangs, blocker, enforced)
}

/// Sharp-tail seeds (§3.2 step 4, §3.3 step 3): islands floating 0.1·w above the layer below
/// that survive an erosion by 0.1·w. B also needs area and box below 100 mm² (or a ring too
/// thin for the small-overhang erosion), and no contact two layers down.
fn seeds(context: &Context, layer: usize, blocker: &Rings, enforced: &Rings) -> Vec<(Island, f32)> {
    let lower = &context.extrudable[layer - 1];
    let strict = context.flavour.strict_tail_seeds();
    let mut candidates = context.extrudable[layer].clone();
    if strict {
        if !blocker.is_empty() {
            candidates = difference(&candidates, blocker);
        }
        if !enforced.is_empty() {
            candidates = flatten(union_islands(&candidates, enforced));
        }
    }
    let margin = 0.1 * context.width_mm;
    let big = context.units(BIG_OVERHANG_MM);
    let mut found = Vec::new();
    for island in islands(&candidates) {
        let grown = context.grow(&island, margin);
        if overlaps(&grown, lower) {
            continue;
        }
        if strict {
            let (width, height) = size(&island);
            let thin = || {
                context
                    .grow(&island, -SMALL_OVERHANG_RADIUS_WIDTHS * context.width_mm)
                    .is_empty()
            };
            if area(&island) >= big * big || (width * height >= big * big && !thin()) {
                continue;
            }
            if layer >= 2 && overlaps(&grown, &context.extrudable[layer - 2]) {
                continue;
            }
        }
        if !context.grow(&island, -margin).is_empty() {
            found.push((island, 0.0f32));
        }
    }
    found
}

/// Cantilevers (§3.2 step 5, §3.3 step 4): some vertex of the island's outer ring lies more
/// than 3 mm from the part resting on the layer below (Q = J ∩ U); B also when one of its
/// rings has no vertex inside X_{k−1} ⊕□ w (a floating ring).
fn cantilevers(
    context: &Context,
    layer: usize,
    overhangs: &[Island],
    lower_offset: &Rings,
    step_mm: f64,
) -> Vec<Island> {
    let lower = &context.extrudable[layer - 1];
    let support_reach = (context.width_mm - step_mm).max(0.0) + 0.1;
    let resting = context.grow(lower_offset, support_reach);
    let near_lower = if context.flavour.floating_ring_is_cantilever() {
        offset(lower, context.width, Join::Square)
    } else {
        Rings::new()
    };
    let limit = context.units(CANTILEVER_MM);
    overhangs
        .iter()
        .filter(|island| {
            if context.flavour.floating_ring_is_cantilever()
                && has_floating_ring(island, &near_lower)
            {
                return true;
            }
            let base = intersection(island, &resting);
            if base.is_empty() {
                return false;
            }
            island[0]
                .iter()
                .map(|&vertex| distance_to_boundary(&base, vertex))
                .fold(0.0f64, f64::max)
                > limit
        })
        .cloned()
        .collect()
}

fn has_floating_ring(island: &Island, near_lower: &Rings) -> bool {
    let reach = bbox(near_lower);
    island.iter().any(|ring| {
        !ring.iter().any(|&vertex| {
            reach.is_some_and(|reach| reach.contains(vertex)) && contains(near_lower, vertex)
        })
    })
}

/// Box size of an island in units (0 × 0 when empty, as an undefined upstream box).
pub(super) fn size(rings: &[crate::polygon_ops::Ring]) -> (f64, f64) {
    bbox(rings).map_or((0.0, 0.0), |found: IntBox| {
        let (width, height) = found.size();
        (width as f64, height as f64)
    })
}

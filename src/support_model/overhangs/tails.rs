//! Sharp tails (spec 01 §3.4, §3.7): thin features that start in mid-air grow upward from
//! their seeds while they stay small and slow-growing, and every tail layer whose accumulated
//! height (f32, summed in upstream's order) truncates to a multiple of 0.5 mm in tenths emits
//! a tail support: the tail minus the layer below grown by `RESOLUTION`.

use super::context::{
    Context, RESOLUTION_MM, TAIL_GROWTH_MM, TAIL_MAX_HEIGHT_MM, WELL_SUPPORTED_MM,
};
use super::pass::{LayerPass, size};
use crate::polygon_ops::{
    Island, Rings, difference, difference_islands, flatten, intersection, islands, overlaps,
    union_islands,
};

/// Tails of one layer: seeds first, then grown tails, each with its accumulated height.
pub(super) type LayerTails = Vec<(Island, f32)>;

/// Seeds of step 1 plus the tails grown bottom-up (§3.4); growth only for automatic supports.
pub(super) fn grow(context: &Context, passes: &[LayerPass]) -> Vec<LayerTails> {
    let mut tails: Vec<LayerTails> = passes.iter().map(|pass| pass.seeds.clone()).collect();
    if !context.auto {
        return tails;
    }
    for layer in 1..context.layers() {
        let (done, rest) = tails.split_at_mut(layer);
        let below = &done[layer - 1];
        if below.is_empty() {
            continue;
        }
        let grown = grown_on(context, layer, below);
        rest[0].extend(grown);
    }
    tails
}

/// The islands of layer `layer` that continue a tail of the layer below.
fn grown_on(context: &Context, layer: usize, below: &LayerTails) -> LayerTails {
    let below_rings: Rings = below
        .iter()
        .flat_map(|(tail, _)| tail.iter().cloned())
        .collect();
    let (below_width, below_height) = size(&below_rings);
    let well_supported = context.units(WELL_SUPPORTED_MM);
    let growth = context.units(TAIL_GROWTH_MM);
    let mut found = Vec::new();
    for island in islands(&candidates(context, layer)) {
        let supported = intersection(&island, &below_rings);
        if supported.is_empty() {
            continue;
        }
        let (width, height) = size(&supported);
        if width > well_supported && height > well_supported {
            continue;
        }
        let mut accumulated = context.heights[layer];
        if let Some((_, base)) = below.iter().find(|(tail, _)| overlaps(tail, &island)) {
            accumulated += *base;
        }
        if accumulated > TAIL_MAX_HEIGHT_MM {
            continue;
        }
        let new_part = difference(&island, &below_rings);
        let (new_width, new_height) = size(&new_part);
        let outgrown = new_width - below_width > growth && new_height - below_height > growth;
        if outgrown || !context.grow(&new_part, -5.0 * context.width_mm).is_empty() {
            continue;
        }
        found.push((island, accumulated));
    }
    found
}

/// What tails may grow on: X_k; in B minus the grown blockers, plus the (ungrown) enforced
/// new material (§3.4).
fn candidates(context: &Context, layer: usize) -> Rings {
    let current = &context.extrudable[layer];
    if !context.flavour.paint_in_detection() {
        return current.clone();
    }
    let mut area = current.clone();
    if layer < context.blocker_count {
        let blocker = context.grown_blocker(layer);
        if !blocker.is_empty() {
            area = difference(&area, &blocker);
        }
    }
    if layer < context.enforcer_count && !context.enforcer(layer).is_empty() {
        let lower = &context.extrudable[layer - 1];
        let enforced = intersection(&difference(current, lower), context.enforcer(layer));
        area = flatten(union_islands(&area, &enforced));
    }
    area
}

/// The tail supports emitted on `layer` (§3.7): on layer 0 every seed in O/R/S (none in B);
/// above, each tail minus X_{k−1} ⊕ ρ when trunc(10·h) is a multiple of 5 (f32).
pub(super) fn supports(
    context: &Context,
    tails: &[LayerTails],
    layer: usize,
) -> Vec<(Island, f32)> {
    if layer == 0 {
        if context.flavour.paint_in_detection() {
            return Vec::new();
        }
        return tails[0].clone();
    }
    let lower = context.grow(&context.extrudable[layer - 1], RESOLUTION_MM);
    let mut found = Vec::new();
    for (tail, height) in &tails[layer] {
        if (height * 10.0) as i32 % 5 != 0 {
            continue;
        }
        for piece in difference_islands(tail, &lower) {
            found.push((piece, *height));
        }
    }
    found
}

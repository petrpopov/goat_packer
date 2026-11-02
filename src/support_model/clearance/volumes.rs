//! The volume formulas of spec 02 §2.3-§2.7, one function each. Offsets use the slicers'
//! joins: ⊕m is a mitre with requested limit 1.2 (acting as 2), ⊕r a round join with the
//! resolution as arc tolerance.

use std::sync::Arc;

use super::{Avoidance, Clearance, Volume};
use crate::polygon_ops::{
    DEFAULT_MITER_LIMIT, FillRule, Join, Rings, difference, intersection, offset, resolve, simplify,
};

/// The mitre limit the volumes ask for; Clipper raises it to 2 (spec 02 §1.1).
const VOLUME_MITER_LIMIT: f64 = 1.2;

/// CollHF shrinks back by the key difference less this many grid units.
const HOLE_FREE_SLACK_UNITS: i64 = 5;

/// A last avoidance step shorter than this is folded into the others (mm).
const SHORTEST_LAST_STEP_MM: f64 = 0.05;

/// Avoidance steps are at most this multiple of max(K, xy_min), so a shrink cannot tunnel
/// through a wall thinner than the step.
const STEP_FACTOR: f64 = 1.9;

fn volume_join() -> Join {
    Join::Miter(VOLUME_MITER_LIMIT)
}

fn round_join(clearance: &Clearance) -> Join {
    Join::Round(clearance.resolution as f64)
}

fn simplified(clearance: &Clearance, rings: &Rings) -> Rings {
    simplify(
        rings,
        clearance.resolution as f64,
        clearance.flavour.strictly_simple(),
    )
}

/// O_l resolved and grown by `value` units (`value` 0: just resolved), shared by the
/// collisions of neighbouring layers.
fn grown(clearance: &Clearance, value: i64, layer: usize) -> Arc<Rings> {
    clearance.memo(Volume::Grown, value, layer, || {
        let outline = resolve(&clearance.outlines[layer], FillRule::NonZero);
        if value == 0 {
            outline
        } else {
            offset(&outline, value as f64, volume_join())
        }
    })
}

/// Coll(K, l) (§2.3): layers l − z_bot..l grown by K + xy_min, layers l + i above (i ≤ z_top)
/// grown by K + ρ_i with ρ_i = trunc(xy_min − (i − h)·xy_min / z_top), h = ½ for a single
/// gap layer so support never touches the model on the downward diagonal. Unsimplified
/// except in R (its off-by-one fix also enables the simplification).
pub(super) fn collision(clearance: &Clearance, key: i64, layer: usize) -> Rings {
    let layers = clearance.layers();
    let mut rings = Rings::new();
    for below in layer.saturating_sub(clearance.z_bottom)..=layer.min(layers.saturating_sub(1)) {
        rings.extend(
            grown(clearance, key + clearance.xy_min, below)
                .iter()
                .cloned(),
        );
    }
    let z_top = clearance.z_top;
    let half = if z_top == 1 { 0.5 } else { 0.0 };
    for step in 1..=z_top {
        let above = layer + step;
        if above >= layers {
            break;
        }
        let xy = clearance.xy_min as f64;
        let reduced = (xy - (step as f64 - half) * xy / z_top as f64) as i64;
        let outline = resolve(&clearance.outlines[above], FillRule::NonZero);
        rings.extend(offset(&outline, (key + reduced) as f64, volume_join()));
    }
    let united = resolve(&rings, FillRule::NonZero);
    if clearance.flavour.simplifies_collision() {
        simplified(clearance, &united)
    } else {
        united
    }
}

/// CollHF(K, l) (§2.4): the collision at the "increase until" key K_R shrunk back to K (less
/// 5 units) with round joins, simplified: a closing of the outline at about K_R + xy_min.
pub(super) fn hole_free(clearance: &Clearance, key: i64, layer: usize) -> Rings {
    let source_key = clearance.keys.key(clearance.increase_until, false);
    let source = clearance.collision_at(source_key, layer);
    let shrink = HOLE_FREE_SLACK_UNITS - (source_key - key);
    simplified(
        clearance,
        &offset(&source, shrink as f64, round_join(clearance)),
    )
}

/// Avoid_τ(K, l) to the bed (§2.5), a bottom-up recurrence: the layer below's avoidance
/// shrunk by the move in steps of at most 1.9·max(K, xy_min), united with this layer's
/// collision (hole-free for slow and fast-safe below R_inc + δ), simplified. Computed from the
/// highest cached layer upward.
pub(super) fn avoidance(
    clearance: &Clearance,
    kind: Avoidance,
    key: i64,
    layer: usize,
) -> Arc<Rings> {
    let volume = Volume::Avoid(kind);
    let mut start = layer;
    while start > 0 && clearance.cached(volume, key, start).is_none() {
        start -= 1;
    }
    let mut latest = if start == 0 {
        clearance.collision(key, 0, true)
    } else {
        clearance.cached(volume, key, start).expect("found above")
    };
    if start == layer {
        return latest;
    }
    let hole_free =
        matches!(kind, Avoidance::Slow | Avoidance::FastSafe) && key < clearance.hole_free_limit();
    let steps = move_steps(clearance, kind, key);
    for current_layer in start + 1..=layer {
        let collision = if hole_free {
            clearance.hole_free_at(key, current_layer)
        } else {
            clearance.collision(key, current_layer, true)
        };
        let mut area: Rings = latest.as_ref().clone();
        for &step in &steps {
            let shrunk = offset(&area, -f64::from(step), round_join(clearance));
            area = resolve(
                &[collision.as_ref().clone(), shrunk].concat(),
                FillRule::NonZero,
            );
        }
        latest = clearance.store(volume, key, current_layer, simplified(clearance, &area));
    }
    latest
}

/// The shrink steps of one avoidance layer (f32, as upstream): n = ⌈trunc(m)/trunc(step)⌉
/// equal steps with the remainder last; a last step under 0.05 mm is spread over the others.
fn move_steps(clearance: &Clearance, kind: Avoidance, key: i64) -> Vec<f32> {
    let max_move = if kind == Avoidance::Slow {
        clearance.move_slow
    } else {
        clearance.move_fast
    } as f32;
    let mut step = (STEP_FACTOR * key.max(clearance.xy_min) as f64) as f32;
    let (whole_move, whole_step) = (max_move as i32, step as i32);
    if whole_step <= 0 {
        return vec![max_move];
    }
    let mut count = (whole_move + whole_step - 1) / whole_step;
    let mut last = max_move - (count - 1) as f32 * step;
    let shortest = clearance.grid.scaled(SHORTEST_LAST_STEP_MM) as f32;
    if last < shortest && count > 1 {
        count -= 1;
        step = max_move / count as f32;
        last = step;
    }
    let mut steps = vec![step; count.max(1) as usize];
    if let Some(final_step) = steps.last_mut() {
        *final_step = last;
    }
    steps
}

/// Placeable areas (§2.6, to-model only): on the model surface of the layer z_bot + 1 below
/// grown by xy_min, minus this layer's collision at xy_min; for K > 0 shrunk by
/// K + xy_min + δ. Layer 0 has none (the machine-border term upstream is empty).
pub(super) fn placeable(clearance: &Clearance, key: i64, layer: usize) -> Rings {
    if layer == 0 {
        return Rings::new();
    }
    let base = clearance.memo(Volume::PlaceableBase, 0, layer, || {
        placeable_base(clearance, layer)
    });
    if key == 0 {
        return base.as_ref().clone();
    }
    let shrink = key + clearance.xy_min + clearance.keys.delta();
    offset(&base, -(shrink as f64), volume_join())
}

fn placeable_base(clearance: &Clearance, layer: usize) -> Rings {
    let Some(below) = layer.checked_sub(clearance.z_bottom + 1) else {
        return Rings::new();
    };
    let surface = resolve(&clearance.outlines[below], FillRule::NonZero);
    let reach = offset(
        &surface,
        clearance.xy_min as f64,
        Join::Miter(DEFAULT_MITER_LIMIT),
    );
    difference(&reach, &grown(clearance, clearance.xy_min, layer))
}

/// WR(K, l) (§2.7): the radius-0 collision of this layer (regular: key δ; tight: key 0)
/// intersected with the key-K collision of the layer below, simplified.
pub(super) fn wall_restriction(
    clearance: &Clearance,
    key: i64,
    layer: usize,
    tight: bool,
) -> Rings {
    if layer == 0 {
        return Rings::new();
    }
    let upper = if tight {
        clearance.collision_at(0, layer)
    } else {
        clearance.collision(0, layer, false)
    };
    let lower = clearance.collision(key, layer - 1, true);
    simplified(clearance, &intersection(&upper, &lower))
}

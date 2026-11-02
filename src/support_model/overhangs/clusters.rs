//! Overhang clusters across layers and the small-overhang removal (spec 01 §3.5-§3.6).
//!
//! An overhang island joins the first cluster (creation order) whose members on the layer
//! below it touches once grown by w, inside the cluster's outline box; otherwise it starts a
//! new cluster. The outline is the first member as is plus every later member grown by w.
//! B keeps every member per layer, O/R/S only the first (the rest only widen the outline).
//! B also types clusters: BigFlat (outline above 100 mm²) matters for its critical-only rule
//! at enforcer layers; ThinPlate only steers non-organic styles and is not modelled.

use std::collections::BTreeMap;

use super::context::{BIG_OVERHANG_MM, Context, SMALL_OVERHANG_MM, SMALL_OVERHANG_RADIUS_WIDTHS};
use super::pass::{LayerPass, size};
use super::tails::LayerTails;
use super::{OverhangFlags, TypedOverhang};
use crate::polygon_ops::{
    FillRule, IntBox, Island, Rings, area, bbox, flatten, intersection_islands, islands, overlaps,
    resolve, union,
};
use crate::support_model::flavour::SmallOverhang;

/// One cluster of overhang islands.
#[derive(Clone, Debug, Default)]
pub(super) struct Cluster {
    pub members: BTreeMap<usize, Vec<Island>>,
    pub outline: Rings,
    pub outline_box: Option<IntBox>,
    pub min_layer: usize,
    pub max_layer: usize,
    pub cantilever: bool,
    pub sharp_tail: bool,
    pub small: bool,
    pub big_flat: bool,
    /// B: the cluster touches an enforcer on one of its layers.
    pub enforce_add: bool,
}

impl Cluster {
    fn new(island: &Island, layer: usize) -> Cluster {
        let outline = resolve(island, FillRule::NonZero);
        Cluster {
            members: BTreeMap::from([(layer, vec![island.clone()])]),
            outline_box: bbox(&outline),
            outline,
            min_layer: layer,
            max_layer: layer,
            ..Cluster::default()
        }
    }

    fn height(&self) -> usize {
        self.max_layer - self.min_layer + 1
    }

    fn members_on(&self, layer: usize) -> Rings {
        self.members
            .get(&layer)
            .map(|members| members.iter().flatten().cloned().collect())
            .unwrap_or_default()
    }
}

/// All clusters, in creation order.
#[derive(Clone, Debug, Default)]
pub(super) struct Clustered {
    pub clusters: Vec<Cluster>,
}

/// Groups the overhangs of every layer bottom-up and classifies the clusters.
pub(super) fn group(context: &Context, passes: &[LayerPass], tails: &[LayerTails]) -> Clustered {
    let mut clusters: Vec<Cluster> = Vec::new();
    for (layer, pass) in passes.iter().enumerate() {
        let cantilevers: Rings = flatten(pass.cantilevers.clone());
        for island in &pass.overhangs {
            let index = join(context, &mut clusters, island, layer);
            if overlaps(island, &cantilevers) {
                clusters[index].cantilever = true;
            }
        }
    }
    if context.auto && context.remove_small {
        for cluster in &mut clusters {
            classify(context, cluster, tails);
        }
    }
    if context.flavour.paint_in_detection() {
        let big = context.units(BIG_OVERHANG_MM);
        for cluster in clusters
            .iter_mut()
            .filter(|cluster| !cluster.small || cluster.enforce_add)
        {
            cluster.big_flat = area(&cluster.outline) > big * big;
        }
    }
    Clustered { clusters }
}

/// Adds `island` to the first cluster it continues, or starts a new one; returns its index.
fn join(context: &Context, clusters: &mut Vec<Cluster>, island: &Island, layer: usize) -> usize {
    if layer >= 1 {
        let grown = context.grow(island, context.width_mm);
        let grown_box = bbox(&grown);
        for (index, cluster) in clusters.iter_mut().enumerate() {
            let below = cluster.members_on(layer - 1);
            let inside_box = match (grown_box, cluster.outline_box) {
                (Some(a), Some(b)) => IntBox::overlap(&a, &b),
                _ => false,
            };
            if below.is_empty() || !inside_box || !overlaps(&below, &grown) {
                continue;
            }
            let members = cluster.members.entry(layer).or_default();
            if members.is_empty() || context.flavour.clusters_keep_all_members() {
                members.push(island.clone());
            }
            cluster.outline = union(&cluster.outline, &grown);
            cluster.outline_box = IntBox::merge(cluster.outline_box, grown_box);
            cluster.min_layer = cluster.min_layer.min(layer);
            cluster.max_layer = cluster.max_layer.max(layer);
            return index;
        }
    }
    clusters.push(Cluster::new(island, layer));
    clusters.len() - 1
}

/// Sharp-tail flag, B's enforcer rescue flag, and the small-overhang verdict (§3.6).
fn classify(context: &Context, cluster: &mut Cluster, tails: &[LayerTails]) {
    let tail_rings = |layer: usize| -> Rings {
        tails[layer]
            .iter()
            .flat_map(|(tail, _)| tail.iter().cloned())
            .collect()
    };
    let layers = cluster.min_layer..=cluster.max_layer;
    match context.flavour.small_overhang() {
        SmallOverhang::Orca => {
            cluster.sharp_tail = layers
                .clone()
                .any(|layer| overlaps(&tail_rings(layer), &cluster.outline));
            if !cluster.sharp_tail && !cluster.cantilever {
                let eroded = context.grow(&cluster.outline, -context.width_mm);
                let (width, height) = size(&eroded);
                cluster.small = width < 2.0 * context.width || height < 2.0 * context.width;
            }
        }
        SmallOverhang::Bambu => {
            cluster.sharp_tail = layers
                .clone()
                .any(|layer| overlaps(&tail_rings(layer), &cluster.members_on(layer)));
            cluster.enforce_add = layers
                .clone()
                .take_while(|&layer| layer < context.enforcer_count)
                .any(|layer| {
                    !context.enforcer(layer).is_empty()
                        && overlaps(context.enforcer(layer), &cluster.members_on(layer))
                });
            cluster.small =
                !cluster.sharp_tail && !cluster.cantilever && bambu_small(context, cluster);
        }
    }
}

/// B's rule: every layer's members narrow (eroded by w/2, widest min box side ≤ w), with
/// average narrow width below w/2, average area below 4 mm² and fewer than 3 layers; or the
/// outline eroded by 2.5w gone or under 2 mm in both box sides. Averages are f32 as upstream.
fn bambu_small(context: &Context, cluster: &Cluster) -> bool {
    let width = context.width;
    let mut all_narrow = true;
    let (mut width_sum, mut area_sum, mut count) = (0.0f32, 0.0f32, 0usize);
    for members in cluster.members.values() {
        let rings: Rings = members.iter().flatten().cloned().collect();
        let eroded = context.grow(&rings, -context.width_mm / 2.0);
        let narrow = islands(&eroded)
            .iter()
            .map(|island| {
                let (w, h) = size(island);
                w.min(h)
            })
            .fold(0.0f64, f64::max);
        if narrow > width {
            all_narrow = false;
        }
        width_sum += narrow as f32;
        area_sum += area(&rings) as f32;
        count += 1;
    }
    let length = context.units(SMALL_OVERHANG_MM);
    if count > 0 {
        let average_width = width_sum / count as f32;
        let average_area = area_sum / count as f32;
        if all_narrow
            && f64::from(average_width) < width / 2.0
            && f64::from(average_area) < length * length
            && cluster.height() < 3
        {
            return true;
        }
    }
    let eroded = context.grow(
        &cluster.outline,
        -SMALL_OVERHANG_RADIUS_WIDTHS * context.width_mm,
    );
    let (box_width, box_height) = size(&eroded);
    eroded.is_empty() || (box_width < length && box_height < length)
}

/// The overhangs clusters hand to the per-layer assembly: B every member of non-small
/// clusters (small ones with an enforcer only where they meet it, cut to it), O/R/S the kept
/// member of each non-small cluster. Per layer in cluster creation order.
pub(super) fn emitted(context: &Context, clustered: &Clustered) -> Vec<Vec<TypedOverhang>> {
    let mut layers: Vec<Vec<TypedOverhang>> = vec![Vec::new(); context.layers()];
    let bambu = context.flavour.paint_in_detection();
    for cluster in &clustered.clusters {
        if cluster.small && !(bambu && cluster.enforce_add) {
            continue;
        }
        let flags = OverhangFlags {
            cantilever: cluster.cantilever,
            sharp_tail: cluster.sharp_tail && bambu,
            small: cluster.small,
            big_flat: cluster.big_flat,
            ..OverhangFlags::default()
        };
        for (&layer, members) in &cluster.members {
            let pieces = if cluster.small {
                let enforcer = context.enforcer(layer.min(context.layers() - 1));
                let rings: Rings = members.iter().flatten().cloned().collect();
                if layer >= context.enforcer_count
                    || enforcer.is_empty()
                    || !overlaps(enforcer, &rings)
                {
                    continue;
                }
                intersection_islands(&rings, enforcer)
            } else {
                members.clone()
            };
            layers[layer].extend(
                pieces
                    .into_iter()
                    .map(|island| TypedOverhang { island, flags }),
            );
        }
    }
    layers
}

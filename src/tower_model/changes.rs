//! Tool changes per layer from the filament sets of the layers (spec 04 §4.2).
//!
//! A layer printing n filaments makes n − 1 changes inside it, plus one more when its first
//! filament is not the one the layer below left loaded. The loaded one is surely on the layer
//! when every filament of the layer below is (the conservative rule of 04 §4.2). Orca-family
//! builds then start with it (S `Print.cpp:3379`); the U1 corpus (R, the goat and six
//! synthetic shapes) never shows more changes than this rule allows.
//!
//! BambuStudio orders all layers together to minimise flushing and may start a layer with
//! another filament so that it ends on the one a single-filament next layer needs, paying the
//! change one layer early. Corpus: the A2L t-bar probe changed tools twice on its layer at
//! z 15.04 (filaments {1, 3}, 3 loaded, the next layer only 3), and its tower came out 6.9 mm
//! wider than the plain rule predicts. Whether the loaded filament is that one depends on how
//! many layers the run before had, which the mesh rule only knows to ±1 layer, so the extra
//! change is allowed on every multi-filament layer followed by a single filament it prints.
//! Its cost: the P1S synthetic probes whose run happened to end on the other filament get a
//! two-change tower they did not need (4.96 mm wider than printed).

use std::collections::BTreeSet;

use crate::first_layer::SlicerBuild;

/// How a build orders the filaments of a layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayerOrdering {
    /// The loaded filament first whenever the layer uses it (O, R, S).
    LoadedFirst,
    /// Flush-minimal over all layers, paying a change early before a single-filament layer (B).
    WholeObject,
}

impl LayerOrdering {
    pub fn of(build: SlicerBuild) -> LayerOrdering {
        match build {
            SlicerBuild::BambuStudio => LayerOrdering::WholeObject,
            _ => LayerOrdering::LoadedFirst,
        }
    }
}

/// Upper bound of the tool changes on each layer. The first layer never counts its first
/// filament: the slicer primes it instead of changing into it, so it can also end on any.
pub fn changes_per_layer(sets: &[BTreeSet<u32>], ordering: LayerOrdering) -> Vec<usize> {
    (0..sets.len())
        .map(|layer| {
            let set = &sets[layer];
            if set.is_empty() {
                return 0;
            }
            let loaded_kept = layer == 0 || sets[layer - 1].is_subset(set);
            let paid_early = ordering == LayerOrdering::WholeObject
                && layer > 0
                && set.len() >= 2
                && sets
                    .get(layer + 1)
                    .is_some_and(|next| next.len() == 1 && next.is_subset(set));
            if loaded_kept && !paid_early {
                set.len() - 1
            } else {
                set.len()
            }
        })
        .collect()
}

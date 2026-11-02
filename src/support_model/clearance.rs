//! Stage A5: the model volumes of the tree generator (spec 02 §2), per (radius key, layer):
//!
//! - collision Coll(K, l): where a branch centre of key K would touch the object (own layer
//!   and the z-gap layers below at K + xy_min, the z-gap layers above at the reduced ρ_i);
//! - hole-free collision CollHF(K, l): the collision closed at the "increase until" radius,
//!   for keys below it;
//! - avoidance Avoid_τ(K, l) to the bed for the three speeds (slow, fast-safe, fast);
//! - placeable areas (to-model only, inactive while supports stand on the bed);
//! - wall restrictions WR(K, l), regular and tight-xy.
//!
//! Everything is a pure function of (K, l) and the inputs, memoized lazily behind a mutex in
//! ordered maps, so any member order and any thread count gives the same polygons. B appends
//! its bed-obstacle ring (`bed`) to every layer outline.

mod bed;
mod keys;
mod volumes;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

pub use bed::{BedPlacement, bed_obstacle};
pub use keys::RadiusKeys;

use super::flavour::Flavour;
use super::settings::Derived;
use crate::polygon_ops::{Grid, Rings};

/// The three avoidance speeds (spec 02 §2.5).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Avoidance {
    /// Moves at the slow (preferred) angle; closes holes for small keys.
    Slow,
    /// Fast moves that still avoid holes the branch will outgrow.
    FastSafe,
    Fast,
}

/// Which volume a cache entry holds.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Volume {
    /// The per-layer outline grown by one value (shared by neighbouring collisions).
    Grown,
    Collision,
    HoleFree,
    Avoid(Avoidance),
    Placeable,
    PlaceableBase,
    Wall,
    WallTight,
}

type Cache = BTreeMap<(Volume, i64, usize), Arc<Rings>>;

/// The clearance volumes of one object (one stage-A run), shared by every ensemble member.
pub struct Clearance {
    flavour: Flavour,
    grid: Grid,
    keys: RadiusKeys,
    /// O_l, with B's bed ring appended (upstream's separate machine-border term is empty in
    /// every build, B `TreeModelVolumes.cpp` "Machine border is currently ignored").
    outlines: Vec<Rings>,
    border: Rings,
    xy_min: i64,
    z_top: usize,
    z_bottom: usize,
    /// V_f′, V_s′: the volumes' move distances (V − 2).
    move_fast: i64,
    move_slow: i64,
    /// R_inc: radius growth stops being tip-driven here (key threshold of CollHF).
    increase_until: i64,
    resolution: i64,
    cache: Mutex<Cache>,
}

impl Clearance {
    /// Volumes over the outlines O_l (`LayerSections::outline`), with `border` (B's bed ring,
    /// empty otherwise) appended to every layer.
    pub fn new(outlines: &[Rings], border: Rings, derived: &Derived) -> Clearance {
        let with_border = outlines
            .iter()
            .map(|outline| outline.iter().chain(&border).cloned().collect())
            .collect();
        Clearance {
            flavour: derived.flavour,
            grid: derived.grid,
            keys: RadiusKeys::new(derived),
            outlines: with_border,
            border,
            xy_min: derived.xy_min,
            z_top: derived.z_top_layers,
            z_bottom: derived.z_bottom_layers,
            move_fast: derived.volume_move_fast,
            move_slow: derived.volume_move_slow,
            increase_until: derived.increase_until_radius,
            resolution: derived.resolution,
            cache: Mutex::new(Cache::new()),
        }
    }

    pub fn keys(&self) -> &RadiusKeys {
        &self.keys
    }

    pub fn grid(&self) -> Grid {
        self.grid
    }

    /// N_obj: layers of the object.
    pub fn layers(&self) -> usize {
        self.outlines.len()
    }

    /// The bed ring pieces (B), empty for Orca builds.
    pub fn border(&self) -> &Rings {
        &self.border
    }

    /// Coll for a branch of `radius` (tight or regular xy) on `layer`.
    pub fn collision(&self, radius: i64, layer: usize, tight_xy: bool) -> Arc<Rings> {
        self.collision_at(self.keys.key(radius, tight_xy), layer)
    }

    /// Coll at an exact key.
    pub fn collision_at(&self, key: i64, layer: usize) -> Arc<Rings> {
        self.memo(Volume::Collision, key, layer, || {
            volumes::collision(self, key, layer)
        })
    }

    /// CollHF at an exact key below R_inc + δ.
    pub fn hole_free_at(&self, key: i64, layer: usize) -> Arc<Rings> {
        self.memo(Volume::HoleFree, key, layer, || {
            volumes::hole_free(self, key, layer)
        })
    }

    /// Avoid to the bed for `radius` at speed `kind`; layer 0 is the collision. Fast-safe
    /// requests at or above R_inc + δ are served as fast (02 §2.5 type redirect).
    pub fn avoidance(
        &self,
        radius: i64,
        layer: usize,
        kind: Avoidance,
        tight_xy: bool,
    ) -> Arc<Rings> {
        let key = self.keys.key(radius, tight_xy);
        if layer == 0 {
            return self.collision_at(key, 0);
        }
        let kind = if kind == Avoidance::FastSafe && key >= self.hole_free_limit() {
            Avoidance::Fast
        } else {
            kind
        };
        volumes::avoidance(self, kind, key, layer)
    }

    /// Placeable areas for `radius` on `layer` (to-model supports; never used by the goats).
    pub fn placeable(&self, radius: i64, layer: usize) -> Arc<Rings> {
        let key = self.keys.round_up(radius);
        self.memo(Volume::Placeable, key, layer, || {
            volumes::placeable(self, key, layer)
        })
    }

    /// WR for `radius` on `layer` ≥ 1: keyed by q(r) without δ; the tight variant exists only
    /// when δ > 0 (02 §2.7).
    pub fn wall_restriction(&self, radius: i64, layer: usize, tight_xy: bool) -> Arc<Rings> {
        let key = self.keys.round_up(radius);
        let tight = tight_xy && self.keys.delta() > 0;
        let volume = if tight {
            Volume::WallTight
        } else {
            Volume::Wall
        };
        self.memo(volume, key, layer, || {
            volumes::wall_restriction(self, key, layer, tight)
        })
    }

    /// The keys upstream precalculates collisions for (02 §2.9): 0, δ, the CollHF source key,
    /// and the tight and regular keys of every tip and trunk radius up to `max_depth` layers
    /// below a tip, plus the foot-zone minimum radii. Smoothing looks up the largest
    /// precalculated key below a radius.
    pub fn precalculated_keys(&self, derived: &Derived, max_depth: i64) -> Vec<i64> {
        let mut keys = vec![
            0,
            self.keys.delta(),
            self.keys.key(self.increase_until, false),
        ];
        for depth in 0..=max_depth.max(0) {
            let radius = derived.radius(depth, 0.0);
            keys.push(self.keys.key(radius, true));
            keys.push(self.keys.key(radius, false));
        }
        for layer in 0..derived.foot_layers.max(0) {
            keys.push(self.keys.key(derived.recommended_min_radius(layer), false));
        }
        keys.sort_unstable();
        keys.dedup();
        keys
    }

    fn hole_free_limit(&self) -> i64 {
        self.increase_until + self.keys.delta()
    }

    fn cached(&self, volume: Volume, key: i64, layer: usize) -> Option<Arc<Rings>> {
        self.lock().get(&(volume, key, layer)).cloned()
    }

    fn store(&self, volume: Volume, key: i64, layer: usize, rings: Rings) -> Arc<Rings> {
        let shared = Arc::new(rings);
        self.lock()
            .entry((volume, key, layer))
            .or_insert_with(|| shared.clone())
            .clone()
    }

    /// The entry, computed outside the lock on a miss (values are pure functions, so a
    /// concurrent duplicate computes the same polygons).
    fn memo(
        &self,
        volume: Volume,
        key: i64,
        layer: usize,
        compute: impl FnOnce() -> Rings,
    ) -> Arc<Rings> {
        if let Some(found) = self.cached(volume, key, layer) {
            return found;
        }
        let rings = compute();
        self.store(volume, key, layer, rings)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

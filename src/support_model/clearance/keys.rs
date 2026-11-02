//! Radius keys, our `radius_key` (spec 02 §2.2): every clearance request rounds its radius up
//! to a coarse key, and the volumes are cached per (key, layer). The rounding, not the nominal
//! xy distance, sets where trunks and feet land (02 §0.5).
//!
//! Rounding q(r): 0 stays 0; up to the tip radius r₀ it is r₀; then a linear stretch toward
//! the 1.5 mm threshold in steps of at most the threshold (one step for the goats, 1.1 mm),
//! and from there ×1.5 truncated until the key reaches r. Keys between r₀ and the branch
//! radius that no tip radius can produce are "ignorable" and skipped. The thresholds are
//! grid-truncated like every other constant (1.5 and 0.5 mm happen to be exact on both grids:
//! B keys 40 000, 150 000, 260 000, 390 000 units; Orca 0.4, 1.5, 2.6, 3.9 mm in nm).

use serde::{Deserialize, Serialize};

use crate::polygon_ops::Grid;
use crate::support_model::settings::Derived;

/// Exponential growth of keys beyond the linear stretch.
const KEY_FACTOR: f64 = 1.5;

/// The linear stretch ends at 1.5 mm (`1.0 · KEY_FACTOR`).
const KEY_THRESHOLD_MM: f64 = 1.0 * KEY_FACTOR;

/// Below this stretch length a single fixed step is taken instead.
const KEY_RESOLUTION_MM: f64 = 0.5;

/// The key ladder of one object's settings.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RadiusKeys {
    tip_radius: i64,
    threshold: i64,
    resolution: i64,
    /// δ = xy − xy_min, added in regular (not tight-xy) mode.
    delta: i64,
    /// Ascending keys no tip radius produces.
    ignorable: Vec<i64>,
}

impl RadiusKeys {
    pub fn new(derived: &Derived) -> RadiusKeys {
        let grid: Grid = derived.grid;
        let mut keys = RadiusKeys {
            tip_radius: derived.radius(0, 0.0),
            threshold: grid.length(KEY_THRESHOLD_MM),
            resolution: grid.length(KEY_RESOLUTION_MM),
            delta: derived.xy_delta,
            ignorable: Vec::new(),
        };
        let mut reachable: Vec<i64> = (0..=derived.tip_layers)
            .flat_map(|depth| {
                let radius = derived.radius(depth, 0.0);
                [keys.round_up(radius), keys.round_up(radius + keys.delta)]
            })
            .collect();
        reachable.sort_unstable();
        reachable.dedup();
        let mut radius = keys.tip_radius;
        while radius <= derived.branch_radius {
            if reachable.binary_search(&radius).is_err() {
                keys.ignorable.push(radius);
            }
            radius = keys.round_up(radius + 1);
        }
        keys
    }

    /// q(r): the rounding without the xy mode.
    pub fn round_up(&self, radius: i64) -> i64 {
        if radius == 0 {
            return 0;
        }
        let mut key = self.tip_radius;
        if radius <= self.tip_radius {
            return key;
        }
        let stretch = self.threshold - self.tip_radius;
        if stretch > self.resolution {
            let steps = (stretch + self.threshold - 1) / self.threshold;
            let step = stretch / steps;
            key += step;
            for _ in 0..steps {
                if key >= radius && !self.ignored(key) {
                    return key;
                }
                key += step;
            }
        } else {
            key += self.resolution;
        }
        while key < radius || self.ignored(key) {
            key = (key as f64 * KEY_FACTOR) as i64;
        }
        key
    }

    /// K(r, mode): tight-xy requests use q(r); regular ones q(r + δ), and exactly δ for r = 0.
    pub fn key(&self, radius: i64, tight_xy: bool) -> i64 {
        if tight_xy {
            self.round_up(radius)
        } else if radius > 0 {
            self.round_up(radius + self.delta)
        } else {
            self.delta
        }
    }

    /// The largest radius that still maps to the same key as `radius` (radius growth checks).
    pub fn next_key(&self, radius: i64, tight_xy: bool) -> i64 {
        if tight_xy {
            self.round_up(radius)
        } else {
            self.round_up(radius + self.delta) - self.delta
        }
    }

    pub fn delta(&self) -> i64 {
        self.delta
    }

    pub fn ignorable(&self) -> &[i64] {
        &self.ignorable
    }

    fn ignored(&self, key: i64) -> bool {
        self.ignorable.binary_search(&key).is_ok()
    }
}

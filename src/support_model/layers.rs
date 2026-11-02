//! Stage A1: the slicer's layer grid (spec 01 §1.2). A height profile `[z0, h0, z1, h1, ...]`
//! comes from a stored variable-layer profile or from the height ranges; only a *fixed*
//! profile (first layer h₀, then h everywhere) is supported, because a variable one makes
//! Bambu switch to Tree Hybrid and Orca refuse Organic (spec 01 §1.2.1, design §5.7).
//! Layers are emitted while their mid-height is below the object height H; heights add up in
//! f64, and slice planes are handed to the mesh slicer as f32.

use serde::{Deserialize, Serialize};

use super::SupportError;
use super::settings::SupportSettings;

/// `is_approx` tolerance of the profile rules and the gap snapping (`EPSILON`).
const EPSILON: f64 = 1e-4;

/// A stored profile is used only if it ends this close to the object height.
const PROFILE_TOP_TOLERANCE: f64 = 1e-3;

/// The smallest minimum layer height the slicers accept (`MIN_LAYER_HEIGHT`).
const MIN_LAYER_HEIGHT_FLOOR: f64 = 0.01;

/// One printed layer: material in (bottom, top], sliced at `slice_z`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Layer {
    pub bottom: f64,
    /// `print_z` (no raft).
    pub top: f64,
    /// The slice plane, already rounded to f32 as the mesh slicer receives it.
    pub slice_z: f64,
}

impl Layer {
    /// f32 layer height, as the overhang stage divides it (spec 01 §3.0 d_k).
    pub fn height_f32(&self) -> f32 {
        (self.top - self.bottom) as f32
    }

    /// The absolute bands of `band_mm` this layer's interval (bottom, top] meets with
    /// positive length (design §4.1 band rule): a top exactly on a boundary stays below.
    pub fn bands(&self, band_mm: f64) -> std::ops::RangeInclusive<usize> {
        let first = libm::floor(self.bottom / band_mm).max(0.0) as usize;
        let last = (libm::ceil(self.top / band_mm) as usize)
            .saturating_sub(1)
            .max(first);
        first..=last
    }
}

/// The layers of one object, bottom up.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct LayerGrid {
    pub layers: Vec<Layer>,
}

impl LayerGrid {
    /// The grid for an object of height `height` (its normal parts, standing on z = 0).
    pub fn new(settings: &SupportSettings, height: f64) -> Result<LayerGrid, SupportError> {
        if height.is_nan() || height <= 0.0 {
            return Err(SupportError::Invalid(format!(
                "object height {height} is not positive"
            )));
        }
        let first = settings.first_layer();
        let layer = settings.layer_height;
        let profile = profile(settings, height, first);
        if !is_fixed(&profile, first, layer) {
            return Err(SupportError::unsupported(
                "layer_height_profile",
                "variable",
                "variable layer heights switch Bambu to Tree Hybrid and are refused by Orca (spec 01 §1.2.1)",
            ));
        }
        let min_height = settings
            .min_layer_height
            .max(MIN_LAYER_HEIGHT_FLOOR)
            .min(layer);
        Ok(LayerGrid {
            layers: walk(&profile, first, min_height, height),
        })
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Slice planes, ascending (paint projection, sections).
    pub fn slice_planes(&self) -> Vec<f64> {
        self.layers.iter().map(|layer| layer.slice_z).collect()
    }

    /// Drops trailing layers (empty sections at the top of the stack, spec 01 §1.2 rule 7).
    pub fn truncate(&mut self, count: usize) {
        self.layers.truncate(count);
    }
}

/// The height profile: a stored one that fits the object, else the one built from ranges.
fn profile(settings: &SupportSettings, height: f64, first: f64) -> Vec<f64> {
    if let Some(stored) = &settings.layer_profile
        && stored.len() >= 4
        && stored.len() % 2 == 0
        && (stored[stored.len() - 2] - height).abs() <= PROFILE_TOP_TOLERANCE
        && (stored[1] - first).abs() < EPSILON
    {
        return stored.clone();
    }
    profile_from_ranges(settings, height, first)
}

/// Height ranges → profile (spec 01 §1.2 rule 1): the first layer, then the ranges sorted,
/// trimmed so none overlaps the previous and clipped at H, gaps filled with h; a third
/// repetition of one height moves the previous point up instead of adding one.
fn profile_from_ranges(settings: &SupportSettings, height: f64, first: f64) -> Vec<f64> {
    let layer = settings.layer_height;
    let mut ranges: Vec<(f64, f64, f64)> = vec![(0.0, first, first)];
    let mut sorted = settings.layer_ranges.clone();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    for (low, high, range_height) in sorted {
        let low = low.max(ranges.last().map_or(0.0, |range| range.1));
        let high = high.min(height);
        if low + EPSILON < high {
            ranges.push((low, high, range_height.unwrap_or(layer)));
        }
    }
    let mut profile = Vec::new();
    for (low, high, range_height) in ranges {
        let last_z = last_z(&profile);
        if low > last_z + EPSILON {
            append(&mut profile, last_z, layer);
            append(&mut profile, low, layer);
        }
        append(&mut profile, low, range_height);
        append(&mut profile, high, range_height);
    }
    let last_z = last_z(&profile);
    if last_z < height {
        append(&mut profile, last_z, layer);
        append(&mut profile, height, layer);
    }
    profile
}

fn last_z(profile: &[f64]) -> f64 {
    if profile.is_empty() {
        0.0
    } else {
        profile[profile.len() - 2]
    }
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < EPSILON
}

fn append(profile: &mut Vec<f64>, z: f64, height: f64) {
    if profile.len() >= 2 && approx(profile[profile.len() - 1], height) {
        if approx(profile[profile.len() - 2], z) {
            return;
        }
        if profile.len() >= 4 && approx(profile[profile.len() - 3], height) {
            let index = profile.len() - 2;
            profile[index] = z;
            return;
        }
    }
    profile.push(z);
    profile.push(height);
}

/// The fixed-profile test (spec 01 §1.2 rule 2).
fn is_fixed(profile: &[f64], first: f64, layer: f64) -> bool {
    if profile.len() != 4 && profile.len() != 8 {
        return false;
    }
    let step_one = approx(profile[1], profile[3]);
    let step_two =
        profile.len() == 4 || (profile[2] == profile[4] && approx(profile[5], profile[7]));
    if !step_one || !step_two || profile[2] < 0.5 * first + EPSILON || !approx(profile[3], first) {
        return false;
    }
    let top = profile[profile.len() - 2];
    let second_mid = first + 0.5 * layer;
    if second_mid > top {
        return true;
    }
    second_mid >= profile[profile.len() - 4] + EPSILON && approx(profile[profile.len() - 1], layer)
}

/// The layer walk (spec 01 §1.2 rule 3): layer 0 = [0, h₀], then heights read from the
/// profile at the next slice position; a layer exists only if its mid-height is below H.
fn walk(profile: &[f64], first: f64, min_height: f64, height: f64) -> Vec<Layer> {
    let mut layers = vec![layer(0.0, first)];
    let mut top = first;
    let mut index = 0usize;
    let mut slice = top + 0.5 * min_height;
    while slice < height {
        let mut next = index + 2;
        while next < profile.len() && slice >= profile[next] {
            index = next;
            next += 2;
        }
        let mut step = profile[index + 1];
        if next < profile.len() {
            let (z1, h1, z2, h2) = (
                profile[index],
                profile[index + 1],
                profile[next],
                profile[next + 1],
            );
            step = h1 + (h2 - h1) * ((slice - z1) / (z2 - z1));
        }
        slice = top + 0.5 * step;
        if slice >= height {
            break;
        }
        layers.push(layer(top, top + step));
        top += step;
        slice = top + 0.5 * min_height;
    }
    layers
}

fn layer(bottom: f64, top: f64) -> Layer {
    Layer {
        bottom,
        top,
        slice_z: f64::from((0.5 * (bottom + top)) as f32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_height_ranges_collapse_to_a_fixed_profile() {
        let mut ranges = vec![(14.9, 18.24, Some(0.28)), (33.14, 60.0, Some(0.28))];
        let settings = |ranges: Vec<(f64, f64, Option<f64>)>| {
            let mut settings = crate::support_model::settings::tests_support::settings();
            settings.layer_ranges = ranges;
            settings
        };
        let profile = profile_from_ranges(&settings(ranges.clone()), 44.7435, 0.25);
        assert_eq!(
            profile,
            vec![0.0, 0.25, 0.25, 0.25, 0.25, 0.28, 44.7435, 0.28]
        );
        assert!(is_fixed(&profile, 0.25, 0.28));
        ranges[1].2 = Some(0.2);
        let variable = profile_from_ranges(&settings(ranges), 44.7435, 0.25);
        assert!(!is_fixed(&variable, 0.25, 0.28));
    }

    #[test]
    fn band_membership_follows_the_layer_interval() {
        let at = |bottom, top| {
            Layer {
                bottom,
                top,
                slice_z: 0.5 * (bottom + top),
            }
            .bands(2.0)
        };
        assert_eq!(at(1.88, 2.16), 0..=1);
        assert_eq!(at(1.72, 2.0), 0..=0);
        assert_eq!(at(0.0, 0.2), 0..=0);
        assert_eq!(at(2.0, 2.28), 1..=1);
    }
}

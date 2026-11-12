//! The support model as one call (design §5.1): everything the packer needs of an object at one
//! rotation class, with the cache in front. Stage A runs once, the ensemble members after it;
//! a hit in `--cache-dir` skips both.

use std::path::Path;
use std::time::Instant;

use super::cache::{Ensemble, cache_key, load, store};
use super::ensemble::{distinct, run_members_with, union};
use super::flavour::Flavour;
use super::merge::MergeOrder;
use super::settings::{SupportSettings, check_supported};
use super::{StageAInput, SupportError, stage_a};
use crate::embedded_mesh::ObjectMesh;
use crate::object_print::Realization;

/// What `predict` needs.
pub struct PredictInput<'a> {
    /// The object in its source frame (`load_object_mesh`); the rotation class is applied here.
    pub mesh: &'a ObjectMesh,
    pub settings: &'a SupportSettings,
    pub flavour: Flavour,
    pub rotation_deg: f64,
    pub cache_dir: Option<&'a Path>,
    /// Which readings of the merge order run (an ensemble axis, `merge::MergeOrder`).
    pub orders: &'a [MergeOrder],
}

/// The ensemble of one class and how it was obtained.
#[derive(Clone, Debug)]
pub struct Prediction {
    pub ensemble: Ensemble,
    pub cached: bool,
    pub cache_key: String,
    pub seconds: f64,
    /// Members run (before dedup); 0 on a cache hit.
    pub runs: usize,
}

/// Predicts the supports of `input`: `Unsupported` before any geometry when the project asks
/// for what the model does not describe (design §5.7).
pub fn predict(input: &PredictInput) -> Result<Prediction, SupportError> {
    let started = Instant::now();
    // Supports off in the project: no member can grow a branch, so neither stage A nor the
    // ensemble runs (a keychain spent 0.3 s per rotation on 20 empty members). The settings
    // checks still run: some refuse a project with or without supports.
    if !input.settings.enable_support {
        check_supported(input.settings, input.flavour)?;
        return Ok(Prediction {
            ensemble: Ensemble {
                members: Vec::new(),
                graphs: 0,
                realizations: vec![Realization::default()],
            },
            cached: false,
            cache_key: String::new(),
            seconds: started.elapsed().as_secs_f64(),
            runs: 0,
        });
    }
    let overlaps = input.flavour.overlap_readings();
    let key = cache_key(
        input.mesh,
        input.settings,
        input.flavour,
        input.rotation_deg,
        input.orders,
        overlaps,
    );
    if let Some(dir) = input.cache_dir
        && let Some(ensemble) = load(dir, &key)
    {
        return Ok(Prediction {
            ensemble,
            cached: true,
            cache_key: key,
            seconds: started.elapsed().as_secs_f64(),
            runs: 0,
        });
    }
    let rotated = input.mesh.rotated(input.rotation_deg);
    let stage = stage_a(&StageAInput {
        mesh: &rotated,
        settings: input.settings,
        flavour: input.flavour,
        bed: None,
    })?;
    let runs = run_members_with(&stage, input.orders, overlaps);
    let ensemble = Ensemble {
        members: runs.iter().map(|run| run.summary()).collect(),
        graphs: distinct(&runs).len(),
        realizations: vec![union(&stage, &runs)],
    };
    if let Some(dir) = input.cache_dir {
        store(dir, &key, &ensemble).map_err(|error| SupportError::Invalid(error.to_string()))?;
    }
    Ok(Prediction {
        ensemble,
        cached: false,
        cache_key: key,
        seconds: started.elapsed().as_secs_f64(),
        runs: runs.len(),
    })
}

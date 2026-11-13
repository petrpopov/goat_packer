//! The modelling step without a slicer (`--support-source model`): per rotation class the
//! supports come from the support model's ensemble, grown by the flavour's calibrated margins,
//! the landing from the first-layer model and the prime tower from the tower model.

use goat_packer::{
    first_layer::landing,
    mesh_section::{BodyBands, body_print},
    object_print::{Margins, ObjectPrint, Realization},
    progress::{self, Stage, StageDetail},
    support_model::{
        cache::{EnvelopePart, envelope_key, load_envelope, store_envelope},
        envelope::{envelope, envelope_banded},
        flavour::Flavour,
        predict::{PredictInput, Prediction, predict},
        settings::{ProjectConfig, SupportSettings},
        SupportError,
    },
};

use crate::AppResult;
use crate::args::SupportSource;
use crate::job::Job;
use crate::model::{ClassSummary, Modelled};
use crate::plan::plan_of;
use crate::tower::tower_spec;

/// Predicts every rotation class and builds the plan.
pub fn modelled_plate(job: &Job) -> AppResult<Modelled> {
    let config = ProjectConfig::load(&job.args.source_3mf)?;
    // Settings the model cannot read are a refusal like any unsupported one (`Auto` probes).
    let mut settings = SupportSettings::from_config(&config)
        .map_err(|reason| SupportError::unsupported("project settings", "", &reason))?;
    // The output 3MF carries the first-layer gap this job forced (`own_gap`): the supports
    // are predicted with the value the slicer will read, as the probe copies are sliced.
    if job.own_gap.edits_project() {
        settings.first_layer_gap = job.own_gap.first_layer_mm;
    }
    let builds = job.flavour.support_builds();
    let margins = match job.args.support_margins {
        Some((first_layer_mm, band_mm)) => Margins {
            first_layer_mm,
            band_mm,
        },
        None => job.flavour.margins(),
    };
    let mut prints = Vec::new();
    let mut summaries = Vec::new();
    let mut seconds = 0.0;
    let count = job.args.rotations.len();
    for (index, &rotation) in job.args.rotations.iter().enumerate() {
        progress::stage_step(
            Stage::Modelling,
            &format!("predicting supports (rotation {rotation}°)..."),
            Some(1),
            &StageDetail::new(index, count)
                .with("rotation_deg", rotation)
                .with("class", index + 1)
                .with("classes", count),
        );
        let mut predictions = Vec::new();
        for &build in &builds {
            let orders = job
                .args
                .support_orders
                .clone()
                .unwrap_or_else(|| build.default_orders().to_vec());
            let prediction = predict(&PredictInput {
                mesh: &job.mesh,
                settings: &settings,
                flavour: build,
                rotation_deg: rotation,
                cache_dir: job.args.cache_dir.as_deref(),
                orders: &orders,
                precision: job.args.precision,
            })?;
            if !settings.enable_support {
                println!("supports are off in the project: none predicted (rotation {rotation}°)");
            } else {
                println!(
                    "support model {}, rotation {rotation}°: {} distinct graphs of {} members in {:.1} s{}",
                    build.name(),
                    prediction.ensemble.graphs,
                    prediction.ensemble.members.len(),
                    prediction.seconds,
                    if prediction.cached { " (cache)" } else { "" }
                );
            }
            seconds += prediction.seconds;
            predictions.push((build, prediction));
        }
        prints.push(class_print(job, rotation, &predictions, margins)?);
        summaries.push(ClassSummary {
            rotation_deg: rotation,
            runs: predictions
                .iter()
                .map(|(_, prediction)| prediction.ensemble.members.len())
                .sum(),
            saturated: true,
            cached: predictions.iter().all(|(_, prediction)| prediction.cached),
            growth_mm2: Vec::new(),
            landing_outside_model_mm2: 0.0,
        });
    }
    let tower = tower_spec(job, &[])?;
    Ok(Modelled {
        plan: plan_of(job, prints, tower),
        source: SupportSource::Model,
        summaries,
        probe_seconds: 0.0,
        model_seconds: seconds,
    })
}

/// The print of one class: body kinds from the mesh, landing from the first-layer model, the
/// supports from the union of every build's envelope, each build's ensemble grown by its own
/// margins (or all by `--support-margins`).
fn class_print(
    job: &Job,
    rotation: f64,
    predictions: &[(Flavour, Prediction)],
    margins: Margins,
) -> AppResult<ObjectPrint> {
    let layer_height = job
        .settings
        .number("layer_height")
        .ok_or("the project has no layer_height")?;
    let first_height = job
        .settings
        .number("initial_layer_print_height")
        .unwrap_or(0.0);
    let bands = BodyBands::printed(layer_height.max(first_height));
    let mut print = body_print(&job.mesh, rotation, &bands);
    print.landing = landing(&job.mesh, &job.settings, job.build, rotation)?.loops;
    let grown = class_envelope(job, predictions, margins)?;
    print.support_base = grown.base;
    print.support = grown.above_base;
    print.interface = grown.interface;
    print.provenance.flavour = predictions
        .iter()
        .map(|(build, _)| build.name())
        .collect::<Vec<_>>()
        .join("+");
    print.provenance.members = predictions
        .iter()
        .map(|(_, prediction)| prediction.ensemble.graphs)
        .sum();
    print.provenance.margins = margins;
    print.provenance.cache_key = predictions
        .iter()
        .map(|(_, prediction)| prediction.cache_key.as_str())
        .collect::<Vec<_>>()
        .join("+");
    Ok(print)
}

/// The supports of one class: every build's ensemble grown by its margins, united. Growing is
/// most of a class on a model cache hit (2.4 s per P1S class at two threads), so with
/// `--cache-dir` the result is cached too, keyed by the ensembles and the margins.
fn class_envelope(
    job: &Job,
    predictions: &[(Flavour, Prediction)],
    margins: Margins,
) -> AppResult<Realization> {
    let fixed = job.args.support_margins.is_some();
    let first_layer_mm = |build: Flavour| {
        if fixed {
            margins.first_layer_mm
        } else {
            build.first_layer_margin_mm()
        }
    };
    let band_mm = |build: Flavour, band: usize| {
        if fixed {
            margins.band_mm
        } else {
            build.band_margin_mm(band)
        }
    };
    // Supports off: no ensemble key, nothing worth caching.
    let key = job
        .args
        .cache_dir
        .as_deref()
        .filter(|_| predictions.iter().all(|(_, p)| !p.cache_key.is_empty()))
        .map(|dir| {
            let parts: Vec<EnvelopePart> = predictions
                .iter()
                .map(|(build, prediction)| {
                    let bands = prediction
                        .ensemble
                        .realizations
                        .iter()
                        .map(|member| member.support.len())
                        .max()
                        .unwrap_or(0);
                    EnvelopePart {
                        ensemble_key: &prediction.cache_key,
                        first_layer_mm: first_layer_mm(*build),
                        band_mm: (0..bands).map(|band| band_mm(*build, band)).collect(),
                    }
                })
                .collect();
            (dir, envelope_key(&parts))
        });
    if let Some((dir, key)) = &key
        && let Some(cached) = load_envelope(dir, key)
    {
        return Ok(cached);
    }
    let envelopes: Vec<Realization> = predictions
        .iter()
        .map(|(build, prediction)| {
            let members: Vec<_> = prediction.ensemble.realizations.iter().collect();
            envelope_banded(&members, first_layer_mm(*build), &|band| {
                band_mm(*build, band)
            })
        })
        .collect();
    let grown = match envelopes.as_slice() {
        [single] => single.clone(),
        all => envelope(&all.iter().collect::<Vec<_>>(), Margins::default()),
    };
    if let Some((dir, key)) = &key {
        store_envelope(dir, key, &grown)?;
    }
    Ok(grown)
}

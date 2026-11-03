//! The modelling step without a slicer (`--support-source model`): per rotation class the
//! supports come from the support model's ensemble, grown by the flavour's calibrated margins,
//! the landing from the first-layer model and the prime tower from the tower model.

use goat_packer::{
    first_layer::landing,
    mesh_section::{BodyBands, body_print},
    object_print::{Margins, ObjectPrint},
    progress::{self, Stage},
    support_model::{
        envelope::envelope,
        predict::{PredictInput, Prediction, predict},
        settings::{ProjectConfig, SupportSettings},
    },
};

use crate::AppResult;
use crate::job::Job;
use crate::model::{ClassSummary, Modelled};
use crate::plan::plan_of;
use crate::tower::tower_spec;

/// Predicts every rotation class and builds the plan.
pub fn modelled_plate(job: &Job) -> AppResult<Modelled> {
    let config = ProjectConfig::load(&job.args.source_3mf)?;
    let settings = SupportSettings::from_config(&config)?;
    let margins = job.flavour.flavour.initial_margins();
    let mut prints = Vec::new();
    let mut summaries = Vec::new();
    let mut seconds = 0.0;
    for &rotation in &job.args.rotations {
        progress::stage(
            Stage::Modelling,
            &format!("predicting supports (rotation {rotation}°)..."),
            None,
            Some(1),
        );
        let prediction = predict(&PredictInput {
            mesh: &job.mesh,
            settings: &settings,
            flavour: job.flavour.flavour,
            rotation_deg: rotation,
            cache_dir: job.args.cache_dir.as_deref(),
        })?;
        println!(
            "support model, rotation {rotation}°: {} distinct realizations of {} members in {:.1} s{}",
            prediction.ensemble.realizations.len(),
            prediction.ensemble.members.len(),
            prediction.seconds,
            if prediction.cached { " (cache)" } else { "" }
        );
        seconds += prediction.seconds;
        prints.push(class_print(job, rotation, &prediction, margins)?);
        summaries.push(ClassSummary {
            rotation_deg: rotation,
            runs: prediction.ensemble.members.len(),
            saturated: true,
            cached: prediction.cached,
            growth_mm2: Vec::new(),
            landing_outside_model_mm2: 0.0,
        });
    }
    let tower = tower_spec(job, &[])?;
    Ok(Modelled {
        plan: plan_of(job, prints, tower),
        summaries,
        probe_seconds: 0.0,
        model_seconds: seconds,
    })
}

/// The print of one class: body kinds from the mesh, landing from the first-layer model, the
/// supports from the envelope of the ensemble.
fn class_print(
    job: &Job,
    rotation: f64,
    prediction: &Prediction,
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
    let members: Vec<_> = prediction.ensemble.realizations.iter().collect();
    let grown = envelope(&members, margins);
    print.support_base = grown.base;
    print.support = grown.above_base;
    print.interface = grown.interface;
    print.provenance.flavour = job.flavour.flavour.name().to_owned();
    print.provenance.members = prediction.ensemble.realizations.len();
    print.provenance.margins = margins;
    print.provenance.cache_key = prediction.cache_key.clone();
    Ok(print)
}

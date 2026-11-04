//! The modelling step: one probe per rotation class, turned into the class's print, and the
//! prime tower the probes and the target GUI's model agree on.

use goat_packer::support_model::SupportError;
use serde::{Deserialize, Serialize};

use crate::AppResult;
use crate::args::SupportSource;
use crate::class_print::class_print;
use crate::from_model::modelled_plate;
use crate::job::Job;
use crate::plan::{PackPlan, plan_of};
use crate::probe::{ClassStep, probe_class};
use crate::tower::tower_spec;

/// What one class's probe did, for the report.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClassSummary {
    pub rotation_deg: f64,
    /// Slices run (of the run that filled the cache, on a hit).
    pub runs: usize,
    /// The last runs added nothing; false = the run limit stopped the probe.
    pub saturated: bool,
    pub cached: bool,
    /// Area added to the union by each run, mm² (the largest band).
    pub growth_mm2: Vec<f64>,
    /// Printed first layer outside the first-layer model's landing, mm².
    pub landing_outside_model_mm2: f64,
}

pub struct Modelled {
    pub plan: PackPlan,
    /// Where the supports came from: `Probe` or `Model`, never `Auto`.
    pub source: SupportSource,
    pub summaries: Vec<ClassSummary>,
    /// Seconds of slicing the probes took (0 when cached): scales later progress estimates.
    pub probe_seconds: f64,
    /// Seconds the support model took (0 for probes; the cache's own time on a hit).
    pub model_seconds: f64,
}

/// Builds the plan with the supports the job asks for: the support model (no slicer call), or
/// probes of every rotation class in turn (one live slicer at a time). `Auto` probes only
/// when the model refuses the project; any other model error stays an error.
pub fn model_plate(job: &Job) -> AppResult<Modelled> {
    match job.args.support_source {
        SupportSource::Model => modelled_plate(job),
        SupportSource::Probe => probed_plate(job),
        SupportSource::Auto => match modelled_plate(job) {
            Err(error) if refused(&*error) => {
                // A plain line like the gap-floor warning; the report repeats it (`warnings`).
                eprintln!(
                    "warning: the support model does not cover this project ({error}); probing with the slicer"
                );
                probed_plate(job)
            }
            other => other,
        },
    }
}

/// The model refused the project's settings (it never guesses outside its specs).
fn refused(error: &(dyn std::error::Error + 'static)) -> bool {
    matches!(
        error.downcast_ref::<SupportError>(),
        Some(SupportError::Unsupported { .. })
    )
}

fn probed_plate(job: &Job) -> AppResult<Modelled> {
    let mut probes = Vec::new();
    let mut prints = Vec::new();
    let mut summaries = Vec::new();
    let count = job.args.rotations.len();
    for (index, &rotation) in job.args.rotations.iter().enumerate() {
        let probe = probe_class(job, rotation, ClassStep { index, count })?;
        let (print, check) = class_print(job, &probe, &probe.cache_key)?;
        summaries.push(ClassSummary {
            rotation_deg: rotation,
            runs: probe.runs,
            saturated: probe.saturated,
            cached: probe.cached,
            growth_mm2: probe.growth_mm2.clone(),
            landing_outside_model_mm2: check.probe_landing_outside_model_mm2,
        });
        prints.push(print);
        probes.push(probe);
    }
    let tower = tower_spec(job, &probes)?;
    let probe_seconds = probes
        .iter()
        .filter(|probe| !probe.cached && probe.runs > 0)
        .map(|probe| probe.seconds / probe.runs as f64)
        .fold(0.0, f64::max);
    Ok(Modelled {
        plan: plan_of(job, prints, tower),
        source: SupportSource::Probe,
        summaries,
        probe_seconds,
        model_seconds: 0.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_refusal_of_the_model_falls_back_to_probes() {
        let boxed = |error: SupportError| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) };
        assert!(refused(&*boxed(SupportError::unsupported("raft_layers", "2", "rafts"))));
        assert!(!refused(&*boxed(SupportError::Invalid("broken mesh".to_owned()))));
        let other: Box<dyn std::error::Error + Send + Sync> = "slicer missing".into();
        assert!(!refused(&*other));
    }
}

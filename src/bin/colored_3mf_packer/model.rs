//! The modelling step: one probe per rotation class, turned into the class's print, and the
//! prime tower the probes and the target GUI's model agree on.

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
    pub summaries: Vec<ClassSummary>,
    /// Seconds of slicing the probes took (0 when cached): scales later progress estimates.
    pub probe_seconds: f64,
    /// Seconds the support model took (0 for probes; the cache's own time on a hit).
    pub model_seconds: f64,
}

/// Builds the plan with the supports the job asks for: the support model (no slicer call), or
/// probes of every rotation class in turn (one live slicer at a time).
pub fn model_plate(job: &Job) -> AppResult<Modelled> {
    match job.args.support_source {
        SupportSource::Model => modelled_plate(job),
        SupportSource::Probe => probed_plate(job),
    }
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
        summaries,
        probe_seconds,
        model_seconds: 0.0,
    })
}

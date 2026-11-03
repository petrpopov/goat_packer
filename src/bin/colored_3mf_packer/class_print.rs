//! One rotation class as an `ObjectPrint`: the body kinds from the mesh, the landing from the
//! first-layer model and the probes' printed first layer, the supports from the probes grown
//! by the margins (design §4.2).

use goat_packer::{
    first_layer::landing,
    mesh_section::{BodyBands, body_print},
    object_print::{Margins, ObjectPrint},
};

use crate::AppResult;
use crate::job::Job;
use crate::loops_ops::{area_mm2, grown, union};
use crate::margins::{DELTA_BAND_MM, DELTA_FIRST_LAYER_MM};
use crate::probe::ClassProbe;

/// What building a print showed besides the print: the cross-check of the two landings.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrintCheck {
    /// Printed first-layer body outside the model landing, mm². Zero when the model covers
    /// what the slicer printed; the print holds the union of both either way.
    pub probe_landing_outside_model_mm2: f64,
}

/// The print of `probe`'s rotation class. `cache_key` is recorded in its provenance.
pub fn class_print(
    job: &Job,
    probe: &ClassProbe,
    cache_key: &str,
) -> AppResult<(ObjectPrint, PrintCheck)> {
    let layer_height = job
        .settings
        .number("layer_height")
        .ok_or("the project has no layer_height")?;
    let first_height = job.settings.number("initial_layer_print_height").unwrap_or(0.0);
    let bands = BodyBands::printed(layer_height.max(first_height));
    let mut print = body_print(&job.mesh, probe.rotation_deg, &bands);
    let (landing, check) = merged_landing(job, probe);
    print.landing = landing;
    print.support_base = grown(&probe.base, DELTA_FIRST_LAYER_MM);
    print.support = probe.above_base.iter().map(|band| grown(band, DELTA_BAND_MM)).collect();
    print.interface = probe.interface.iter().map(|band| grown(band, DELTA_BAND_MM)).collect();
    print.provenance.flavour = job.flavour.flavour.name().to_owned();
    print.provenance.members = probe.runs;
    print.provenance.margins = Margins {
        first_layer_mm: DELTA_FIRST_LAYER_MM,
        band_mm: DELTA_BAND_MM,
    };
    print.provenance.cache_key = cache_key.to_owned();
    Ok((print, check))
}

/// The landing the slicer printed, united with the first-layer model's: the model does not
/// depend on a slice, the probe on no assumption of ours. When the model refuses the
/// project's settings the probe stands alone.
fn merged_landing(job: &Job, probe: &ClassProbe) -> (goat_packer::object_print::Loops, PrintCheck) {
    match landing(&job.mesh, &job.settings, job.build, probe.rotation_deg) {
        Ok(model) => {
            let merged = union(&[&model.loops, &probe.landing]);
            let outside = area_mm2(&merged) - area_mm2(&model.loops);
            let check = PrintCheck {
                probe_landing_outside_model_mm2: outside.max(0.0),
            };
            (merged, check)
        }
        Err(refused) => {
            eprintln!("note: first-layer model skipped, the probes' printed first layer stands: {refused}");
            (union(&[&probe.landing]), PrintCheck::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_job;
    use goat_packer::Point;
    use goat_packer::object_print::Loops;

    fn square(half: f64) -> Loops {
        let p = |x: f64, y: f64| Point { x, y };
        vec![vec![p(-half, -half), p(half, -half), p(half, half), p(-half, half), p(-half, -half)]]
    }

    fn bounds(loops: &Loops) -> (f64, f64) {
        let xs = loops.iter().flatten().map(|p| p.x);
        xs.fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)))
    }

    #[test]
    fn the_print_holds_the_body_the_landing_and_the_supports_grown_by_the_margins() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let probe = ClassProbe {
            rotation_deg: 180.0,
            runs: 3,
            landing: square(5.0),
            base: square(8.0),
            above_base: vec![square(8.0), square(9.0)],
            interface: vec![Loops::new(), square(9.0)],
            ..ClassProbe::default()
        };
        let (print, _) = class_print(&job, &probe, "key").unwrap();
        assert_eq!(print.rotation_deg, 180.0);
        assert!(!print.silhouette.is_empty() && !print.body.is_empty() && !print.part_hulls.is_empty());
        let (lo, hi) = bounds(&print.support_base);
        assert!(lo <= -8.0 - DELTA_FIRST_LAYER_MM && hi >= 8.0 + DELTA_FIRST_LAYER_MM, "{lo} {hi}");
        assert!(hi < 8.0 + DELTA_FIRST_LAYER_MM + 0.2, "{hi}");
        let (lo, hi) = bounds(&print.support[1]);
        assert!(lo <= -9.0 - DELTA_BAND_MM && hi >= 9.0 + DELTA_BAND_MM, "{lo} {hi}");
        assert_eq!(print.support.len(), print.interface.len());
        // The landing is at least what the slicer printed.
        let (lo, hi) = bounds(&print.landing);
        assert!(lo <= -5.0 && hi >= 5.0);
        assert_eq!((print.provenance.members, print.provenance.cache_key.as_str()), (3, "key"));
        assert_eq!(print.provenance.margins.band_mm, DELTA_BAND_MM);
    }

    #[test]
    fn a_probe_without_supports_gives_a_print_without_support_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let probe = ClassProbe { landing: square(5.0), runs: 3, ..ClassProbe::default() };
        let (print, _) = class_print(&job, &probe, "").unwrap();
        assert!(print.support_base.is_empty() && print.support.is_empty());
    }
}

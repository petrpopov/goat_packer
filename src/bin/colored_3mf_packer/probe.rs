//! The support probe of one rotation class: the source object turned by the class angle,
//! alone at the bed centre (interior, so the slicer clips and bends nothing), sliced a few
//! times with the profile's slicer. What the slicer printed (supports, first-layer body,
//! prime tower) is read from the G-code and unioned over the runs: tree supports differ
//! between slices, so the union stands for the family, and the runs stop once more of them
//! add nothing (design §3.2, `ensemble` of the slicer-free plan, with real slices as members).

use std::{path::PathBuf, time::Instant};

use goat_packer::{
    PackedPlacement, Point,
    assemble::{AssembleOptions, assemble_project},
    gcode_collisions::{TowerLayer, tower_layers},
    nozzle_clean::inject_nozzle_clean,
    object_print::{Loops, Realization},
    print_time::TimeSample,
    progress::{self, Stage, StageDetail},
    realization_from_gcode,
};
use serde::{Deserialize, Serialize};

use crate::AppResult;
use crate::job::Job;
use crate::loops_ops::{area_mm2, union};
use crate::own_gap::apply_own_gap;
use crate::probe_cache::{cache_key, load, store};
use crate::slicing::slice_project;
use crate::time_window::{even_one_exceeds, hours, time_sample};

/// Fewest slices of a class: two runs agreeing says little about a family of realizations.
pub const MIN_RUNS: usize = 3;
/// Most slices of a class, saturated or not (a slice of one goat takes 10-25 s).
pub const MAX_RUNS: usize = 8;
/// A run adds nothing when it grows no band by this much, mm².
const SATURATION_MM2: f64 = 1.0;
/// Consecutive runs that must add nothing before the probe stops.
const QUIET_RUNS: usize = 2;

/// What the probes of one rotation class measured, in the class's plate-aligned frame (the
/// frame origin at 0, supports not rotated back).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ClassProbe {
    pub rotation_deg: f64,
    pub runs: usize,
    /// The last `QUIET_RUNS` runs added nothing (false: the run limit stopped it).
    pub saturated: bool,
    /// First-layer body the slicer printed (unioned over the runs).
    pub landing: Loops,
    pub base: Loops,
    /// Support of the layers above layer 0 per absolute band (`Realization::above_base`).
    pub above_base: Vec<Loops>,
    pub interface: Vec<Loops>,
    /// The tower layer by layer, plate coordinates, unioned over the runs.
    pub tower: Vec<TowerLayer>,
    /// The largest band growth each run caused, mm².
    pub growth_mm2: Vec<f64>,
    pub seconds: f64,
    /// The cache key the probe was stored under (recorded in the print's provenance).
    pub cache_key: String,
    /// Served from the cache: no slicer ran. Not stored.
    #[serde(skip)]
    pub cached: bool,
}

/// Where the probe object stands: the printable area's centre.
pub fn probe_centre(job: &Job) -> Point {
    let bed = job.profile.printable_bed();
    Point {
        x: (bed.min_x + bed.max_x) * 0.5,
        y: (bed.min_y + bed.max_y) * 0.5,
    }
}

/// Which class of the rotation list a probe is (0-based `index` of `count`), for the progress
/// events: the webapp shows one probe stage over all classes, its bar moving by slices.
#[derive(Clone, Copy, Debug)]
pub struct ClassStep {
    pub index: usize,
    pub count: usize,
}

impl ClassStep {
    /// The stage detail of slice `run` (1-based) of this class; `cached`: no slicer runs. The
    /// steps are counted as if every class took `MAX_RUNS`, so a saturated class jumps ahead.
    pub fn detail(self, rotation_deg: f64, run: usize, cached: bool) -> StageDetail {
        let step = if cached { (self.index + 1) * MAX_RUNS } else { self.index * MAX_RUNS + run - 1 };
        StageDetail::new(step, self.count * MAX_RUNS)
            .with("rotation_deg", rotation_deg)
            .with("class", self.index + 1)
            .with("classes", self.count)
            .with("run", run)
            .with("min_runs", MIN_RUNS)
            .with("max_runs", MAX_RUNS)
            .with("cached", cached)
    }
}

/// The probe of `rotation_deg`: from the cache when `--cache-dir` has it, else sliced.
pub fn probe_class(job: &Job, rotation_deg: f64, class: ClassStep) -> AppResult<ClassProbe> {
    let centre = probe_centre(job);
    let key = cache_key(job, rotation_deg, centre);
    if let Some(dir) = &job.args.cache_dir
        && let Some(mut hit) = load(dir, &key)
    {
        progress::stage_step(
            Stage::ProbeSlice,
            &format!("support probe of rotation {rotation_deg}°: cache hit ({} runs)", hit.runs),
            Some(1),
            &class.detail(rotation_deg, hit.runs, true),
        );
        hit.cached = true;
        return Ok(hit);
    }
    let probe = slice_class(job, rotation_deg, centre, &key, class)?;
    if let Some(dir) = &job.args.cache_dir {
        store(dir, &key, &probe)?;
    }
    Ok(probe)
}

fn slice_class(
    job: &Job,
    rotation_deg: f64,
    centre: Point,
    key: &str,
    class: ClassStep,
) -> AppResult<ClassProbe> {
    let project = write_copy(job, rotation_deg, centre)?;
    let started = Instant::now();
    let mut probe = ClassProbe {
        rotation_deg,
        cache_key: key.to_owned(),
        ..ClassProbe::default()
    };
    while probe.runs < MAX_RUNS && !is_saturated(&probe.growth_mm2) {
        let run = probe.runs + 1;
        // The first line keeps its wording: the webapp's text fallback (no JSON events) starts
        // the probe stage on "probing prime tower position".
        let message = if run == 1 {
            format!("probing prime tower position and supports (rotation {rotation_deg}°)...")
        } else {
            format!("probe slice {run} of at most {MAX_RUNS} (rotation {rotation_deg}°)...")
        };
        progress::stage_step(Stage::ProbeSlice, &message, Some(1), &class.detail(rotation_deg, run, false));
        let label = format!("probe_{}_{}", rotation_deg.to_string().replace('.', "_"), run);
        let slice = slice_project(&job.profile, &project, 1, &job.args.work_dir, &label)?;
        let realization = realization_from_gcode(&slice.gcode, None, (centre.x, centre.y), probe.runs as u32)
            .map_err(|error| format!("probe slice {label}: {error}"))?;
        let growth = probe.absorb(&realization, tower_layers(&slice.gcode));
        println!("{label}: union grew by {growth:.2} mm²");
    }
    probe.saturated = is_saturated(&probe.growth_mm2);
    probe.seconds = started.elapsed().as_secs_f64();
    Ok(probe)
}

/// Enough runs, and the last `QUIET_RUNS` of them added less than `SATURATION_MM2` each.
fn is_saturated(growth_mm2: &[f64]) -> bool {
    growth_mm2.len() >= MIN_RUNS
        && growth_mm2[growth_mm2.len() - QUIET_RUNS..]
            .iter()
            .all(|growth| *growth < SATURATION_MM2)
}

impl ClassProbe {
    /// Unions one run in; returns the most any band grew, mm² (the first run: its own area).
    fn absorb(&mut self, run: &Realization, tower: Vec<TowerLayer>) -> f64 {
        let mut growth = merge(&mut self.base, &run.base);
        let bands = run.above_base.len().max(self.above_base.len());
        self.above_base.resize(bands, Loops::new());
        self.interface.resize(bands, Loops::new());
        for band in 0..bands {
            let empty = Loops::new();
            let band_of = |bands: &'_ [Loops]| bands.get(band).unwrap_or(&empty).clone();
            growth = growth.max(merge(&mut self.above_base[band], &band_of(&run.above_base)));
            merge(&mut self.interface[band], &band_of(&run.interface));
        }
        merge(&mut self.landing, &run.landing);
        merge_tower(&mut self.tower, tower);
        self.runs += 1;
        self.growth_mm2.push(growth);
        growth
    }
}

/// `into` united with `new`; how much area that added, mm².
fn merge(into: &mut Loops, new: &Loops) -> f64 {
    let merged = union(&[into, new]);
    let growth = area_mm2(&merged) - area_mm2(into);
    *into = merged;
    growth
}

/// The tower of a run into the union: per layer, the union of the extents.
fn merge_tower(tower: &mut Vec<TowerLayer>, run: Vec<TowerLayer>) {
    for layer in run {
        match tower.iter_mut().find(|known| known.layer == layer.layer) {
            Some(known) => known.extent = known.extent.union(layer.extent),
            None => tower.push(layer),
        }
    }
    tower.sort_by_key(|layer| layer.layer);
}

/// A one-object copy of the source: turned by `rotation_deg` about its frame centre, which
/// stands at `centre`. Built by the same assembler as the final plate, so the probe differs
/// from a clone in nothing but its number.
fn write_copy(job: &Job, rotation_deg: f64, centre: Point) -> AppResult<PathBuf> {
    let path = job
        .args
        .work_dir
        .join(format!("probe_{}.3mf", rotation_deg.to_string().replace('.', "_")));
    let (min, max) = job.mesh.rotated(rotation_deg).normal_bounds();
    let corner = |x: f64, y: f64| Point { x: centre.x + x, y: centre.y + y };
    let outline = vec![
        corner(min[0], min[1]),
        corner(max[0], min[1]),
        corner(max[0], max[1]),
        corner(min[0], max[1]),
        corner(min[0], min[1]),
    ];
    let options = AssembleOptions {
        placements: vec![PackedPlacement {
            x_mm: centre.x,
            y_mm: centre.y,
            angle_deg: rotation_deg,
        }],
        stl_path: PathBuf::new(),
        edge_margin: 0.0,
        purge_lines: Vec::new(),
        keepouts: Vec::new(),
        duplicate_mesh_per_object: false,
        reuse_source_object_instances: false,
        inline_colored_mesh_objects: false,
        footprint_override: None,
        frame: Some(job.frame),
        world_polygons: Some(vec![vec![outline]]),
    };
    assemble_project(&job.args.source_3mf, &path, &options)
        .map_err(|error| format!("probe copy for rotation {rotation_deg}°: {error}"))?;
    // The probes must print the supports the output 3MF will (same forced gap).
    apply_own_gap(&path, job.own_gap)?;
    Ok(path)
}

/// The corrected time of one object, for a time window's first aim. Sliced from the first
/// class's probe copy with the same wipe injection as the final plate, so it is comparable
/// to the window slices; a single object that already overruns the window fails here, before
/// the packing.
pub fn time_probe(job: &Job, window_s: f64) -> AppResult<TimeSample> {
    let rotation = job.args.rotations[0];
    let mut project = write_copy(job, rotation, probe_centre(job))?;
    if let Some(spec) = &job.args.nozzle_clean {
        let injected = job.args.work_dir.join("probe_time.3mf");
        inject_nozzle_clean(&job.profile, &project, &injected, spec)
            .map_err(|error| format!("nozzle-clean injection failed: {error}"))?;
        project = injected;
    }
    progress::stage_step(
        Stage::ProbeSlice,
        "probing prime tower position for the time window (slicing 1 object)...",
        Some(1),
        &StageDetail::new(0, 1).with("purpose", "time"),
    );
    let slice = slice_project(&job.profile, &project, 1, &job.args.work_dir, "probe_time")?;
    let estimate = slice
        .estimate
        .map_err(|error| format!("probe slice time estimate unavailable: {error}"))?;
    let sample = time_sample(1, &estimate, job.slicer_change_s, &job.correction);
    if sample.corrected_s > window_s {
        return Err(even_one_exceeds(sample.corrected_s, window_s).into());
    }
    println!(
        "time window {:.1} h: 1 object = {:.1} h corrected ({} filament changes)",
        hours(window_s),
        hours(sample.corrected_s),
        sample.changes
    );
    Ok(sample)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_stops_after_two_quiet_runs_but_never_before_the_third() {
        assert!(!is_saturated(&[]));
        assert!(!is_saturated(&[0.0, 0.0]));
        assert!(is_saturated(&[50.0, 0.2, 0.9]));
        assert!(!is_saturated(&[50.0, 12.0, 0.9]));
        assert!(!is_saturated(&[50.0, 0.5, 4.0]));
        assert!(is_saturated(&[50.0, 8.0, 3.0, 0.1, 0.0]));
    }

    #[test]
    fn a_run_of_new_geometry_grows_the_union_and_a_repeat_adds_nothing() {
        let square = |x: f64| {
            let p = |dx: f64, dy: f64| Point { x: x + dx, y: dy };
            vec![vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0), p(0.0, 0.0)]]
        };
        let run = |x: f64| Realization {
            base: square(x),
            above_base: vec![square(x)],
            ..Realization::default()
        };
        let mut probe = ClassProbe::default();
        assert!((probe.absorb(&run(0.0), Vec::new()) - 100.0).abs() < 1e-6);
        assert!(probe.absorb(&run(0.0), Vec::new()).abs() < 1e-6);
        assert!((probe.absorb(&run(5.0), Vec::new()) - 50.0).abs() < 1e-6);
        assert_eq!(probe.runs, 3);
        assert_eq!(probe.growth_mm2.len(), 3);
    }
}

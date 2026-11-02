//! The probe: one object sliced from the source project, and what it shows — the prime tower,
//! the time of a single object and the footprint the object really prints.

use goat_packer::{
    Footprint,
    colored_packer::PrimeTowerBounds,
    first_layer_footprint_of,
    nozzle_clean::inject_nozzle_clean,
    patch_overshoot_mm, patch_within_silhouette,
    print_time::TimeSample,
    progress::{self, Stage},
    support_interface_loops_of, union_loops,
};

use crate::AppResult;
use crate::job::Job;
use crate::slicing::{SliceOutput, slice_runs};
use crate::time_window::{even_one_exceeds, hours, time_sample};
use crate::tower::probe_tower_of;

/// Supports and brim legitimately add a few mm, but a patch far larger than the silhouette
/// means foreign extrusions (prime tower, purge line) leaked into the parsed first layer —
/// every clone would reserve them and the layout would come out sparse with no other error.
/// This caught the Bambu dialect bug where the tower inflated a 54 mm goat to a 97.5 mm patch.
const FOREIGN_EXTRUSION_EXCESS_MM: f64 = 15.0;

/// A patch that stays inside the mesh outline plus this much carries no supports or brim
/// beyond the model. The tolerance absorbs wall-bead artifacts (miter on sharp tips ~1 mm;
/// arachne slivers along a curved belly up to ~1.6 mm, 2.45 mm² total on the P1S goat); real
/// supports wander 3-5 mm past the outline with hundreds of mm².
const SILHOUETTE_TOLERANCE_MM: f64 = 2.0;

/// The probe slices (several runs, see `slicing::slice_runs`) and what they show.
pub struct Probe {
    pub runs: Vec<SliceOutput>,
    /// How long the slicing took: scales the expected duration of every later slice.
    pub seconds: f64,
    pub tower: Option<PrimeTowerBounds>,
    /// The corrected time of one object, when a time window needs it.
    pub sample: Option<TimeSample>,
}

impl Probe {
    fn gcodes(&self) -> Vec<&[u8]> {
        self.runs.iter().map(|run| run.gcode.as_slice()).collect()
    }
}

/// Slices one object of the source project and reads the prime tower and the time off it.
pub fn probe_source(job: &Job) -> AppResult<Probe> {
    let args = &job.args;
    // With --nozzle-clean the probe slices a copy carrying the same wipe injection as every
    // final plate, so its time (and tower) is comparable to theirs; the wipe block is gated on
    // `layer_num > 0`, so the first-layer patch parsed later is unaffected. The probe still
    // prints the tower at the source position (the relocation is decided from this very
    // slice); the window's shrink step uses the window-vs-max secant, which cancels that.
    let probe_project = match &args.nozzle_clean {
        Some(spec) => {
            let copy = args.work_dir.join("probe_source.3mf");
            inject_nozzle_clean(&job.profile, &args.source_3mf, &copy, spec)
                .map_err(|error| format!("nozzle-clean injection failed: {error}"))?;
            copy
        }
        None => args.source_3mf.clone(),
    };
    progress::stage(
        Stage::ProbeSlice,
        "probing prime tower position (slicing 1 object)...",
        None,
        Some(1),
    );
    let started = std::time::Instant::now();
    let runs = slice_runs(&job.profile, &probe_project, 1, &args.work_dir, "probe")?;
    let seconds = started.elapsed().as_secs_f64();
    let sample = single_object_sample(job, &runs[0])?;
    let gcodes: Vec<&[u8]> = runs.iter().map(|run| run.gcode.as_slice()).collect();
    let tower = probe_tower_of(&gcodes, &job.profile, args.clearance);
    match tower {
        Some(tower) => println!(
            "probe tower keepout: {:.2},{:.2} → {:.2},{:.2}",
            tower.keepout.min_x, tower.keepout.min_y, tower.keepout.max_x, tower.keepout.max_y
        ),
        None => println!(
            "no prime/wipe tower markers in source slice (single material/color); packing \
             without a tower keepout"
        ),
    }
    Ok(Probe {
        runs,
        seconds,
        tower,
        sample,
    })
}

/// Spec step 1: with a window, a single object that already overruns it fails here, before
/// the expensive packing.
fn single_object_sample(job: &Job, probe: &SliceOutput) -> AppResult<Option<TimeSample>> {
    let Some(window_s) = job.window_s else {
        return Ok(None);
    };
    let estimate = probe
        .estimate
        .as_ref()
        .map_err(|error| format!("probe slice time estimate unavailable: {error}"))?;
    let sample = time_sample(1, estimate, job.slicer_change_s, &job.correction);
    if sample.corrected_s > window_s {
        return Err(even_one_exceeds(sample.corrected_s, window_s).into());
    }
    println!(
        "time window {:.1} h: 1 object = {:.1} h corrected ({} filament changes)",
        hours(window_s),
        hours(sample.corrected_s),
        sample.changes
    );
    Ok(Some(sample))
}

/// The footprint the packer works with, and how it constrains the layout.
pub struct ProbeFootprint {
    pub footprint: Footprint,
    pub lock_orientation: bool,
    /// How far the first layer overshoots the silhouette at thin tips: widens the purge-line
    /// keepout only.
    pub purge_tip_overshoot: f64,
}

/// The probe slice already shows the object exactly as it prints — body, tree supports,
/// brim. Packing against that first-layer patch stops supports from touching neighbouring
/// bodies, which the pure mesh projection could not promise (tree supports wander several
/// mm past the silhouette on a real lying-animal plate). `model_center` is where the model's
/// bbox center sits in the probe slice: the layer-1 patch must be centered exactly there to
/// stay in the assembler's placement frame.
pub fn footprint_from_probe(
    job: &Job,
    probe: &Probe,
    silhouette: &Footprint,
    model_center: Option<(f64, f64)>,
) -> AppResult<ProbeFootprint> {
    let gcodes = probe.gcodes();
    let center =
        model_center.ok_or("cannot establish probe geometry frame; refusing unverified packing")?;
    let patch = first_layer_footprint_of(&gcodes, Some(center))
        .ok_or("probe has no object extrusions; refusing mesh-only fallback")?;
    report_patch(&patch, silhouette, &probe.runs[0].gcode);
    // First-layer containment does NOT prove absence of upper supports. Learn every height
    // before choosing rotation freedom, even when the optional multi-object sample is off.
    let placement = goat_packer::PackedPlacement {
        x_mm: center.0,
        y_mm: center.1,
        angle_deg: 0.0,
    };
    let (zones, _) =
        goat_packer::gcode_collisions::layer_zones_from_sample(&gcodes, &patch, &[placement])
            .map_err(|e| format!("probe support geometry: {e}"))?;
    let has_supports = zones.iter().any(|z| !z.support_loops.is_empty());
    let resolution = job.args.resolution;
    let mut found = if !has_supports
        && patch_within_silhouette(&patch, silhouette, resolution, SILHOUETTE_TOLERANCE_MM)
    {
        silhouette_footprint(&patch, silhouette, resolution)
    } else {
        patch_footprint(&patch, silhouette, &gcodes, Some(center), resolution)
    };
    found.footprint.merge_layer_zones(&zones);
    Ok(found)
}

fn report_patch(patch: &Footprint, silhouette: &Footprint, probe_gcode: &[u8]) {
    eprintln!(
        "probe slice's first layer: patch {:.1}x{:.1} mm, mesh silhouette {:.1}x{:.1} mm",
        patch.width, patch.height, silhouette.width, silhouette.height
    );
    // Arc-fitted probes parse differently from straight-line ones (the patch is rebuilt from
    // flattened G2/G3 sweeps); the count tells the operator which kind of gcode the patch
    // below came from when a layout surprises them.
    let arc_moves = probe_gcode
        .split(|&byte| byte == b'\n')
        .filter(|line| line.starts_with(b"G2 ") || line.starts_with(b"G3 "))
        .count();
    if arc_moves > 0 {
        eprintln!("probe gcode carries {arc_moves} G2/G3 arc moves; the patch flattens them");
    }
    if patch.width > silhouette.width + FOREIGN_EXTRUSION_EXCESS_MM
        || patch.height > silhouette.height + FOREIGN_EXTRUSION_EXCESS_MM
    {
        eprintln!(
            "WARNING: first-layer patch {:.1}x{:.1} mm exceeds the mesh silhouette \
             {:.1}x{:.1} mm by more than {FOREIGN_EXTRUSION_EXCESS_MM} mm — foreign \
             extrusions (prime tower / purge line) probably leaked into the patch; \
             expecting a sparse layout",
            patch.width, patch.height, silhouette.width, silhouette.height
        );
    }
}

/// The patch stays inside the silhouette: pack the silhouette, rotations allowed.
fn silhouette_footprint(
    patch: &Footprint,
    silhouette: &Footprint,
    resolution: f64,
) -> ProbeFootprint {
    eprintln!(
        "patch fits inside the mesh silhouette (+{SILHOUETTE_TOLERANCE_MM:.1} mm) — no supports \
         or brim reach past the model; packing the silhouette with rotations allowed"
    );
    // Within tolerance is not within: Arachne thin-tip lines print past the mesh outline
    // (measured up to 1.66 mm on a P1S goat's legs), and the prime line is machine start gcode
    // — nothing routes it around objects, so a goat whose silhouette cleared the purge-line
    // keepout printed its leg tips straight through the line. The measured overshoot widens
    // the PURGE-LINE keepout only: the tower and exclusion corner keep their own validated
    // safeties, and object-vs-object clearance is untouched.
    let overshoot = patch_overshoot_mm(patch, silhouette, resolution);
    if overshoot > 0.0 {
        eprintln!(
            "first layer overshoots the mesh silhouette by up to {overshoot:.1} mm at thin \
             tips; the purge-line keepout keeps that standoff"
        );
    }
    // Bed bounds still measure the printed patch: the overshoot at thin tips is real ink past
    // the silhouette, and a silhouette that clears the bed edge can print outside it (see the
    // bed_loops note in `prepare::prepare_plate`).
    let mut footprint = silhouette.clone();
    // Measured wall/bead overshoot also matters between objects, not only at bed edges.
    footprint.loops = union_loops(patch, silhouette);
    footprint.body_loops = footprint.loops.clone();
    footprint.bed_loops = Some(footprint.loops.clone());
    ProbeFootprint {
        footprint,
        lock_orientation: false,
        purge_tip_overshoot: overshoot,
    }
}

/// The patch reaches past the silhouette (supports/brim): pack patch ∪ silhouette with the
/// orientation locked to the sliced one.
fn patch_footprint(
    patch: &Footprint,
    silhouette: &Footprint,
    gcodes: &[&[u8]],
    model_center: Option<(f64, f64)>,
    resolution: f64,
) -> ProbeFootprint {
    // How far the patch actually sticks out separates the two realities this branch exists
    // for: real supports/brim wander whole millimetres, while a fraction of a millimetre is a
    // parse artifact (a stray chord, a mitered tip) worth a second look before accepting the
    // locked-orientation layout.
    let overshoot = patch_overshoot_mm(patch, silhouette, resolution);
    eprintln!(
        "patch reaches past the mesh silhouette by up to {overshoot:.1} mm \
         (supports/brim) — packing the patch ∪ silhouette, orientation locked to \
         the sliced one"
    );
    // Collisions must respect BOTH truths: the printed patch (supports/brim beyond the
    // outline) AND the mesh body where layer 1 prints nothing (a lying animal's raised horns;
    // on P1S the patch is inset ~0.4 mm per side, so a bare-patch layout glued 24 goat pairs
    // by up to 1.7 mm²). The body zone is the mesh outline — walls and walls' strips are all
    // inset representations of the same body — and silhouettes keep clearance from each
    // other. A neighbour's supports only must not land on what this object prints on the bed:
    // its first-layer body (`support_landing_loops`; see the field for the 40-goat plate that
    // proved the silhouette rule too strict).
    let mut combined = patch.clone();
    combined.loops = union_loops(patch, silhouette);
    combined.body_loops = silhouette.loops.clone();
    // Support interfaces print up to ~2 mm past the overhangs they carry, at the overhang's
    // height: they belong to this object's 3D extent, so they join the silhouette that keeps
    // clearance from the neighbours' silhouettes (four P1S goat pairs touched a neighbour's
    // wall at z ≈ 23 mm without this, and the strict-rule fallback cost the plate 4 goats).
    if let Some(center) = model_center {
        let interfaces = support_interface_loops_of(gcodes, center);
        if !interfaces.is_empty() {
            let mut interface_zone = silhouette.clone();
            interface_zone.loops = interfaces;
            let overshoot = patch_overshoot_mm(&interface_zone, silhouette, resolution);
            eprintln!(
                "support interfaces reach past the mesh silhouette by up to \
                 {overshoot:.1} mm; the silhouette zone includes them"
            );
            combined.body_loops = union_loops(silhouette, &interface_zone);
        }
    }
    combined.support_landing_loops = patch.body_loops.clone();
    // The patch is the measured print (tips included), so purge checks run against its real
    // extent already — no extra overshoot.
    ProbeFootprint {
        footprint: combined,
        lock_orientation: true,
        purge_tip_overshoot: 0.0,
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    use crate::test_support::test_job;

    #[test]
    fn unreadable_probe_does_not_silently_fall_back_to_mesh_packing() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let silhouette = goat_packer::extract_footprint(&job.footprint_stl).unwrap();
        let probe = Probe {
            runs: vec![SliceOutput {
                gcode: vec![],
                estimate: Err("empty".into()),
            }],
            seconds: 0.0,
            tower: None,
            sample: None,
        };
        assert!(footprint_from_probe(&job, &probe, &silhouette, Some((5.0, 5.0))).is_err());
        assert!(footprint_from_probe(&job, &probe, &silhouette, None).is_err());
    }
}

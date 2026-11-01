//! Writing the packed 3MF.

use std::fs;

use goat_packer::{
    Footprint, PackResult, Rect,
    assemble::{AssembleOptions, assemble_project},
    nozzle_clean::inject_nozzle_clean,
    plate_centre::relocate_wipe_tower_in_3mf,
};

use crate::AppResult;
use crate::job::Job;

pub fn build_colored_project(
    job: &Job,
    footprint: &Footprint,
    packed: &PackResult,
    keepouts: &[Rect],
    tower_shift: Option<(f64, f64)>,
) -> AppResult<()> {
    let args = &job.args;
    let profile = &job.profile;
    let placements = args.work_dir.join("placements.json");
    fs::write(&placements, serde_json::to_vec_pretty(packed)?)?;
    // Deliberately NOT reuse_source_object_instances: that mode (one shared <object>, N build
    // items) reliably reproduces BambuStudio's real CLI "gcode path conflicts found between
    // goat3.obj and goat3.obj" / "found slicing result conflict" error on a real ~20-object dense
    // plate, even though the exact same layout slices cleanly in the default mode used here (N
    // distinct <object> resources sharing one external mesh component, the same pattern already
    // proven for the uncolored Goat 4 packer). Paint attributes live in the untouched shared mesh
    // component either way, so this mode is equally paint-safe.
    let options = AssembleOptions {
        placements: packed.placements.clone(),
        stl_path: job.footprint_stl.clone(),
        edge_margin: profile.edge_margin_mm,
        purge_lines: profile.purge_lines.clone(),
        keepouts: keepouts.to_vec(),
        duplicate_mesh_per_object: false,
        reuse_source_object_instances: false,
        inline_colored_mesh_objects: false,
        footprint_override: Some(footprint.clone()),
    };
    assemble_project(&args.source_3mf, &args.output, &options)
        .map_err(|error| format!("3MF assembly failed: {error}"))?;
    // The one documented exception to "project_settings.config is copied byte-for-byte": a
    // centre-out window plate moves the tower next to its objects, and the tower position IS a
    // project setting (`wipe_tower_x/y[0]`). Applied to the output, so the CLI copies sliced
    // below and the written project carry the same tower the keepout reserved. Each build
    // re-assembles from the source, so the shift never accumulates.
    if let Some(shift) = tower_shift {
        let (x, y) = relocate_wipe_tower_in_3mf(&args.output, shift)
            .map_err(|error| format!("wipe tower relocation failed: {error}"))?;
        eprintln!("wipe tower relocated: wipe_tower_x/y[0] = {x:.3}, {y:.3}");
    }
    // Applied after assembly (both here and on the single repack path): the CLI copies sliced
    // for final/window checks derive from this same file, so their timings and tower state
    // reflect the real, wipe-carrying G-code (the probe gets its own injected copy in
    // `probe::probe_source`).
    if let Some(spec) = &args.nozzle_clean {
        inject_nozzle_clean(profile, &args.output, &args.output, spec)
            .map_err(|error| format!("nozzle-clean injection failed: {error}"))?;
        // Loud on purpose: an invisible injection once read as "the checkbox does nothing" —
        // the job log must state plainly that the wipe macro reached the output file.
        eprintln!(
            "nozzle clean injected into layer_change_gcode ({}) of {}",
            spec.describe(),
            args.output.display()
        );
    }
    Ok(())
}

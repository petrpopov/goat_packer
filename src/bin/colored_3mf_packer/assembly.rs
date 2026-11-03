//! Writing the packed 3MF.

use std::{fs, path::PathBuf};

use goat_packer::{
    PackResult, PackedPlacement, Point, PrintPlacement, Rect,
    assemble::{AssembleOptions, assemble_project},
    nozzle_clean::inject_nozzle_clean,
    object_print::ObjectPrint,
    plate_centre::relocate_wipe_tower_in_3mf,
};

use crate::AppResult;
use crate::gaps::check_output;
use crate::job::Job;
use crate::plan::PackPlan;
use crate::source::derive_embedded_stl;

/// Writes the plate: a clone of the source object per placement, turned by its class
/// rotation (the clone's frame origin lands on the placement), and the wipe tower moved to
/// `anchor`.
pub fn build_colored_project(
    job: &Job,
    plan: &PackPlan,
    placements: &[PrintPlacement],
    anchor: Option<Point>,
) -> AppResult<()> {
    let args = &job.args;
    let profile = &job.profile;
    write_placements(job, plan, placements)?;
    derive_embedded_stl(&args.source_3mf, &args.work_dir.join("embedded_source.stl"))?;
    let shifted = |line: &[f64; 5]| {
        let (dx, dy) = job.gcode_offset;
        [line[0] + dx, line[1] + dy, line[2] + dx, line[3] + dy, line[4]]
    };
    // Deliberately NOT reuse_source_object_instances: that mode (one shared <object>, N build
    // items) reliably reproduces BambuStudio's real CLI "gcode path conflicts found between
    // goat3.obj and goat3.obj" error on a real ~20-object dense plate, even though the same
    // layout slices cleanly in the default mode used here (N distinct <object> resources
    // sharing one external mesh component). Paint lives in the untouched shared mesh either
    // way. The shared mesh also keeps the supports one set per class (design §9.7).
    let options = AssembleOptions {
        placements: placements
            .iter()
            .map(|placement| PackedPlacement {
                x_mm: placement.x_mm,
                y_mm: placement.y_mm,
                angle_deg: plan.prints[placement.print].rotation_deg,
            })
            .collect(),
        stl_path: PathBuf::new(),
        edge_margin: profile.edge_margin_mm,
        purge_lines: profile.purge_lines.iter().map(shifted).collect(),
        keepouts: tower_rect(plan, anchor).into_iter().collect(),
        duplicate_mesh_per_object: false,
        reuse_source_object_instances: false,
        inline_colored_mesh_objects: false,
        footprint_override: None,
        frame: Some(job.frame),
        world_polygons: Some(world_polygons(&plan.prints, placements)),
    };
    assemble_project(&args.source_3mf, &args.output, &options)
        .map_err(|error| format!("3MF assembly failed: {error}"))?;
    // The one documented exception to "project_settings.config is copied byte-for-byte": the
    // tower position IS a project setting (`wipe_tower_x/y[0]`), and the packer chose it.
    // Applied to the output, so a slice of it prints the tower the layout reserved. Each build
    // re-assembles from the source, so the shift never accumulates.
    if let (Some(anchor), Some(stored)) = (anchor, job.tower_anchor) {
        let shift = (anchor.x - stored.0, anchor.y - stored.1);
        if shift != (0.0, 0.0) {
            let (x, y) = relocate_wipe_tower_in_3mf(&args.output, shift)
                .map_err(|error| format!("wipe tower relocation failed: {error}"))?;
            eprintln!("wipe tower placed: wipe_tower_x/y[0] = {x:.3}, {y:.3}");
        }
    }
    // Applied after assembly: every later slice derives from this file, so its timings and
    // tower reflect the real, wipe-carrying G-code.
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
    // The settings were rewritten above (tower, nozzle clean); the slice must still print
    // supports the way the probes did.
    check_output(&args.output, job.own_gap)
}

/// `placements.json` in the legacy `PackResult` shape that `verify_outputs`, the webapp and
/// the catalog qualifier read: where each copy's frame origin goes and the class rotation.
fn write_placements(job: &Job, plan: &PackPlan, placements: &[PrintPlacement]) -> AppResult<()> {
    let args = &job.args;
    let result = PackResult {
        placements: placements
            .iter()
            .map(|placement| PackedPlacement {
                x_mm: placement.x_mm,
                y_mm: placement.y_mm,
                angle_deg: plan.prints[placement.print].rotation_deg,
            })
            .collect(),
        resolution_mm: args.resolution,
        requested_clearance_mm: args.clearance,
        angle_step_deg: args.angle_step,
        attempts: args.attempts,
        attempts_run: 1,
        seed: args.seed,
    };
    fs::write(args.work_dir.join("placements.json"), serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}

/// What each copy reserves, plate coordinates: the silhouette and the first-layer support,
/// for the previews and the plate JSON.
fn world_polygons(prints: &[ObjectPrint], placements: &[PrintPlacement]) -> Vec<Vec<Vec<Point>>> {
    placements
        .iter()
        .map(|placement| {
            let print = &prints[placement.print];
            print
                .silhouette
                .iter()
                .chain(&print.support_base)
                .map(|ring| {
                    ring.iter()
                        .map(|p| Point {
                            x: p.x + placement.x_mm,
                            y: p.y + placement.y_mm,
                        })
                        .collect()
                })
                .collect()
        })
        .collect()
}

/// The box of everything the tower prints at `anchor`.
pub fn tower_rect(plan: &PackPlan, anchor: Option<Point>) -> Option<Rect> {
    let (spec, anchor) = (plan.tower.as_ref()?, anchor?);
    let reach = spec.chamfer.iter().map(|ring| ring[1]).fold(spec.brim_l0_mm, f64::max);
    Some(spec.body.grown(reach).translated(anchor.x, anchor.y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fixture_job, test_plan};
    use crate::gaps::OwnGap;
    use goat_packer::embedded_mesh::load_object_mesh;

    #[test]
    fn the_output_carries_the_gaps_the_probes_used_and_a_mismatch_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut job = fixture_job(dir.path());
        let plan = test_plan(&job, 1);
        let placements = [PrintPlacement { print: 0, x_mm: 100.0, y_mm: 120.0 }];
        // The project's own values (the fixture has none: the slicer defaults) survive the
        // assembly and the tower rewrite; build_colored_project checks them itself.
        build_colored_project(&job, &plan, &placements, Some(Point { x: 50.0, y: 50.0 })).unwrap();
        check_output(&job.args.output, job.own_gap).unwrap();
        // Probes that sliced with other values than the file carries: refused, naming both.
        let error = check_output(&job.args.output, OwnGap { first_layer_mm: 0.75, xy_mm: 0.35 })
            .unwrap_err()
            .to_string();
        assert!(error.contains("support_object_first_layer_gap") && error.contains("0.2"), "{error}");
        let error = check_output(&job.args.output, OwnGap { first_layer_mm: 0.2, xy_mm: 0.75 })
            .unwrap_err()
            .to_string();
        assert!(error.contains("support_object_xy_distance"), "{error}");
        // A file whose settings drifted after the probes: the assembly itself fails.
        job.own_gap.xy_mm = 0.75;
        let error = build_colored_project(&job, &plan, &placements, None).unwrap_err().to_string();
        assert!(error.contains("support_object_xy_distance"), "{error}");
    }

    /// A clone of each rotation class stands with its frame centre on the placement, whatever
    /// translations the source's component and build item carry (the fixture has both), and is
    /// turned by the class angle about that centre.
    #[test]
    fn a_clone_of_each_class_lands_its_frame_centre_on_the_placement() {
        let dir = tempfile::tempdir().unwrap();
        let job = fixture_job(dir.path());
        let plan = test_plan(&job, 2);
        for (class, (x, y)) in [(0, (100.0, 120.0)), (1, (80.0, 60.0))] {
            let placements = [PrintPlacement { print: class, x_mm: x, y_mm: y }];
            build_colored_project(&job, &plan, &placements, None).unwrap();
            let built = load_object_mesh(&job.args.output).unwrap();
            assert!((built.frame_origin[0] - x).abs() < 1e-6, "{:?}", built.frame_origin);
            assert!((built.frame_origin[1] - y).abs() < 1e-6, "{:?}", built.frame_origin);
            // The same object, turned: its extent in the frame is the rotated source's.
            let turned = job.mesh.rotated(plan.prints[class].rotation_deg).normal_bounds();
            let (min, max) = built.normal_bounds();
            for axis in 0..3 {
                assert!((min[axis] - turned.0[axis]).abs() < 1e-6, "{axis} {min:?} {turned:?}");
                assert!((max[axis] - turned.1[axis]).abs() < 1e-6, "{axis} {max:?} {turned:?}");
            }
        }
        let legacy: PackResult =
            serde_json::from_slice(&fs::read(job.args.work_dir.join("placements.json")).unwrap())
                .unwrap();
        assert_eq!(legacy.placements.len(), 1);
        assert!(job.args.work_dir.join("embedded_source.stl").is_file());
    }
}

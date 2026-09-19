//! Sweeps pack seeds WITHOUT slicing: reuses an existing probe slice's artifacts (the
//! embedded STL and the probe gcode from a previous full run's work dir) to answer "which
//! seed packs N objects under the current footprint rules". The layout is a lottery over
//! the seed — a rule change that honestly costs ~1 mm somewhere can flip the best-found
//! count for one seed while another seed finds the old count, and re-running the full
//! pipeline per seed is minutes of slicing per try.
//!
//! Usage:
//! ```text
//! cargo run --release --example seed_sweep -- \
//!   --source goat3_v4.3mf \
//!   --profile profiles/p1s.json \
//!   --probe-gcode work/probe_slice/plate_1.gcode \
//!   --tower-keepout 225.85,200.10,246.15,252.15 \
//!   --seeds 20270831+32 --clearance 0.7 --resolution 0.25 \
//!   --attempts 4800 --candidates 2400 --stall 1200
//! ```

use std::{fs, path::PathBuf, process::exit};

use goat_packer::{
    PackOptions, Rect, extract_footprint, first_layer_footprint, grid_layout, pack_footprint,
    patch_overshoot_mm, patch_within_silhouette,
    colored_packer::load_printer_profile,
    embedded_mesh::{embedded_mesh_triangles, stl_center_3d, write_binary_stl},
};
use rayon::prelude::*;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut source: Option<PathBuf> = None;
    let mut profile_path: Option<PathBuf> = None;
    let mut probe_gcode: Option<PathBuf> = None;
    let mut tower_keepout: Option<Rect> = None;
    let mut first_seed: u64 = 20_270_831;
    let mut seed_count: u64 = 16;
    let mut clearance = 0.7f64;
    let mut resolution = 0.25f64;
    let mut attempts = 4_800usize;
    let mut candidates = 2_400usize;
    let mut stall = 1_200usize;
    let mut angle_step = 2.0f64;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .unwrap_or_else(|| panic!("missing value after {flag}"))
        };
        match flag.as_str() {
            "--source" => source = Some(value().into()),
            "--profile" => profile_path = Some(value().into()),
            "--probe-gcode" => probe_gcode = Some(value().into()),
            "--tower-keepout" => {
                let raw = value();
                let values: Vec<f64> = raw
                    .split(',')
                    .map(|value| value.parse().expect("tower keepout numbers"))
                    .collect();
                assert_eq!(values.len(), 4, "tower keepout needs x0,y0,x1,y1");
                tower_keepout = Some(Rect::new(values[0], values[1], values[2], values[3]));
            }
            "--seeds" => {
                let raw = value();
                if let Some((first, count)) = raw.split_once('+') {
                    first_seed = first.parse().expect("seed start");
                    seed_count = count.parse().expect("seed count");
                } else {
                    first_seed = raw.parse().expect("seed");
                    seed_count = 1;
                }
            }
            "--clearance" => clearance = value().parse().expect("clearance"),
            "--resolution" => resolution = value().parse().expect("resolution"),
            "--attempts" => attempts = value().parse().expect("attempts"),
            "--candidates" => candidates = value().parse().expect("candidates"),
            "--stall" => stall = value().parse().expect("stall"),
            "--angle-step" => angle_step = value().parse().expect("angle step"),
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    let source = source.ok_or("--source is required")?;
    let profile_path = profile_path.ok_or("--profile is required")?;
    let probe_gcode = probe_gcode.ok_or("--probe-gcode is required")?;
    let profile = load_printer_profile(&profile_path).map_err(|e| e.to_string())?;

    // Same footprint derivation as the packer binary's fits-inside branch.
    let footprint_stl = std::env::temp_dir().join("seed_sweep_footprint.stl");
    let triangles = embedded_mesh_triangles(&source).map_err(|e| e.to_string())?;
    write_binary_stl(&footprint_stl, &triangles).map_err(|e| e.to_string())?;
    let silhouette = extract_footprint(&footprint_stl).map_err(|e| e.to_string())?;
    let model_center = model_center_on_bed(&source, &footprint_stl);
    let gcode = fs::read(&probe_gcode).map_err(|e| format!("read {}: {e}", probe_gcode.display()))?;
    let patch = first_layer_footprint(&gcode, model_center)
        .ok_or("probe gcode holds no first-layer patch")?;
    eprintln!(
        "patch {:.1}x{:.1} vs silhouette {:.1}x{:.1}; containment(+2.0): {}",
        patch.width,
        patch.height,
        silhouette.width,
        silhouette.height,
        patch_within_silhouette(&patch, &silhouette, resolution, 2.0)
    );
    let mut exclusions;
    if patch_within_silhouette(&patch, &silhouette, resolution, 2.0) {
        let overshoot = patch_overshoot_mm(&patch, &silhouette, resolution);
        eprintln!("rotations branch, purge tip overshoot = {overshoot:.2} mm");
        exclusions = profile.fixed_keepouts(overshoot).map_err(|e| e.to_string())?;
    } else {
        eprintln!("patch-locked branch; sweep not implemented for it (rare for plates)");
        return Ok(());
    }
    let mut footprint = silhouette.clone();
    if let Some(tower) = tower_keepout {
        exclusions.push(tower);
    }

    let results: Vec<(u64, usize)> = (first_seed..first_seed + seed_count)
        .into_par_iter()
        .map(|seed| {
            let options = PackOptions {
                bed: profile.usable_bed().expect("usable bed"),
                exclusions: exclusions.clone(),
                resolution,
                clearance,
                angle_step_deg: angle_step,
                attempts,
                candidates_per_step: candidates,
                stall_attempts: stall,
                max_objects: 0,
                lock_orientation: false,
                seed,
            };
            let grid = grid_layout(&footprint, &options).placements.len();
            let random = pack_footprint(&footprint, &options).placements.len();
            (seed, grid.max(random))
        })
        .collect();
    let mut best = 0usize;
    for (seed, count) in &results {
        println!("seed {seed}: {count}");
        best = best.max(*count);
    }
    eprintln!(
        "best {best}; seeds reaching it: {}",
        results
            .iter()
            .filter(|(_, count)| *count == best)
            .map(|(seed, _)| seed.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

fn model_center_on_bed(source: &std::path::Path, baked_stl: &std::path::Path) -> Option<(f64, f64)> {
    let (cx, cy, cz) = stl_center_3d(baked_stl).ok()?;
    let file = fs::File::open(source).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name("3D/3dmodel.model").ok()?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes).ok()?;
    goat_packer::assemble::source_model_center(&bytes, (cx, cy, cz)).ok()
}

//! Re-packs a job's saved plan (`<output>.state.json` / the webapp's `pack_state.json`) without a
//! slicer: the same footprint, keepouts and bed rules the packer used, so a layout question
//! ("why 47, not 48?") can be answered offline. Prints the grid baseline and the search count.
//!
//! Usage:
//! ```text
//! cargo run --release --example repack_state -- --state pack_state.json \
//!   --profile profiles/snapmaker_u1.json [--clearance 0.7] [--resolution 0.25] \
//!   [--edge-reserve 1.5] [--placements out.json]
//! ```

use std::{fs, path::PathBuf, process::exit, time::Instant};

use goat_packer::{
    Footprint, PackOptions, Rect, colored_packer::load_printer_profile, grid_layout, grow_beyond,
    pack_footprint, validate_result,
};

/// The packer's bed-edge reserve for supports (`SUPPORT_EDGE_RESERVE_MM` in colored_3mf_packer).
const DEFAULT_EDGE_RESERVE_MM: f64 = 1.5;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        exit(1);
    }
}

fn field<T: serde::de::DeserializeOwned>(plan: &serde_json::Value, key: &str) -> Result<T, String> {
    serde_json::from_value(plan[key].clone()).map_err(|e| format!("plan.{key}: {e}"))
}

fn run() -> Result<(), String> {
    let mut state_path: Option<PathBuf> = None;
    let mut profile_path: Option<PathBuf> = None;
    let mut placements_path: Option<PathBuf> = None;
    let mut check_path: Option<PathBuf> = None;
    let mut silhouette_from: Option<PathBuf> = None;
    let mut clearance = 0.7;
    let mut resolution = 0.25;
    let mut edge_reserve = DEFAULT_EDGE_RESERVE_MM;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} expects a value"));
        let number = |text: String| text.parse::<f64>().map_err(|e| format!("{flag}: {e}"));
        match flag.as_str() {
            "--state" => state_path = Some(value()?.into()),
            "--profile" => profile_path = Some(value()?.into()),
            "--placements" => placements_path = Some(value()?.into()),
            "--check" => check_path = Some(value()?.into()),
            "--silhouette-from" => silhouette_from = Some(value()?.into()),
            "--clearance" => clearance = number(value()?)?,
            "--resolution" => resolution = number(value()?)?,
            "--edge-reserve" => edge_reserve = number(value()?)?,
            _ => return Err(format!("unknown option: {flag}")),
        }
    }
    let state_path = state_path.ok_or("--state is required")?;
    let profile = load_printer_profile(&profile_path.ok_or("--profile is required")?)
        .map_err(|e| e.to_string())?;
    let state: serde_json::Value = serde_json::from_slice(
        &fs::read(&state_path).map_err(|e| format!("{}: {e}", state_path.display()))?,
    )
    .map_err(|e| e.to_string())?;
    let plan = &state["plan"];
    let mut footprint: Footprint = field(plan, "footprint")?;
    let fixed_keepouts: Vec<Rect> = field(plan, "fixed_keepouts")?;
    let tower_keepout: Option<Rect> = field(plan, "tower_keepout")?;
    let lock_orientation: bool = field(plan, "lock_orientation")?;
    if let Some(source) = silhouette_from {
        // Experiment: the body zone as the bare mesh silhouette (no support interfaces).
        let stl = std::env::temp_dir().join("repack_state_silhouette.stl");
        let triangles = goat_packer::embedded_mesh::embedded_mesh_triangles(&source).map_err(|e| e.to_string())?;
        goat_packer::embedded_mesh::write_binary_stl(&stl, &triangles).map_err(|e| e.to_string())?;
        footprint.body_loops = goat_packer::extract_footprint(&stl).map_err(|e| e.to_string())?.loops;
    }
    if plan["rule"].as_str() == Some("silhouette") {
        footprint.support_landing_loops.clear();
    }
    if lock_orientation && !footprint.body_loops.is_empty() {
        let bed = footprint.bed_loops.clone().unwrap_or_else(|| footprint.loops.clone());
        footprint.bed_loops = Some(grow_beyond(&bed, &footprint.body_loops, edge_reserve));
    }
    let (bed, body_bed) = match profile.support_edge_margin_mm {
        Some(_) => (profile.support_bed()?, Some(profile.usable_bed()?)),
        None => (profile.usable_bed()?, None),
    };
    let options = PackOptions {
        bed,
        body_bed,
        exclusions: fixed_keepouts,
        body_exclusions: tower_keepout.into_iter().collect(),
        resolution,
        clearance,
        angle_step_deg: 2.0,
        attempts: 16000,
        candidates_per_step: 2400,
        stall_attempts: 2500,
        max_objects: 0,
        lock_orientation,
        seed: 20270831,
        support_overlap_mm: None,
    };
    if let Some(path) = check_path {
        // A given layout (e.g. a hand-made plate's placements) against the same rules.
        let placements: Vec<goat_packer::PackedPlacement> = serde_json::from_slice(
            &fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
        )
        .map_err(|e| e.to_string())?;
        let layout = goat_packer::PackResult {
            placements,
            resolution_mm: resolution,
            requested_clearance_mm: clearance,
            angle_step_deg: options.angle_step_deg,
            attempts: 1,
            attempts_run: 1,
            seed: options.seed,
        };
        match validate_result(&footprint, &options, &layout) {
            Ok(()) => println!("check: {} objects valid", layout.placements.len()),
            Err(error) => println!("check: {error}"),
        }
        return Ok(());
    }
    let started = Instant::now();
    let grid = grid_layout(&footprint, &options);
    validate_result(&footprint, &options, &grid)?;
    println!("grid: {} objects ({:.1} s)", grid.placements.len(), started.elapsed().as_secs_f64());
    let started = Instant::now();
    let searched = pack_footprint(&footprint, &options);
    validate_result(&footprint, &options, &searched)?;
    println!("search: {} objects ({:.1} s)", searched.placements.len(), started.elapsed().as_secs_f64());
    if let Some(path) = placements_path {
        let best = if searched.placements.len() > grid.placements.len() { &searched } else { &grid };
        fs::write(&path, serde_json::to_vec_pretty(best).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

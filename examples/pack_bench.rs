//! Packing benchmark: how many objects, how fast, and how stable across input rotations and
//! seeds. The same physical model rotated before export must pack to the same count — the
//! packer sees only its silhouette — so any spread across `--rotations` is instability, not
//! geometry.
//!
//! Usage:
//! ```text
//! cargo run --release --example pack_bench -- \
//!   --stl tests/fixtures/goat4_v2.stl --profile profiles/p1s.json --overshoot 1.7 \
//!   --tower 225.85,200.10,246.15,252.15 --rotations 0,13,45,90,137 --seeds 3
//! ```
//! `--footprint-json <path>` loads a `Footprint` serialized as JSON instead of an STL (the
//! test fixtures in `tests/fixtures/`), `--dump-footprint <path>` writes the loaded one.

use std::{path::PathBuf, process::exit, time::Instant};

use goat_packer::{
    Footprint, PackOptions, Point, Rect, colored_packer::load_printer_profile, extract_footprint,
    grid_layout, pack_footprint, validate_result,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut stl: Option<PathBuf> = None;
    let mut footprint_json: Option<PathBuf> = None;
    let mut dump_footprint: Option<PathBuf> = None;
    let mut profile_path = PathBuf::from("profiles/p1s.json");
    let mut tower: Option<Rect> = None;
    let mut overshoot = 0.0f64;
    let mut rotations: Vec<f64> = vec![0.0];
    let mut seeds = 1u64;
    let mut first_seed = 20_270_831u64;
    let mut clearance = 0.7f64;
    let mut resolution = 0.25f64;
    let mut angle_step = 2.0f64;
    let mut attempts = 1_000usize;
    let mut candidates = 1_000usize;
    let mut stall = 0usize;
    let mut threads: Option<usize> = None;
    let mut args = std::env::args().skip(1);
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        let number = |value: &str| value.parse::<f64>().map_err(|e| format!("{option}: {e}"));
        match option.as_str() {
            "--stl" => stl = Some(value.into()),
            "--footprint-json" => footprint_json = Some(value.into()),
            "--dump-footprint" => dump_footprint = Some(value.into()),
            "--profile" => profile_path = value.into(),
            "--tower" => {
                let values: Vec<f64> = value.split(',').map(number).collect::<Result<_, _>>()?;
                if values.len() != 4 {
                    return Err("--tower needs x0,y0,x1,y1".to_owned());
                }
                tower = Some(Rect::new(values[0], values[1], values[2], values[3]));
            }
            "--overshoot" => overshoot = number(&value)?,
            "--rotations" => {
                rotations = value.split(',').map(number).collect::<Result<_, _>>()?;
            }
            "--seeds" => seeds = value.parse().map_err(|e| format!("{option}: {e}"))?,
            "--first-seed" => first_seed = value.parse().map_err(|e| format!("{option}: {e}"))?,
            "--clearance" => clearance = number(&value)?,
            "--resolution" => resolution = number(&value)?,
            "--angle-step" => angle_step = number(&value)?,
            "--attempts" => attempts = value.parse().map_err(|e| format!("{option}: {e}"))?,
            "--candidates" => candidates = value.parse().map_err(|e| format!("{option}: {e}"))?,
            "--threads" => threads = Some(value.parse().map_err(|e| format!("{option}: {e}"))?),
            "--stall" => stall = value.parse().map_err(|e| format!("{option}: {e}"))?,
            other => return Err(format!("unknown option: {other}")),
        }
    }
    if let Some(threads) = threads {
        if threads == 0 {
            return Err("--threads must be positive".to_owned());
        }
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .map_err(|e| e.to_string())?;
    }
    if seeds == 0 || rotations.is_empty() {
        return Err("at least one rotation and seed are required".to_owned());
    }
    let last_seed = first_seed.checked_add(seeds).ok_or("seed range overflow")?;
    let footprint = match (stl, footprint_json) {
        (Some(path), None) => extract_footprint(&path).map_err(|e| e.to_string())?,
        (None, Some(path)) => {
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?
        }
        _ => return Err("exactly one of --stl / --footprint-json is required".to_owned()),
    };
    if let Some(path) = dump_footprint {
        std::fs::write(&path, serde_json::to_vec(&footprint).unwrap())
            .map_err(|e| e.to_string())?;
    }
    let profile = load_printer_profile(&profile_path).map_err(|e| e.to_string())?;
    let mut exclusions = profile
        .fixed_keepouts(overshoot)
        .map_err(|e| e.to_string())?;
    exclusions.extend(tower);
    eprintln!(
        "footprint {:.2} x {:.2} mm, {} loops, {} vertices",
        footprint.width,
        footprint.height,
        footprint.loops.len(),
        footprint.loops.iter().map(Vec::len).sum::<usize>()
    );

    let mut counts = Vec::new();
    for &rotation in &rotations {
        let rotated = rotate_input(&footprint, rotation);
        for seed in first_seed..last_seed {
            let options = PackOptions {
                bed: profile.usable_bed()?,
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
            let started = Instant::now();
            let grid = grid_layout(&rotated, &options);
            let search = pack_footprint(&rotated, &options);
            let best = if search.placements.len() > grid.placements.len() {
                search
            } else {
                grid
            };
            let elapsed = started.elapsed().as_secs_f64();
            validate_result(&rotated, &options, &best)?;
            println!(
                "rotation {rotation:>7.2}° seed {seed}: {} objects in {elapsed:.2} s",
                best.placements.len()
            );
            counts.push(best.placements.len());
        }
    }
    let min = counts.iter().min().copied().unwrap_or(0);
    let max = counts.iter().max().copied().unwrap_or(0);
    println!("summary: min {min}, max {max}, runs {}", counts.len());
    Ok(())
}

/// The footprint of the same mesh rotated by `degrees` before export: loops rotate and get
/// re-centered on the rotated bbox center, exactly what `extract_footprint` would produce.
fn rotate_input(footprint: &Footprint, degrees: f64) -> Footprint {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let rotate = |loops: &[Vec<Point>]| -> Vec<Vec<Point>> {
        loops
            .iter()
            .map(|loop_| {
                loop_
                    .iter()
                    .map(|p| Point {
                        x: p.x * cos - p.y * sin,
                        y: p.x * sin + p.y * cos,
                    })
                    .collect()
            })
            .collect()
    };
    let loops = rotate(&footprint.loops);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in loops.iter().flatten() {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    let (cx, cy) = ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5);
    let shift = |loops: Vec<Vec<Point>>| -> Vec<Vec<Point>> {
        loops
            .into_iter()
            .map(|loop_| {
                loop_
                    .into_iter()
                    .map(|p| Point {
                        x: p.x - cx,
                        y: p.y - cy,
                    })
                    .collect()
            })
            .collect()
    };
    Footprint {
        loops: shift(loops),
        body_loops: shift(rotate(&footprint.body_loops)),
        bed_loops: footprint.bed_loops.as_ref().map(|l| shift(rotate(l))),
        width: max_x - min_x,
        height: max_y - min_y,
        min_z: footprint.min_z,
        max_z: footprint.max_z,
        triangle_count: footprint.triangle_count,
    }
}

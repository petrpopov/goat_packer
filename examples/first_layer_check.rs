//! Offline check of the landing (spec 05) and brim (spec 03 §5) models against printed first
//! layers.
//!
//! Usage: `cargo run --release --example first_layer_check -- [--build <name>]
//! <rot000 project.3mf> <derived dir> [<derived dir> ...]`, where a derived dir holds
//! `rot*/run*.json` files of the calibration corpus (`l1.body_polygon` and `l1.brim_polygon`
//! in the object frame, `rotation_deg`). The build defaults to the project's `Application`.
//! For every slice it prints how far the printed first layer and brim reach outside the
//! predictions (must be 0) and the areas.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use goat_packer::embedded_mesh::load_object_mesh;
use goat_packer::first_layer::{ProjectSettings, SlicerBuild, brim, landing};
use goat_packer::mesh_section::{SectionOptions, object_section};
use goat_packer::orca_project::project_settings;
use goat_packer::tower_model::read_filament_sources;
use goat_packer::{Point, point_in_footprint, polygon_area};

/// Boundary sampling step for the exceedance, as in the corpus measurement (05 §3).
const SAMPLE_STEP_MM: f64 = 0.02;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let forced = match args.first().map(String::as_str) {
        Some("--build") => {
            let name = args.get(1).cloned().ok_or("--build needs a name")?;
            args.drain(..2);
            Some(SlicerBuild::from_name(&name).ok_or("unknown build")?)
        }
        _ => None,
    };
    let (project, dirs) = args
        .split_first()
        .ok_or("usage: [--build <name>] <project.3mf> <derived dir>...")?;
    let mesh = load_object_mesh(Path::new(project))?;
    let settings = ProjectSettings::new(project_settings(Path::new(project))?);
    let build = forced
        .or_else(|| application(Path::new(project)))
        .unwrap_or(SlicerBuild::BambuStudio);
    let filaments = read_filament_sources(Path::new(project))?.part_filaments_of(&mesh);
    let mut landings = BTreeMap::new();
    let mut worst = 0.0_f64;
    let mut worst_section = 0.0_f64;
    let mut worst_brim = 0.0_f64;
    for dir in dirs {
        for file in slice_files(Path::new(dir))? {
            let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&file)?)?;
            let rotation = json["rotation_deg"].as_f64().unwrap_or(0.0);
            let key = (rotation * 1000.0).round() as i64;
            if !landings.contains_key(&key) {
                let predicted = landing(&mesh, &settings, build, rotation)?;
                let section = object_section(
                    &mesh.rotated(rotation),
                    predicted.slice_z_mm,
                    &SectionOptions::slicer(
                        settings.number("slice_closing_radius").unwrap_or(0.049),
                    ),
                );
                let brim = brim(&mesh, &filaments, &settings, build, rotation)?;
                landings.insert(key, (predicted, section, brim));
            }
            let (predicted, section, brim) = &landings[&key];
            let printed = printed_rings(&json["object"]["l1"]["body_polygon"], rotation);
            let excess = max_excess(&printed, &predicted.loops);
            let past_section = max_excess(&printed, section);
            let printed_brim = printed_rings(&json["object"]["l1"]["brim_polygon"], rotation);
            let brim_excess = max_excess(&printed_brim, &brim.loops);
            worst = worst.max(excess);
            worst_section = worst_section.max(past_section);
            worst_brim = worst_brim.max(brim_excess);
            println!(
                "{} rot {rotation:5.1}: printed {:7.2} mm², landing {:7.2} mm², outside landing {excess:.4} mm, outside S_0 {past_section:.4} mm; brim printed {:6.1} mm², predicted {:6.1} mm², outside {brim_excess:.4} mm",
                file.display(),
                even_odd_area(&printed),
                even_odd_area(&predicted.loops),
                even_odd_area(&printed_brim),
                even_odd_area(&brim.loops),
            );
        }
    }
    println!(
        "worst over all slices: outside landing {worst:.4} mm, outside S_0 {worst_section:.4} mm, outside brim {worst_brim:.4} mm (build {build:?})"
    );
    Ok(())
}

fn application(project: &Path) -> Option<SlicerBuild> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(project).ok()?).ok()?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("3D/3dmodel.model").ok()?, &mut text)
        .ok()?;
    let start = text.find("name=\"Application\">")? + "name=\"Application\">".len();
    let end = start + text[start..].find('<')?;
    SlicerBuild::from_name(&text[start..end])
}

fn slice_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for rotation in std::fs::read_dir(dir)? {
        let rotation = rotation?.path();
        if rotation.is_dir() {
            for run in std::fs::read_dir(&rotation)? {
                let run = run?.path();
                if run.extension().is_some_and(|e| e == "json") {
                    files.push(run);
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Corpus polygons (`[[ring xy...], [hole xy...]]` per polygon, object frame) turned into the
/// plate-aligned frame of `rotation` degrees.
fn printed_rings(value: &serde_json::Value, rotation: f64) -> Vec<Vec<Point>> {
    let (sin, cos) = goat_packer::embedded_mesh::sin_cos_degrees(rotation);
    let mut rings = Vec::new();
    for polygon in value.as_array().into_iter().flatten() {
        for ring in polygon.as_array().into_iter().flatten() {
            let flat: Vec<f64> = ring
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_f64())
                .collect();
            let mut points: Vec<Point> = flat
                .chunks(2)
                .map(|xy| Point {
                    x: xy[0] * cos - xy[1] * sin,
                    y: xy[0] * sin + xy[1] * cos,
                })
                .collect();
            if let Some(first) = points.first().copied() {
                points.push(first);
                rings.push(points);
            }
        }
    }
    rings
}

fn even_odd_area(rings: &[Vec<Point>]) -> f64 {
    // Outer rings and holes come with opposite signs from the resolvers; corpus rings keep the
    // shapely orientation (outer CCW, holes CW), so the signed sum is the area.
    rings
        .iter()
        .map(|ring| polygon_area(ring))
        .sum::<f64>()
        .abs()
}

/// Largest distance from a printed boundary point outside `predicted` to `predicted`
/// (infinite when something printed where nothing was predicted).
fn max_excess(printed: &[Vec<Point>], predicted: &[Vec<Point>]) -> f64 {
    if predicted.is_empty() {
        return if printed.is_empty() {
            0.0
        } else {
            f64::INFINITY
        };
    }
    let mut worst = 0.0_f64;
    for ring in printed {
        for pair in ring.windows(2) {
            let length = (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
            let steps = (length / SAMPLE_STEP_MM).ceil().max(1.0) as usize;
            for step in 0..steps {
                let t = step as f64 / steps as f64;
                let x = pair[0].x + t * (pair[1].x - pair[0].x);
                let y = pair[0].y + t * (pair[1].y - pair[0].y);
                if !point_in_footprint(predicted, x, y) {
                    worst = worst.max(distance_to_rings(predicted, x, y));
                }
            }
        }
    }
    worst
}

fn distance_to_rings(rings: &[Vec<Point>], x: f64, y: f64) -> f64 {
    rings
        .iter()
        .flat_map(|ring| ring.windows(2))
        .map(|pair| {
            let (ax, ay, bx, by) = (pair[0].x, pair[0].y, pair[1].x, pair[1].y);
            let (dx, dy) = (bx - ax, by - ay);
            let length2 = dx * dx + dy * dy;
            let t = if length2 == 0.0 {
                0.0
            } else {
                (((x - ax) * dx + (y - ay) * dy) / length2).clamp(0.0, 1.0)
            };
            (x - ax - t * dx).hypot(y - ay - t * dy)
        })
        .fold(f64::INFINITY, f64::min)
}

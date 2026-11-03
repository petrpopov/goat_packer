//! Offline check of the prime-tower model (spec 04) against sliced towers of the calibration
//! corpus.
//!
//! Usage: `cargo run --release --example tower_check -- [--build <name>] <project.3mf>
//! <derived slice.json> [[--build <name>] <project.3mf> <derived slice.json> ...]`. A derived
//! slice holds `tower.per_layer_bbox` (centre-line bboxes per layer, plate frame) and
//! `tower.top_z`. `--build` applies to the pairs after it; the default is the project's
//! `Application`. For each pair it prints how far the printed column and first layer reach
//! past the prediction (positive = unsafe, covered up to `TOWER_EDGE_TOLERANCE_MM`), and the
//! predicted and printed tower tops.

use std::path::Path;

use goat_packer::Rect;
use goat_packer::embedded_mesh::load_object_mesh;
use goat_packer::first_layer::{ProjectSettings, SlicerBuild};
use goat_packer::orca_project::project_settings;
use goat_packer::tower_model::{
    TowerPlate, printed_anchor, printed_tower, read_filament_sources, tool_schedule,
};

type Error = Box<dyn std::error::Error + Send + Sync>;

fn main() -> Result<(), Error> {
    let mut args = std::env::args().skip(1);
    let mut forced = None;
    let mut worst = (f64::MIN, f64::MIN);
    while let Some(arg) = args.next() {
        if arg == "--build" {
            let name = args.next().ok_or("--build needs a name")?;
            forced = Some(SlicerBuild::from_name(&name).ok_or("unknown build")?);
            continue;
        }
        let slice = args
            .next()
            .ok_or("every project needs a derived slice json")?;
        let (body, first) = check(Path::new(&arg), Path::new(&slice), forced)?;
        worst = (worst.0.max(body), worst.1.max(first));
    }
    println!(
        "worst printed past predicted: column {:+.3} mm, first layer {:+.3} mm",
        worst.0, worst.1
    );
    Ok(())
}

/// Prints one comparison; returns the largest printed-past-predicted distance of the column
/// and of the first layer.
fn check(project: &Path, slice: &Path, forced: Option<SlicerBuild>) -> Result<(f64, f64), Error> {
    let mesh = load_object_mesh(project)?;
    let settings = ProjectSettings::new(project_settings(project)?);
    let build = forced
        .or_else(|| application(project))
        .unwrap_or(SlicerBuild::BambuStudio);
    let sources = read_filament_sources(project)?;
    let schedule = tool_schedule(&mesh, &sources, &settings, build)?;
    let plate = TowerPlate {
        printable: printable_area(&settings),
        object_height: mesh.height(),
    };
    let tower = printed_tower(&settings, &schedule, build, &plate)?.ok_or("no tower predicted")?;
    let anchor = tower
        .anchor_range
        .and_then(|range| printed_anchor(build, tower.source_anchor, range))
        .unwrap_or(tower.source_anchor);
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(slice)?)?;
    let layers = json["tower"]["per_layer_bbox"]
        .as_array()
        .ok_or("no tower in the slice")?;
    // The first layer above the chamfer is the bare column.
    let column = bbox(
        &layers
            .get(tower.chamfer.len() + 1)
            .ok_or("tower too short")?["bbox"],
    )?;
    let first = bbox(&json["tower"]["l1_bbox"])?;
    let predicted = Rect::new(
        anchor.0,
        anchor.1,
        anchor.0 + tower.body_size.0,
        anchor.1 + tower.body_size.1,
    );
    let column_past = past(column, predicted);
    let first_past = past(first, predicted.grown(tower.l0_brim_mm));
    println!(
        "{} {build:?}: body {:.3} × {:.3}, k_max {}, column past [{}], L0 past [{}], top {:.2} (printed {:.2})",
        project.file_name().unwrap_or_default().to_string_lossy(),
        tower.body_size.0,
        tower.body_size.1,
        tower.max_changes,
        format_sides(column_past),
        format_sides(first_past),
        tower.height,
        json["tower"]["top_z"].as_f64().unwrap_or(f64::NAN),
    );
    Ok((largest(column_past), largest(first_past)))
}

/// How far each side of `printed` lies outside `predicted` (left, bottom, right, top).
fn past(printed: Rect, predicted: Rect) -> [f64; 4] {
    [
        predicted.min_x - printed.min_x,
        predicted.min_y - printed.min_y,
        printed.max_x - predicted.max_x,
        printed.max_y - predicted.max_y,
    ]
}

fn largest(sides: [f64; 4]) -> f64 {
    sides.into_iter().fold(f64::MIN, f64::max)
}

fn format_sides(sides: [f64; 4]) -> String {
    sides.map(|side| format!("{side:+.3}")).join(" ")
}

fn bbox(value: &serde_json::Value) -> Result<Rect, Error> {
    let numbers: Vec<f64> = value
        .as_array()
        .ok_or("bbox is not an array")?
        .iter()
        .filter_map(serde_json::Value::as_f64)
        .collect();
    match numbers[..] {
        [min_x, min_y, max_x, max_y] => Ok(Rect::new(min_x, min_y, max_x, max_y)),
        _ => Err("bbox needs four numbers".into()),
    }
}

/// The bbox of `printable_area` (`"XxY"` corner strings).
fn printable_area(settings: &ProjectSettings) -> Rect {
    let corners: Vec<(f64, f64)> = settings
        .texts("printable_area")
        .iter()
        .filter_map(|corner| {
            let (x, y) = corner.split_once('x')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect();
    let fold = |pick: fn(&(f64, f64)) -> f64, start: f64, op: fn(f64, f64) -> f64| {
        corners.iter().map(pick).fold(start, op)
    };
    Rect::new(
        fold(|c| c.0, f64::INFINITY, f64::min),
        fold(|c| c.1, f64::INFINITY, f64::min),
        fold(|c| c.0, f64::NEG_INFINITY, f64::max),
        fold(|c| c.1, f64::NEG_INFINITY, f64::max),
    )
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

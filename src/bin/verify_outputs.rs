//! Independent exact-geometry and 3MF integrity checks for packed projects.
//!
//! Rust port of `verify_outputs.py` (contract item 9 in
//! `docs/superpowers/specs/2026-09-01-eliminate-python-rust-port-design.md`). This binary is the
//! project's independent second opinion: it shares no packing code — only the footprint/geometry
//! types from the library — so a packing bug cannot hide from the checks that gate a print.
//!
//! CLI (mirrors the Python; `--flag value` and `--flag=value` both accepted):
//!
//! ```text
//! verify_outputs <stl> <placements> <project>
//!                [--minimum-gap F] [--edge-margin F]
//!                [--purge-line x0,y0,x1,y1,w]... [--purge-gap F]
//!                [--keepout min_x,min_y,max_x,max_y]... [--report PATH]
//! ```
//!
//! On success the JSON report goes to stdout (and to `--report`). On the first failed check the
//! process exits 1 with an `AssertionError: ...` message carrying the same identifying prefix as
//! the Python verifier.

use std::{
    collections::HashSet,
    fs, io,
    io::Read as _,
    path::{Path, PathBuf},
};

use geo::LineString;
use geo::Polygon as GeoPolygon;
use geo::algorithm::area::Area;
use geo::algorithm::bool_ops::BooleanOps;
use geo::algorithm::covers::Covers;
use geo::algorithm::line_measures::{Distance, Euclidean};
use goat_packer::{Point, Rect, axis_aligned_line_keepout, extract_footprint};
use quick_xml::{
    events::{BytesStart, Event},
    name::{NamespaceResolver, ResolveResult},
    reader::NsReader,
};
use serde::Serialize;
use serde_json::Value;
use zip::ZipArchive;

const CORE: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const PRODUCTION: &str = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06";

/// A verifier failure. `Assertion` is a failed check (the Python raised `AssertionError` with the
/// same message); `Error` is a tooling failure (the Python crashed with a different exception).
/// Both exit non-zero; only the stderr prefix differs.
#[derive(Debug)]
enum Failure {
    Assertion(String),
    Error(String),
}

fn assertion(message: impl Into<String>) -> Failure {
    Failure::Assertion(message.into())
}

/// 4x3 row-major 3MF transform (last row = translation).
type Matrix = [[f64; 3]; 4];

const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, 0.0],
];

/// Parse a 3MF transform into a 4x3 row-major matrix (last row = translation).
///
/// 3MF uses row vectors: p' = p·M, i.e. p'_j = sum_i p_i * M[i][j] + M[3][j].
fn parse_transform(text: Option<&str>) -> Result<Matrix, Failure> {
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return Ok(IDENTITY);
    };
    let values = text
        .split_whitespace()
        .map(|value| {
            value
                .parse::<f64>()
                .map_err(|error| Failure::Error(format!("invalid number in 3MF transform: {error}")))
        })
        .collect::<Result<Vec<f64>, Failure>>()?;
    if values.len() != 12 {
        return Err(assertion(format!(
            "3MF transform is not 12 numbers: {text:?}"
        )));
    }
    Ok([
        [values[0], values[1], values[2]],
        [values[3], values[4], values[5]],
        [values[6], values[7], values[8]],
        [values[9], values[10], values[11]],
    ])
}

fn apply_matrix(matrix: &Matrix, point: (f64, f64, f64)) -> (f64, f64, f64) {
    let (x, y, z) = point;
    (
        x * matrix[0][0] + y * matrix[1][0] + z * matrix[2][0] + matrix[3][0],
        x * matrix[0][1] + y * matrix[1][1] + z * matrix[2][1] + matrix[3][1],
        x * matrix[0][2] + y * matrix[1][2] + z * matrix[2][2] + matrix[3][2],
    )
}

#[derive(Debug, Serialize)]
struct Report {
    project: String,
    printer_model: Option<String>,
    count: usize,
    minimum_exact_gap_mm: Value,
    required_edge_margin_mm: f64,
    minimum_edge_clearance_mm: f64,
    required_purge_gap_mm: Value,
    minimum_purge_gap_mm: Value,
    all_inside_printable_area: bool,
    exclusion_clear: bool,
    zip_crc_ok: bool,
    metadata_object_counts_match: bool,
    embedded_mesh_transform_ok: bool,
}

struct Args {
    stl: PathBuf,
    placements: PathBuf,
    project: PathBuf,
    minimum_gap: f64,
    edge_margin: f64,
    purge_lines: Vec<[f64; 5]>,
    purge_gap: f64,
    custom_keepouts: Vec<Rect>,
    report: Option<PathBuf>,
}

fn main() {
    let args = match parse_args_from(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(usage) => {
            eprintln!("error: {usage}");
            std::process::exit(2);
        }
    };
    match verify(&args) {
        Ok(report) => {
            let text = serde_json::to_string_pretty(&report).expect("report serializes");
            println!("{text}");
            if let Some(path) = &args.report
                && let Err(error) = fs::write(path, text + "\n")
            {
                eprintln!("error: failed to write report {}: {error}", path.display());
                std::process::exit(1);
            }
        }
        Err(Failure::Assertion(message)) => {
            eprintln!("AssertionError: {message}");
            std::process::exit(1);
        }
        Err(Failure::Error(message)) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
    }
}

fn parse_flag_f64(option: &str, raw: String, name: &str) -> Result<f64, String> {
    raw.parse::<f64>()
        .map_err(|_| format!("{option}: {name} must be a number, got {raw:?}"))
}

fn parse_args_from<I: Iterator<Item = String>>(mut args: I) -> Result<Args, String> {
    let mut positionals = Vec::new();
    let mut minimum_gap = 0.5;
    let mut edge_margin = 0.0;
    let mut purge_lines = Vec::new();
    let mut purge_gap = 0.5;
    let mut custom_keepouts = Vec::new();
    let mut report = None;
    while let Some(option) = args.next() {
        let Some(flag) = option.strip_prefix("--") else {
            positionals.push(option);
            continue;
        };
        let (flag, inline) = match flag.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (flag, None),
        };
        // The Python accepted `--flag value` only; both spellings parse here.
        let mut value = || -> Result<String, String> {
            match inline {
                Some(value) => Ok(value.to_owned()),
                None => args.next().ok_or_else(|| format!("{option}: expected one argument")),
            }
        };
        match flag {
            "minimum-gap" => minimum_gap = parse_flag_f64(&option, value()?, "minimum gap")?,
            "edge-margin" => edge_margin = parse_flag_f64(&option, value()?, "edge margin")?,
            "purge-gap" => purge_gap = parse_flag_f64(&option, value()?, "purge gap")?,
            "purge-line" => {
                let raw = value()?;
                let parts = raw
                    .split(',')
                    .map(|part| part.parse::<f64>())
                    .collect::<Result<Vec<f64>, _>>()
                    .map_err(|_| "--purge-line must be x0,y0,x1,y1,width".to_owned())?;
                if parts.len() != 5 {
                    return Err("--purge-line must be x0,y0,x1,y1,width".to_owned());
                }
                purge_lines.push(parts.try_into().expect("five numbers"));
            }
            "keepout" => {
                let raw = value()?;
                let parts = raw
                    .split(',')
                    .map(|part| part.parse::<f64>())
                    .collect::<Result<Vec<f64>, _>>()
                    .map_err(|_| "--keepout must be min_x,min_y,max_x,max_y".to_owned())?;
                if parts.len() != 4 {
                    return Err("--keepout must be min_x,min_y,max_x,max_y".to_owned());
                }
                custom_keepouts.push(Rect::new(parts[0], parts[1], parts[2], parts[3]));
            }
            "report" => report = Some(PathBuf::from(value()?)),
            _ => return Err(format!("unknown option: {option}")),
        }
    }
    if positionals.len() != 3 {
        return Err(
            "usage: verify_outputs <stl> <placements> <project> [--minimum-gap F] \
             [--edge-margin F] [--purge-line x0,y0,x1,y1,w]... [--purge-gap F] \
             [--keepout min_x,min_y,max_x,max_y]... [--report PATH]"
                .to_owned(),
        );
    }
    let mut positionals = positionals.into_iter();
    let stl = positionals.next().expect("three positionals");
    let placements = positionals.next().expect("three positionals");
    let project = positionals.next().expect("three positionals");
    Ok(Args {
        stl: PathBuf::from(stl),
        placements: PathBuf::from(placements),
        project: PathBuf::from(project),
        minimum_gap,
        edge_margin,
        purge_lines,
        purge_gap,
        custom_keepouts,
        report,
    })
}

fn verify(args: &Args) -> Result<Report, Failure> {
    let placement_text = fs::read_to_string(&args.placements).map_err(|error| {
        Failure::Error(format!(
            "failed to read placements {}: {error}",
            args.placements.display()
        ))
    })?;
    let placement_data: Value = serde_json::from_str(&placement_text)
        .map_err(|error| Failure::Error(format!("failed to parse placements JSON: {error}")))?;
    let placement_values = placement_data
        .get("placements")
        .and_then(Value::as_array)
        .ok_or_else(|| Failure::Error("placements JSON has no \"placements\" array".to_owned()))?;
    let placements = placement_values
        .iter()
        .map(|placement| {
            let field = |name: &str| {
                placement.get(name).and_then(Value::as_f64).ok_or_else(|| {
                    Failure::Error(format!("placement is missing a numeric \"{name}\""))
                })
            };
            Ok((field("x_mm")?, field("y_mm")?, field("angle_deg")?))
        })
        .collect::<Result<Vec<(f64, f64, f64)>, Failure>>()?;
    let count = placements.len();

    // extract_footprint returns the min-Z bottom outline (exterior loop first, holes after),
    // already centered on the STL's XY bbox center — the geometry the Python rebuilt from the
    // bottom triangles with lower_polygon().
    let footprint = extract_footprint(&args.stl).map_err(|error| {
        Failure::Error(format!("failed to read STL {}: {error}", args.stl.display()))
    })?;
    // loops is never empty when extract_footprint succeeded (a boundary implies a loop).
    let base = footprint.loops.first().expect("footprint has an exterior loop");
    let polygons: Vec<GeoPolygon<f64>> = placements
        .iter()
        .map(|(x_mm, y_mm, angle_deg)| world_polygon(&footprint.loops, base, *angle_deg, *x_mm, *y_mm))
        .collect();

    let mut archive = ZipArchive::new(
        fs::File::open(&args.project)
            .map_err(|error| Failure::Error(format!("failed to open project {}: {error}", args.project.display())))?,
    )
    .map_err(|error| Failure::Error(format!("failed to read project zip: {error}")))?;
    // Reading every entry to EOF makes the zip crate verify each member's CRC32 — the
    // testzip() equivalent.
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| Failure::Error(format!("failed to read archive entry {index}: {error}")))?;
        let name = entry.name().to_owned();
        io::copy(&mut entry, &mut io::sink())
            .map_err(|_| assertion(format!("ZIP CRC failed: {name}")))?;
    }

    let settings: Value = serde_json::from_str(
        &read_entry(&mut archive, "Metadata/project_settings.config")?,
    )
    .map_err(|error| {
        Failure::Error(format!("failed to parse Metadata/project_settings.config: {error}"))
    })?;
    let printable_points = parse_xy(
        settings
            .get("printable_area")
            .and_then(Value::as_array)
            .ok_or_else(|| Failure::Error("\"printable_area\" missing from project_settings".to_owned()))?,
    )?;
    let printable_bounds = bounds(&printable_points);
    // shapely's buffer(-margin, join_style=2) on the (always rectangular) printable area is
    // exactly this rectangle inset.
    let safe_min_x = printable_bounds.0 + args.edge_margin;
    let safe_min_y = printable_bounds.1 + args.edge_margin;
    let safe_max_x = printable_bounds.2 - args.edge_margin;
    let safe_max_y = printable_bounds.3 - args.edge_margin;
    if safe_min_x >= safe_max_x || safe_min_y >= safe_max_y {
        return Err(assertion(format!(
            "edge margin {} mm consumes printable area",
            py_f64(args.edge_margin)
        )));
    }
    let safe_printable = rect_polygon(safe_min_x, safe_min_y, safe_max_x, safe_max_y);
    let printable_boundary = LineString::from(closed_ring(&printable_points));
    let exclusion = settings
        .get("bed_exclude_area")
        .and_then(Value::as_array)
        .filter(|values| values.len() >= 3)
        .map(|values| {
            parse_xy(values).map(|points| GeoPolygon::new(LineString::from(closed_ring(&points)), vec![]))
        })
        .transpose()?;

    let model = parse_project_model(&read_entry(&mut archive, "3D/3dmodel.model")?)?;
    let (config_objects, instances) =
        parse_model_settings(&read_entry(&mut archive, "Metadata/model_settings.config")?)?;

    if model.items.len() != placements.len() {
        return Err(assertion(format!(
            "object-count mismatch: placements={}, resources={}, items={}, config={}, instances={}",
            placements.len(),
            model.resource_objects,
            model.items.len(),
            config_objects,
            instances
        )));
    }
    if ![1, placements.len()].contains(&model.resource_objects)
        || ![1, placements.len()].contains(&config_objects)
        || ![1, placements.len()].contains(&instances)
    {
        return Err(assertion(format!(
            "unsupported resource/config/instance cardinality: resources={}, config={}, instances={}",
            model.resource_objects, config_objects, instances
        )));
    }
    // A missing objectid/id maps to "" which never matches a real id — the same set-inequality
    // outcome as the Python's None in the set.
    let item_object_ids: HashSet<String> = model
        .items
        .iter()
        .map(|item| item.object_id.clone().unwrap_or_default())
        .collect();
    if model.resource_ids != item_object_ids {
        return Err(assertion(
            "build items do not reference exactly the resource objects".to_owned(),
        ));
    }
    let archive_names: HashSet<String> = archive.file_names().map(str::to_owned).collect();
    let mut component_paths = Vec::new();
    for path in &model.component_paths {
        let Some(path) = path else {
            return Err(Failure::Error("component without a p:path attribute".to_owned()));
        };
        component_paths.push(path.trim_start_matches('/').to_owned());
    }
    if component_paths.iter().any(|path| !archive_names.contains(path)) {
        return Err(assertion(
            "a component path is missing from the archive".to_owned(),
        ));
    }
    // Python indexed paths[0] and crashed on an empty list; same exit code, clearer message.
    let Some(first_path) = component_paths.first() else {
        return Err(assertion(
            "a component path is missing from the archive".to_owned(),
        ));
    };
    let embedded_vertices = parse_mesh_vertices(&read_entry(&mut archive, first_path)?)?;
    if embedded_vertices.is_empty() {
        return Err(assertion(
            "embedded component mesh has no vertices".to_owned(),
        ));
    }
    // The component's own transform sits between the embedded mesh and every build item
    // (world = mesh ∘ component ∘ item). It can carry a translation (Goat 3:
    // (-35.38, +40.12, -4.3)); ignoring it makes both the XY and Z placement checks lie.
    let component_matrix = parse_transform(model.first_resource_component_transform.as_deref())?;
    let local_vertices: Vec<(f64, f64, f64)> = embedded_vertices
        .iter()
        .map(|vertex| apply_matrix(&component_matrix, *vertex))
        .collect();
    let local_min_z = local_vertices.iter().map(|vertex| vertex.2).fold(f64::INFINITY, f64::min);
    let embedded_height =
        local_vertices.iter().map(|vertex| vertex.2).fold(f64::NEG_INFINITY, f64::max) - local_min_z;

    let stl_center = stl_xy_center(&args.stl)?;
    for (index, item) in model.items.iter().enumerate() {
        let expected = &placements[index];
        let Some(transform) = item.transform.as_deref() else {
            return Err(assertion(format!("item {index} transform is not 3x4")));
        };
        let values = transform
            .split_whitespace()
            .map(|value| {
                value.parse::<f64>().map_err(|error| {
                    Failure::Error(format!("item {index} transform has a non-numeric value: {error}"))
                })
            })
            .collect::<Result<Vec<f64>, Failure>>()?;
        if values.len() != 12 {
            return Err(assertion(format!("item {index} transform is not 3x4")));
        }
        let angle = values[1].atan2(values[0]).to_degrees().rem_euclid(360.0);
        let expected_angle = expected.2;
        if (((angle - expected_angle + 180.0).rem_euclid(360.0)) - 180.0).abs() > 1e-6 {
            return Err(assertion(format!(
                "item {index} angle mismatch: {} != {}",
                py_f64(angle),
                py_f64(expected_angle)
            )));
        }
        // The item translation places the object-local footprint center (embedded mesh plus
        // the source component's transform) at (x_mm, y_mm), so the expected translation is
        // the placement minus the angle-rotated STL bbox center — the same compensation
        // build_3mf.py applies. Comparing raw matrix[9]/[10] to the placement would only hold
        // for a component whose own translation is zero.
        let radians = expected_angle.to_radians();
        let (center_x, center_y) = stl_center;
        let expected_tx = expected.0 - (center_x * radians.cos() - center_y * radians.sin());
        let expected_ty = expected.1 - (center_x * radians.sin() + center_y * radians.cos());
        if (values[9] - expected_tx).abs() > 1e-6 || (values[10] - expected_ty).abs() > 1e-6 {
            return Err(assertion(format!(
                "item {index} XY translation mismatch: ({}, {}) != ({}, {})",
                py_f64(values[9]),
                py_f64(values[10]),
                py_f64(expected_tx),
                py_f64(expected_ty)
            )));
        }
        let item_matrix = parse_transform(item.transform.as_deref())?;
        let mut transformed_min_z = f64::INFINITY;
        let mut transformed_max_z = f64::NEG_INFINITY;
        for vertex in &local_vertices {
            let z = apply_matrix(&item_matrix, *vertex).2;
            transformed_min_z = transformed_min_z.min(z);
            transformed_max_z = transformed_max_z.max(z);
        }
        if transformed_min_z.abs() > 1e-5 || (transformed_max_z - embedded_height).abs() > 1e-4 {
            return Err(assertion(format!(
                "item {index} is not on bed: z={}..{}",
                py_f64(transformed_min_z),
                py_f64(transformed_max_z)
            )));
        }
    }

    let mut edge_clearances = Vec::new();
    for (index, polygon) in polygons.iter().enumerate() {
        if !safe_printable.covers(polygon) {
            return Err(assertion(format!(
                "placement {index} violates {:.3} mm printable-area margin",
                args.edge_margin
            )));
        }
        edge_clearances.push(Euclidean.distance(polygon, &printable_boundary));
        if let Some(exclusion) = &exclusion
            && polygon.intersection(exclusion).unsigned_area() > 1e-7
        {
            return Err(assertion(format!(
                "placement {index} enters exclusion polygon"
            )));
        }
        for keepout in &args.custom_keepouts {
            let keepout_polygon = rect_polygon(keepout.min_x, keepout.min_y, keepout.max_x, keepout.max_y);
            if polygon.intersection(&keepout_polygon).unsigned_area() > 1e-7 {
                return Err(assertion(format!(
                    "placement {index} enters custom keepout"
                )));
            }
        }
    }

    let mut minimum_distance = f64::INFINITY;
    for (i, polygon) in polygons.iter().enumerate() {
        for previous in polygons.iter().take(i) {
            minimum_distance = minimum_distance.min(Euclidean.distance(polygon, previous));
        }
    }
    if minimum_distance + 1e-6 < args.minimum_gap {
        return Err(assertion(format!(
            "minimum exact gap {:.6} mm is below {:.6} mm",
            minimum_distance, args.minimum_gap
        )));
    }
    // The shapely buffer(cap_style=2, join_style=2) of an axis-aligned segment is this plain
    // rectangle (never buffer-discretized arcs).
    let mut purge_polygons = Vec::new();
    for [x0, y0, x1, y1, width] in &args.purge_lines {
        let rect = axis_aligned_line_keepout(*x0, *y0, *x1, *y1, *width).map_err(Failure::Error)?;
        purge_polygons.push(rect_polygon(rect.min_x, rect.min_y, rect.max_x, rect.max_y));
    }
    let mut minimum_purge_distance = f64::INFINITY;
    for polygon in &polygons {
        for purge in &purge_polygons {
            minimum_purge_distance = minimum_purge_distance.min(Euclidean.distance(polygon, purge));
        }
    }
    if minimum_purge_distance + 1e-6 < args.purge_gap {
        return Err(assertion(format!(
            "minimum purge-line gap {:.6} mm is below {:.6} mm",
            minimum_purge_distance, args.purge_gap
        )));
    }

    Ok(Report {
        project: args.project.display().to_string(),
        printer_model: settings.get("printer_model").and_then(Value::as_str).map(str::to_owned),
        count,
        minimum_exact_gap_mm: json_number(minimum_distance),
        required_edge_margin_mm: args.edge_margin,
        minimum_edge_clearance_mm: edge_clearances
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min),
        required_purge_gap_mm: if purge_polygons.is_empty() {
            Value::Null
        } else {
            json_number(args.purge_gap)
        },
        minimum_purge_gap_mm: if purge_polygons.is_empty() {
            Value::Null
        } else {
            json_number(minimum_purge_distance)
        },
        all_inside_printable_area: true,
        exclusion_clear: true,
        zip_crc_ok: true,
        metadata_object_counts_match: true,
        embedded_mesh_transform_ok: true,
    })
}

/// Rotate the footprint CCW by `angle_deg` about the loop origin and translate by
/// (`x_mm`, `y_mm`) — shapely `rotate(origin=(0, 0))` + `translate`.
fn world_polygon(
    loops: &[Vec<Point>],
    base: &[Point],
    angle_deg: f64,
    x_mm: f64,
    y_mm: f64,
) -> GeoPolygon<f64> {
    let radians = angle_deg.to_radians();
    let (sin, cos) = radians.sin_cos();
    let place = |loop_points: &[Point]| {
        loop_points
            .iter()
            .map(|point| (point.x * cos - point.y * sin + x_mm, point.x * sin + point.y * cos + y_mm))
            .collect::<Vec<(f64, f64)>>()
    };
    let interiors = loops[1..]
        .iter()
        .map(|loop_points| LineString::from(place(loop_points)))
        .collect();
    GeoPolygon::new(LineString::from(place(base)), interiors)
}

fn rect_polygon(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> GeoPolygon<f64> {
    GeoPolygon::new(
        LineString::from(vec![
            (min_x, min_y),
            (max_x, min_y),
            (max_x, max_y),
            (min_x, max_y),
            (min_x, min_y),
        ]),
        vec![],
    )
}

/// `parse_xy`: "WxH" strings → (x, y) pairs.
fn parse_xy(values: &[Value]) -> Result<Vec<(f64, f64)>, Failure> {
    values
        .iter()
        .map(|value| {
            let text = value.as_str().ok_or_else(|| {
                Failure::Error(format!("printable area entry {value} is not a string"))
            })?;
            let mut parts = text.split('x');
            let mut coordinate = |name: &str| {
                parts.next().ok_or_else(|| {
                    Failure::Error(format!("printable area entry {text:?} has no {name} part"))
                })
            };
            let x = coordinate("x")?
                .parse::<f64>()
                .map_err(|error| Failure::Error(format!("printable area entry {text:?}: {error}")))?;
            let y = coordinate("y")?
                .parse::<f64>()
                .map_err(|error| Failure::Error(format!("printable area entry {text:?}: {error}")))?;
            Ok((x, y))
        })
        .collect()
}

fn bounds(points: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (x, y) in points {
        min_x = min_x.min(*x);
        min_y = min_y.min(*y);
        max_x = max_x.max(*x);
        max_y = max_y.max(*y);
    }
    (min_x, min_y, max_x, max_y)
}

fn closed_ring(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut ring = points.to_vec();
    if ring.first() != ring.last()
        && let Some(first) = ring.first().copied()
    {
        ring.push(first);
    }
    ring
}

/// Python's json.dumps happily emits `Infinity`; strict JSON (and serde_json) cannot, so an
/// empty layout's infinite gap serializes as null instead.
fn json_number(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// Floats in assertion messages formatted like Python's str(float) (integral values keep ".0").
fn py_f64(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e16 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}

fn read_entry(archive: &mut ZipArchive<fs::File>, name: &str) -> Result<String, Failure> {
    let mut entry = archive
        .by_name(name)
        .map_err(|error| Failure::Error(format!("archive is missing {name}: {error}")))?;
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|error| Failure::Error(format!("failed to read {name}: {error}")))?;
    Ok(text)
}

/// XY bbox center over ALL STL vertices — the `stl_bbox_center` the Python feeds the item
/// translation expectation. Binary STL only: the collision STL is always produced by the
/// packer in binary form (80 zero bytes, LE u32 count, 50-byte records).
fn stl_xy_center(path: &Path) -> Result<(f64, f64), Failure> {
    let data = fs::read(path)
        .map_err(|error| Failure::Error(format!("failed to read STL {}: {error}", path.display())))?;
    if data.len() < 84 {
        return Err(Failure::Error("STL header is truncated".to_owned()));
    }
    let triangle_count = u32::from_le_bytes(data[80..84].try_into().expect("four bytes")) as usize;
    let expected_len = 84usize
        + triangle_count
            .checked_mul(50)
            .ok_or_else(|| Failure::Error("STL triangle count overflows".to_owned()))?;
    if data.len() != expected_len {
        return Err(Failure::Error(format!(
            "binary STL has {} bytes; expected {expected_len}",
            data.len()
        )));
    }
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for index in 0..triangle_count {
        // 12-byte facet normal header precedes the three vertices in each record.
        let base = 84 + index * 50 + 12;
        for vertex in 0..3 {
            let offset = base + vertex * 12;
            let x = f64::from(f32::from_le_bytes(
                data[offset..offset + 4].try_into().expect("four bytes"),
            ));
            let y = f64::from(f32::from_le_bytes(
                data[offset + 4..offset + 8].try_into().expect("four bytes"),
            ));
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    Ok(((min_x + max_x) * 0.5, (min_y + max_y) * 0.5))
}

#[derive(Debug, Default)]
struct ParsedItem {
    /// Core `objectid`; a missing attribute becomes `None` (and never matches a resource id).
    object_id: Option<String>,
    transform: Option<String>,
}

#[derive(Debug, Default)]
struct ParsedProjectModel {
    /// `<object>` element count under `<resources>` (duplicates counted, like ElementTree).
    resource_objects: usize,
    /// Core `id` of every resource object; a missing id maps to "" which never matches.
    resource_ids: HashSet<String>,
    items: Vec<ParsedItem>,
    /// Production `p:path` per `<component>` in document order (None when the attr is absent).
    component_paths: Vec<Option<String>>,
    /// Transform of the first component nested under resources → object → components.
    first_resource_component_transform: Option<String>,
}

fn parse_project_model(xml: &str) -> Result<ParsedProjectModel, Failure> {
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut parsed = ParsedProjectModel::default();
    // (namespace URI, local name) ancestry stack: the ElementTree `.//a/b` paths are matched
    // against it by namespace URI + parent instead of by prefix spelling.
    let mut stack: Vec<(String, String)> = Vec::new();
    let mut seen_resource_component = false;
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|error| {
            Failure::Error(format!("failed to parse 3D/3dmodel.model: {error}"))
        })?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let empty = matches!(event, Event::Empty(_));
                let ns_uri = namespace_str(&namespace)?;
                let local = start.name().local_name().into_inner().to_owned();
                let parent = stack.last().map(|(ns, name)| (ns.as_str(), name.as_str()));
                let grandparent = stack
                    .len()
                    .checked_sub(2)
                    .and_then(|index| stack.get(index))
                    .map(|(ns, name)| (ns.as_str(), name.as_str()));
                if ns_uri == CORE && local == "object" && parent == Some((CORE, "resources")) {
                    parsed.resource_objects += 1;
                    parsed
                        .resource_ids
                        .insert(attr(start, reader.resolver(), None, "id").unwrap_or_default());
                } else if ns_uri == CORE && local == "item" && parent == Some((CORE, "build")) {
                    parsed.items.push(ParsedItem {
                        object_id: attr(start, reader.resolver(), None, "objectid"),
                        transform: attr(start, reader.resolver(), None, "transform"),
                    });
                } else if ns_uri == CORE && local == "component" {
                    parsed
                        .component_paths
                        .push(attr(start, reader.resolver(), Some(PRODUCTION), "path"));
                    // root.find(".//c:resources/c:object/c:components/c:component"): the first
                    // component nested under resources → object → components, document order.
                    if !seen_resource_component
                        && parent == Some((CORE, "components"))
                        && grandparent == Some((CORE, "object"))
                        && stack.iter().any(|(ns, name)| ns == CORE && name == "resources")
                    {
                        seen_resource_component = true;
                        parsed.first_resource_component_transform =
                            attr(start, reader.resolver(), None, "transform");
                    }
                }
                stack.push((ns_uri, local));
                if empty {
                    stack.pop();
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(parsed)
}

fn parse_mesh_vertices(xml: &str) -> Result<Vec<(f64, f64, f64)>, Failure> {
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut vertices = Vec::new();
    loop {
        let (namespace, event) = reader
            .read_resolved_event()
            .map_err(|error| Failure::Error(format!("failed to parse component mesh: {error}")))?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let ns_uri = namespace_str(&namespace)?;
                let local = start.name().local_name().into_inner();
                if ns_uri == CORE && local == "vertex" {
                    let number = |name: &str| -> Result<f64, Failure> {
                        let raw = attr(start, reader.resolver(), None, name).ok_or_else(|| {
                            Failure::Error(format!("mesh vertex is missing the {name} attribute"))
                        })?;
                        raw.parse::<f64>().map_err(|error| {
                            Failure::Error(format!("mesh vertex {name}=\"{raw}\" is not a number: {error}"))
                        })
                    };
                    vertices.push((number("x")?, number("y")?, number("z")?));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(vertices)
}

/// Returns (object count, plate/model_instance count) — ElementTree's `findall("object")` and
/// `findall("plate/model_instance")` on the namespace-free model_settings.config.
fn parse_model_settings(xml: &str) -> Result<(usize, usize), Failure> {
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut objects = 0;
    let mut instances = 0;
    let mut stack: Vec<(String, String)> = Vec::new();
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|error| {
            Failure::Error(format!("failed to parse Metadata/model_settings.config: {error}"))
        })?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let empty = matches!(event, Event::Empty(_));
                let ns_uri = namespace_str(&namespace)?;
                let local = start.name().local_name().into_inner().to_owned();
                let parent = stack.last().map(|(ns, name)| (ns.as_str(), name.as_str()));
                // Root-level <object> (depth 1) and <model_instance> under a root-level
                // <plate> (depth 2): stack still holds only ancestors while we process.
                if ns_uri.is_empty() && local == "object" && stack.len() == 1 {
                    objects += 1;
                }
                if ns_uri.is_empty()
                    && local == "model_instance"
                    && stack.len() == 2
                    && parent == Some(("", "plate"))
                {
                    instances += 1;
                }
                stack.push((ns_uri, local));
                if empty {
                    stack.pop();
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((objects, instances))
}

/// Attribute lookup by (namespace URI, local name) — the ElementTree `{uri}local` spelling.
/// `None` means an unprefixed (no-namespace) attribute.
fn attr(
    start: &BytesStart,
    resolver: &NamespaceResolver,
    uri: Option<&str>,
    local: &str,
) -> Option<String> {
    start
        .attributes()
        .with_checks(false)
        .filter_map(Result::ok)
        .find(|attribute| {
            let (attribute_ns, attribute_local) = resolver.resolve_attribute(attribute.key);
            let namespace_ok = match (uri, &attribute_ns) {
                (Some(uri), ResolveResult::Bound(bound)) => bound.0 == uri,
                (None, ResolveResult::Unbound) => true,
                _ => false,
            };
            namespace_ok && attribute_local.into_inner() == local
        })
        .map(|attribute| attribute.value.into_owned())
}

fn namespace_str(namespace: &ResolveResult) -> Result<String, Failure> {
    Ok(match namespace {
        ResolveResult::Bound(bound) => bound.0.to_owned(),
        ResolveResult::Unbound => String::new(),
        ResolveResult::Unknown(prefix) => {
            return Err(Failure::Error(format!("undeclared namespace prefix: {prefix}")));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::TempDir;
    use zip::{ZipWriter, write::SimpleFileOptions};

    struct Fixture {
        stl: PathBuf,
        placements: PathBuf,
        project: PathBuf,
    }

    const PROJECT_SETTINGS: &str = r#"{"printable_area": ["0x0", "200x0", "200x200", "0x200"], "printer_model": "P1S"}"#;

    const MODEL_SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<settings>
 <object id="2"/>
 <plate>
  <model_instance/>
 </plate>
</settings>
"#;

    const MESH_MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
 <resources>
  <object id="1" type="model">
   <mesh>
    <vertices>
     <vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/>
     <vertex x="10" y="10" z="0"/><vertex x="0" y="10" z="0"/>
    </vertices>
    <triangles>
     <triangle v1="0" v2="1" v3="2"/><triangle v1="0" v2="2" v3="3"/>
    </triangles>
   </mesh>
  </object>
 </resources>
 <build/>
</model>
"#;

    /// Binary STL of the baked square (10,20)..(20,30) at z=-0.5: 80 zero bytes, LE u32
    /// count, 50-byte records.
    fn baked_square_stl() -> Vec<u8> {
        let vertex = |x: f64, y: f64| [x as f32, y as f32, -0.5f32];
        let triangles = [
            [vertex(10.0, 20.0), vertex(20.0, 20.0), vertex(20.0, 30.0)],
            [vertex(10.0, 20.0), vertex(20.0, 30.0), vertex(10.0, 30.0)],
        ];
        let mut out = vec![0u8; 80];
        out.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
        for triangle in triangles {
            out.extend_from_slice(&[0u8; 12]);
            for v in triangle {
                for coordinate in v {
                    out.extend_from_slice(&coordinate.to_le_bytes());
                }
            }
            out.extend_from_slice(&0u16.to_le_bytes());
        }
        out
    }

    /// build_3mf semantics: translation = placement − R(angle)·c_stl with c_stl = (15, 25), the
    /// XY bbox center of the baked square; the +0.5 z lifts the component-baked mesh (flat at
    /// z=-0.5) onto the bed.
    fn item_transform(x_mm: f64, y_mm: f64, angle_deg: f64) -> String {
        let radians = angle_deg.to_radians();
        let (sin, cos) = radians.sin_cos();
        let (center_x, center_y) = (15.0, 25.0);
        let tx = x_mm - (center_x * cos - center_y * sin);
        let ty = y_mm - (center_x * sin + center_y * cos);
        format!("{cos} {sin} 0 {} {cos} 0 0 0 1 {tx} {ty} 0.5", -sin)
    }

    fn core_model(items: &[String]) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06">
 <resources>
  <object id="2" type="model">
   <components>
    <component p:path="/3D/Objects/mesh.model" transform="1 0 0 0 1 0 0 0 1 10 20 -0.5"/>
   </components>
  </object>
 </resources>
 <build>
{}
 </build>
</model>
"#,
            items.join("\n")
        )
    }

    fn write_fixture(
        dir: &TempDir,
        placements: &[(f64, f64, f64)],
        tweak: impl Fn(usize, String) -> String,
    ) -> Fixture {
        let project = dir.path().join("project.3mf");
        let file = fs::File::create(&project).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        let items: Vec<String> = placements
            .iter()
            .enumerate()
            .map(|(index, (x_mm, y_mm, angle_deg))| {
                let transform = tweak(index, item_transform(*x_mm, *y_mm, *angle_deg));
                format!(r#"  <item objectid="2" printable="1" transform="{transform}"/>"#)
            })
            .collect();
        let mut add = |name: &str, data: &str| {
            zip.start_file(name, options).unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        };
        add("3D/3dmodel.model", &core_model(&items));
        add("3D/Objects/mesh.model", MESH_MODEL);
        add("Metadata/project_settings.config", PROJECT_SETTINGS);
        add("Metadata/model_settings.config", MODEL_SETTINGS);
        zip.finish().unwrap();

        let stl = dir.path().join("collision.stl");
        fs::write(&stl, baked_square_stl()).unwrap();
        let placements_path = dir.path().join("placements.json");
        let placement_json = format!(
            "{{\"placements\": [{}]}}",
            placements
                .iter()
                .map(|(x_mm, y_mm, angle_deg)| format!(
                    "{{\"x_mm\": {x_mm}, \"y_mm\": {y_mm}, \"angle_deg\": {angle_deg}}}"
                ))
                .collect::<Vec<_>>()
                .join(", ")
        );
        fs::write(&placements_path, placement_json).unwrap();
        Fixture {
            stl,
            placements: placements_path,
            project,
        }
    }

    fn run(fixture: &Fixture, minimum_gap: f64) -> Result<Report, Failure> {
        verify(&Args {
            stl: fixture.stl.clone(),
            placements: fixture.placements.clone(),
            project: fixture.project.clone(),
            minimum_gap,
            edge_margin: 0.0,
            purge_lines: Vec::new(),
            purge_gap: 0.5,
            custom_keepouts: Vec::new(),
            report: None,
        })
    }

    #[test]
    fn good_two_placements_pass() {
        let dir = TempDir::new().unwrap();
        let fixture = write_fixture(&dir, &[(60.0, 70.0, 0.0), (150.0, 160.0, 90.0)], |_, t| t);
        let report = run(&fixture, 0.5).unwrap();
        assert_eq!(report.count, 2);
        assert!(report.all_inside_printable_area);
        assert!(report.exclusion_clear);
        assert!(report.zip_crc_ok);
        assert!(report.metadata_object_counts_match);
        assert!(report.embedded_mesh_transform_ok);
        // Squares [55,65]x[65,75] and [145,155]x[155,165]: corner-to-corner distance.
        let gap = report.minimum_exact_gap_mm.as_f64().unwrap();
        assert!((gap - 80.0f64.hypot(80.0)).abs() < 1e-9, "{gap}");
        // Clearances: 55 for the first square, 35 (200 - 165) for the second.
        assert!((report.minimum_edge_clearance_mm - 35.0).abs() < 1e-12);
        assert!(report.required_purge_gap_mm.is_null());
        assert!(report.minimum_purge_gap_mm.is_null());
        assert_eq!(report.printer_model.as_deref(), Some("P1S"));
        assert_eq!(report.project, fixture.project.display().to_string());
    }

    #[test]
    fn mutated_item_translation_fails() {
        let dir = TempDir::new().unwrap();
        let fixture = write_fixture(&dir, &[(60.0, 70.0, 0.0), (150.0, 160.0, 90.0)], |index, transform| {
            // item 0 at (60, 70, 0°) has translation (45, 45); shift x by +1 mm.
            if index == 0 {
                transform.replacen(" 45 45 ", " 46 45 ", 1)
            } else {
                transform
            }
        });
        let Failure::Assertion(message) = run(&fixture, 0.5).unwrap_err() else {
            panic!("expected an assertion failure");
        };
        assert!(message.contains("item 0 XY translation mismatch"), "{message}");
    }

    #[test]
    fn placement_outside_printable_fails() {
        let dir = TempDir::new().unwrap();
        let fixture = write_fixture(&dir, &[(203.0, 70.0, 0.0), (150.0, 160.0, 90.0)], |_, t| t);
        let Failure::Assertion(message) = run(&fixture, 0.5).unwrap_err() else {
            panic!("expected an assertion failure");
        };
        assert!(
            message.contains("placement 0 violates 0.000 mm printable-area margin"),
            "{message}"
        );
    }

    #[test]
    fn too_tight_gap_fails() {
        let dir = TempDir::new().unwrap();
        // 0.3 mm apart: the squares overlap, so the exact gap is 0 (< 0.5 required).
        let fixture = write_fixture(&dir, &[(60.0, 70.0, 0.0), (60.0, 70.3, 0.0)], |_, t| t);
        let Failure::Assertion(message) = run(&fixture, 0.5).unwrap_err() else {
            panic!("expected an assertion failure");
        };
        assert!(
            message.contains("minimum exact gap 0.000000 mm is below 0.500000 mm"),
            "{message}"
        );
    }

    #[test]
    fn cli_accepts_both_flag_spellings() {
        let parsed = parse_args_from(
            [
                "stl.stl",
                "placements.json",
                "project.3mf",
                "--minimum-gap",
                "0.25",
                "--edge-margin=2",
                "--purge-line",
                "0,0,200,0,4",
                "--purge-line=-10,5,300,5,8",
                "--purge-gap=1",
                "--keepout",
                "10,10,20,20",
                "--report=out.json",
            ]
            .into_iter()
            .map(String::from),
        )
        .unwrap();
        assert_eq!(parsed.stl, PathBuf::from("stl.stl"));
        assert_eq!(parsed.placements, PathBuf::from("placements.json"));
        assert_eq!(parsed.project, PathBuf::from("project.3mf"));
        assert_eq!(parsed.minimum_gap, 0.25);
        assert_eq!(parsed.edge_margin, 2.0);
        assert_eq!(parsed.purge_lines, vec![[0.0, 0.0, 200.0, 0.0, 4.0], [-10.0, 5.0, 300.0, 5.0, 8.0]]);
        assert_eq!(parsed.purge_gap, 1.0);
        assert_eq!(parsed.custom_keepouts, vec![Rect::new(10.0, 10.0, 20.0, 20.0)]);
        assert_eq!(parsed.report, Some(PathBuf::from("out.json")));
    }
}

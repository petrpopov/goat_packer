//! The checks themselves, in the order they run. Each returns the first `Failure` it finds.

use std::{collections::HashSet, fs, io};

use geo::LineString;
use geo::Polygon as GeoPolygon;
use geo::algorithm::area::Area;
use geo::algorithm::bool_ops::BooleanOps;
use geo::algorithm::covers::Covers;
use geo::algorithm::line_measures::{Distance, Euclidean};
use goat_packer::{Footprint, Point, axis_aligned_line_keepout, extract_footprint};
use serde_json::Value;
use zip::ZipArchive;

use crate::args::Args;
use crate::matrix::{apply_matrix, parse_transform};
use crate::project::{
    ParsedProjectModel, parse_mesh_vertices, parse_model_settings, parse_project_model, parse_xy,
    read_entry,
};
use crate::report::{Report, json_number, py_f64};
use crate::rules::{self, Foreign, Kind, Rule, Zone};
use crate::stl::stl_xy_center;
use crate::{Failure, assertion};

/// Float slack for the printable-area containment check only (see `BedLimits::new`).
const EDGE_TOLERANCE_MM: f64 = 1e-6;
/// Intersections smaller than this area (mm²) are float noise, not an object in a forbidden zone.
const MIN_INTRUSION_AREA_MM2: f64 = 1e-7;
/// Slack of the gap checks, mm: the exact distance may fall a hair under the required one.
const GAP_TOLERANCE_MM: f64 = 1e-6;
/// An item's rotation may differ from the placement's by this many degrees.
const ANGLE_TOLERANCE_DEG: f64 = 1e-6;
/// An item's translation may differ from the compensated placement by this many mm.
const TRANSLATION_TOLERANCE_MM: f64 = 1e-6;
/// A mesh sits on the bed when its lowest vertex is within this many mm of z = 0 ...
const BED_Z_TOLERANCE_MM: f64 = 1e-5;
/// ... and its height is the embedded mesh's within this many mm.
const HEIGHT_TOLERANCE_MM: f64 = 1e-4;

type Placement = (f64, f64, f64);

pub fn verify(args: &Args) -> Result<Report, Failure> {
    let placements = read_placements(args)?;

    // extract_footprint returns the min-Z bottom outline (exterior loop first, holes after),
    // already centered on the STL's XY bbox center — the geometry the Python rebuilt from the
    // bottom triangles with lower_polygon().
    let footprint = extract_footprint(&args.stl).map_err(|error| {
        Failure::Error(format!(
            "failed to read STL {}: {error}",
            args.stl.display()
        ))
    })?;
    let polygons = world_polygons(&footprint, &placements);

    let mut archive = open_project(args)?;
    check_zip_crc(&mut archive)?;
    let settings: Value = serde_json::from_str(&read_entry(
        &mut archive,
        "Metadata/project_settings.config",
    )?)
    .map_err(|error| {
        Failure::Error(format!(
            "failed to parse Metadata/project_settings.config: {error}"
        ))
    })?;
    let limits = BedLimits::new(&settings, args.edge_margin)?;

    let model = parse_project_model(&read_entry(&mut archive, "3D/3dmodel.model")?)?;
    let (config_objects, instances) =
        parse_model_settings(&read_entry(&mut archive, "Metadata/model_settings.config")?)?;
    check_object_counts(&model, placements.len(), config_objects, instances)?;
    let local_vertices = component_vertices(&mut archive, &model)?;
    check_item_transforms(
        &model,
        &placements,
        &local_vertices,
        stl_xy_center(&args.stl)?,
    )?;

    let edge_clearances = check_inside_bed(&polygons, &limits, args)?;
    let minimum_distance = check_object_gaps(&polygons, body_gap(args.minimum_gap, args.foreign))?;
    let purge = check_purge_gap(&polygons, args)?;

    Ok(Report {
        project: args.project.display().to_string(),
        printer_model: settings
            .get("printer_model")
            .and_then(Value::as_str)
            .map(str::to_owned),
        count: placements.len(),
        minimum_exact_gap_mm: json_number(minimum_distance),
        required_edge_margin_mm: args.edge_margin,
        minimum_edge_clearance_mm: edge_clearances
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min),
        required_purge_gap_mm: purge.map_or(Value::Null, |_| json_number(args.purge_gap)),
        minimum_purge_gap_mm: purge.map_or(Value::Null, json_number),
        all_inside_printable_area: true,
        exclusion_clear: true,
        zip_crc_ok: true,
        metadata_object_counts_match: true,
        embedded_mesh_transform_ok: true,
    })
}

/// `(x_mm, y_mm, angle_deg)` of every placement in the placements JSON.
fn read_placements(args: &Args) -> Result<Vec<Placement>, Failure> {
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
    placement_values
        .iter()
        .map(|placement| {
            let field = |name: &str| {
                placement.get(name).and_then(Value::as_f64).ok_or_else(|| {
                    Failure::Error(format!("placement is missing a numeric \"{name}\""))
                })
            };
            Ok((field("x_mm")?, field("y_mm")?, field("angle_deg")?))
        })
        .collect()
}

fn world_polygons(footprint: &Footprint, placements: &[Placement]) -> Vec<GeoPolygon<f64>> {
    // loops is never empty when extract_footprint succeeded (a boundary implies a loop).
    let base = footprint
        .loops
        .first()
        .expect("footprint has an exterior loop");
    placements
        .iter()
        .map(|(x_mm, y_mm, angle_deg)| {
            world_polygon(&footprint.loops, base, *angle_deg, *x_mm, *y_mm)
        })
        .collect()
}

fn open_project(args: &Args) -> Result<ZipArchive<fs::File>, Failure> {
    ZipArchive::new(fs::File::open(&args.project).map_err(|error| {
        Failure::Error(format!(
            "failed to open project {}: {error}",
            args.project.display()
        ))
    })?)
    .map_err(|error| Failure::Error(format!("failed to read project zip: {error}")))
}

/// Reading every entry to EOF makes the zip crate verify each member's CRC32 — the testzip()
/// equivalent.
fn check_zip_crc(archive: &mut ZipArchive<fs::File>) -> Result<(), Failure> {
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            Failure::Error(format!("failed to read archive entry {index}: {error}"))
        })?;
        let name = entry.name().to_owned();
        io::copy(&mut entry, &mut io::sink())
            .map_err(|_| assertion(format!("ZIP CRC failed: {name}")))?;
    }
    Ok(())
}

/// Where objects may be: the printable area shrunk by the edge margin, its boundary, and the
/// exclusion polygon.
struct BedLimits {
    safe_printable: GeoPolygon<f64>,
    printable_boundary: LineString<f64>,
    exclusion: Option<GeoPolygon<f64>>,
}

impl BedLimits {
    fn new(settings: &Value, edge_margin: f64) -> Result<Self, Failure> {
        let printable_points = parse_xy(
            settings
                .get("printable_area")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    Failure::Error("\"printable_area\" missing from project_settings".to_owned())
                })?,
        )?;
        let printable_bounds = bounds(&printable_points);
        // shapely's buffer(-margin, join_style=2) on the (always rectangular) printable area is
        // exactly this rectangle inset.
        let safe_min_x = printable_bounds.0 + edge_margin;
        let safe_min_y = printable_bounds.1 + edge_margin;
        let safe_max_x = printable_bounds.2 - edge_margin;
        let safe_max_y = printable_bounds.3 - edge_margin;
        if safe_min_x >= safe_max_x || safe_min_y >= safe_max_y {
            return Err(assertion(format!(
                "edge margin {} mm consumes printable area",
                py_f64(edge_margin)
            )));
        }
        // The containment test gets EDGE_TOLERANCE_MM of slack: an object whose flat side sits
        // exactly on the margin line comes out a few ulps outside after rotation (cos 270° ≈
        // -1.8e-16; spike 2026-11-01 reported clearance 0.4999999999999982 against a 0.5 margin)
        // and a strict `covers` rejected a correct layout. 1e-6 mm is far below any printable
        // distance, so a real violation still fails.
        let safe_printable = rect_polygon(
            safe_min_x - EDGE_TOLERANCE_MM,
            safe_min_y - EDGE_TOLERANCE_MM,
            safe_max_x + EDGE_TOLERANCE_MM,
            safe_max_y + EDGE_TOLERANCE_MM,
        );
        let printable_boundary = LineString::from(closed_ring(&printable_points));
        let exclusion = settings
            .get("bed_exclude_area")
            .and_then(Value::as_array)
            .filter(|values| values.len() >= 3)
            .map(|values| {
                parse_xy(values)
                    .map(|points| GeoPolygon::new(LineString::from(closed_ring(&points)), vec![]))
            })
            .transpose()?;
        Ok(Self {
            safe_printable,
            printable_boundary,
            exclusion,
        })
    }
}

/// Resources, build items, settings objects and plate instances agree with the placements.
fn check_object_counts(
    model: &ParsedProjectModel,
    placement_count: usize,
    config_objects: usize,
    instances: usize,
) -> Result<(), Failure> {
    if model.items.len() != placement_count {
        return Err(assertion(format!(
            "object-count mismatch: placements={}, resources={}, items={}, config={}, instances={}",
            placement_count,
            model.resource_objects,
            model.items.len(),
            config_objects,
            instances
        )));
    }
    if ![1, placement_count].contains(&model.resource_objects)
        || ![1, placement_count].contains(&config_objects)
        || ![1, placement_count].contains(&instances)
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
    Ok(())
}

/// The embedded mesh's vertices through the component's own transform. That transform sits
/// between the embedded mesh and every build item (world = mesh ∘ component ∘ item). It can
/// carry a translation (Goat 3: (-35.38, +40.12, -4.3)); ignoring it makes both the XY and Z
/// placement checks lie.
fn component_vertices(
    archive: &mut ZipArchive<fs::File>,
    model: &ParsedProjectModel,
) -> Result<Vec<(f64, f64, f64)>, Failure> {
    let archive_names: HashSet<String> = archive.file_names().map(str::to_owned).collect();
    let mut component_paths = Vec::new();
    for path in &model.component_paths {
        let Some(path) = path else {
            return Err(Failure::Error(
                "component without a p:path attribute".to_owned(),
            ));
        };
        component_paths.push(path.trim_start_matches('/').to_owned());
    }
    if component_paths
        .iter()
        .any(|path| !archive_names.contains(path))
    {
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
    let embedded_vertices = parse_mesh_vertices(&read_entry(archive, first_path)?)?;
    if embedded_vertices.is_empty() {
        return Err(assertion(
            "embedded component mesh has no vertices".to_owned(),
        ));
    }
    let component_matrix = parse_transform(model.first_resource_component_transform.as_deref())?;
    Ok(embedded_vertices
        .iter()
        .map(|vertex| apply_matrix(&component_matrix, *vertex))
        .collect())
}

/// Every build item has the placement's angle, the compensated translation, and stands on
/// the bed.
fn check_item_transforms(
    model: &ParsedProjectModel,
    placements: &[Placement],
    local_vertices: &[(f64, f64, f64)],
    stl_center: (f64, f64),
) -> Result<(), Failure> {
    let local_min_z = local_vertices
        .iter()
        .map(|vertex| vertex.2)
        .fold(f64::INFINITY, f64::min);
    let embedded_height = local_vertices
        .iter()
        .map(|vertex| vertex.2)
        .fold(f64::NEG_INFINITY, f64::max)
        - local_min_z;
    for (index, item) in model.items.iter().enumerate() {
        let Some(transform) = item.transform.as_deref() else {
            return Err(assertion(format!("item {index} transform is not 3x4")));
        };
        let values = transform
            .split_whitespace()
            .map(|value| {
                value.parse::<f64>().map_err(|error| {
                    Failure::Error(format!(
                        "item {index} transform has a non-numeric value: {error}"
                    ))
                })
            })
            .collect::<Result<Vec<f64>, Failure>>()?;
        if values.len() != 12 {
            return Err(assertion(format!("item {index} transform is not 3x4")));
        }
        check_item_pose(index, &values, &placements[index], stl_center)?;
        check_item_on_bed(
            index,
            &parse_transform(item.transform.as_deref())?,
            local_vertices,
            embedded_height,
        )?;
    }
    Ok(())
}

fn check_item_pose(
    index: usize,
    values: &[f64],
    expected: &Placement,
    stl_center: (f64, f64),
) -> Result<(), Failure> {
    let angle = values[1].atan2(values[0]).to_degrees().rem_euclid(360.0);
    let expected_angle = expected.2;
    if (((angle - expected_angle + 180.0).rem_euclid(360.0)) - 180.0).abs() > ANGLE_TOLERANCE_DEG {
        return Err(assertion(format!(
            "item {index} angle mismatch: {} != {}",
            py_f64(angle),
            py_f64(expected_angle)
        )));
    }
    // The item translation places the object-local footprint center (embedded mesh plus the
    // source component's transform) at (x_mm, y_mm), so the expected translation is the
    // placement minus the angle-rotated STL bbox center — the same compensation build_3mf.py
    // applies. Comparing raw matrix[9]/[10] to the placement would only hold for a component
    // whose own translation is zero.
    let radians = expected_angle.to_radians();
    let (center_x, center_y) = stl_center;
    let expected_tx = expected.0 - (center_x * radians.cos() - center_y * radians.sin());
    let expected_ty = expected.1 - (center_x * radians.sin() + center_y * radians.cos());
    if (values[9] - expected_tx).abs() > TRANSLATION_TOLERANCE_MM
        || (values[10] - expected_ty).abs() > TRANSLATION_TOLERANCE_MM
    {
        return Err(assertion(format!(
            "item {index} XY translation mismatch: ({}, {}) != ({}, {})",
            py_f64(values[9]),
            py_f64(values[10]),
            py_f64(expected_tx),
            py_f64(expected_ty)
        )));
    }
    Ok(())
}

fn check_item_on_bed(
    index: usize,
    item_matrix: &crate::matrix::Matrix,
    local_vertices: &[(f64, f64, f64)],
    embedded_height: f64,
) -> Result<(), Failure> {
    let mut transformed_min_z = f64::INFINITY;
    let mut transformed_max_z = f64::NEG_INFINITY;
    for vertex in local_vertices {
        let z = apply_matrix(item_matrix, *vertex).2;
        transformed_min_z = transformed_min_z.min(z);
        transformed_max_z = transformed_max_z.max(z);
    }
    if transformed_min_z.abs() > BED_Z_TOLERANCE_MM
        || (transformed_max_z - embedded_height).abs() > HEIGHT_TOLERANCE_MM
    {
        return Err(assertion(format!(
            "item {index} is not on bed: z={}..{}",
            py_f64(transformed_min_z),
            py_f64(transformed_max_z)
        )));
    }
    Ok(())
}

/// Every object inside the safe printable area, clear of the exclusion polygon and the custom
/// keepouts; returns each object's distance to the printable boundary.
fn check_inside_bed(
    polygons: &[GeoPolygon<f64>],
    limits: &BedLimits,
    args: &Args,
) -> Result<Vec<f64>, Failure> {
    let mut edge_clearances = Vec::new();
    for (index, polygon) in polygons.iter().enumerate() {
        if !limits.safe_printable.covers(polygon) {
            return Err(assertion(format!(
                "placement {index} violates {:.3} mm printable-area margin",
                args.edge_margin
            )));
        }
        edge_clearances.push(Euclidean.distance(polygon, &limits.printable_boundary));
        if let Some(exclusion) = &limits.exclusion
            && enters_hard_zone(polygon, exclusion, args.minimum_gap, args.foreign)
        {
            return Err(assertion(format!(
                "placement {index} enters exclusion polygon"
            )));
        }
        for keepout in &args.custom_keepouts {
            let keepout_polygon =
                rect_polygon(keepout.min_x, keepout.min_y, keepout.max_x, keepout.max_y);
            if enters_hard_zone(polygon, &keepout_polygon, args.minimum_gap, args.foreign) {
                return Err(assertion(format!(
                    "placement {index} enters custom keepout"
                )));
            }
        }
    }
    Ok(edge_clearances)
}

/// The gap two objects' outlines keep: the table's body/body pair, i.e. the clearance.
fn body_gap(clearance: f64, foreign: Foreign) -> f64 {
    rules::pair_gap(Kind::Silhouette, Kind::Silhouette, false)
        .map_or(0.0, |gap| gap.mm(clearance, foreign))
}

/// An outline against a hard zone (the bed exclusion, a custom keepout) by the table's body
/// row. This binary has the bottom outline only, not per-part hulls, so the hull row is
/// tested on the outline itself: touching passes, an overlap fails.
fn enters_hard_zone(
    outline: &GeoPolygon<f64>,
    zone: &GeoPolygon<f64>,
    clearance: f64,
    foreign: Foreign,
) -> bool {
    let overlaps = || outline.intersection(zone).unsigned_area() > MIN_INTRUSION_AREA_MM2;
    match rules::rule(Kind::Silhouette, Zone::Hard) {
        Rule::Allowed => false,
        Rule::PartHull | Rule::PartBox => overlaps(),
        Rule::Forbidden(gap) => {
            overlaps() || Euclidean.distance(outline, zone) + GAP_TOLERANCE_MM < gap.mm(clearance, foreign)
        }
    }
}

/// The smallest exact distance between two objects, which must reach `minimum_gap`.
fn check_object_gaps(polygons: &[GeoPolygon<f64>], minimum_gap: f64) -> Result<f64, Failure> {
    let mut minimum_distance = f64::INFINITY;
    for (i, polygon) in polygons.iter().enumerate() {
        for previous in polygons.iter().take(i) {
            minimum_distance = minimum_distance.min(Euclidean.distance(polygon, previous));
        }
    }
    if minimum_distance + GAP_TOLERANCE_MM < minimum_gap {
        return Err(assertion(format!(
            "minimum exact gap {:.6} mm is below {:.6} mm",
            minimum_distance, minimum_gap
        )));
    }
    Ok(minimum_distance)
}

/// The smallest distance from any object to a purge line, which must reach `--purge-gap`;
/// `None` without purge lines.
fn check_purge_gap(polygons: &[GeoPolygon<f64>], args: &Args) -> Result<Option<f64>, Failure> {
    // The shapely buffer(cap_style=2, join_style=2) of an axis-aligned segment is this plain
    // rectangle (never buffer-discretized arcs).
    let mut purge_polygons = Vec::new();
    for [x0, y0, x1, y1, width] in &args.purge_lines {
        let rect = axis_aligned_line_keepout(*x0, *y0, *x1, *y1, *width).map_err(Failure::Error)?;
        purge_polygons.push(rect_polygon(rect.min_x, rect.min_y, rect.max_x, rect.max_y));
    }
    let mut minimum_purge_distance = f64::INFINITY;
    for polygon in polygons {
        for purge in &purge_polygons {
            minimum_purge_distance = minimum_purge_distance.min(Euclidean.distance(polygon, purge));
        }
    }
    if minimum_purge_distance + GAP_TOLERANCE_MM < args.purge_gap {
        return Err(assertion(format!(
            "minimum purge-line gap {:.6} mm is below {:.6} mm",
            minimum_purge_distance, args.purge_gap
        )));
    }
    Ok((!purge_polygons.is_empty()).then_some(minimum_purge_distance))
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
            .map(|point| {
                (
                    point.x * cos - point.y * sin + x_mm,
                    point.x * sin + point.y * cos + y_mm,
                )
            })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write as _, path::PathBuf};
    use tempfile::TempDir;
    use zip::{ZipWriter, write::SimpleFileOptions};

    struct Fixture {
        stl: PathBuf,
        placements: PathBuf,
        project: PathBuf,
    }

    const PROJECT_SETTINGS: &str =
        r#"{"printable_area": ["0x0", "200x0", "200x200", "0x200"], "printer_model": "P1S"}"#;

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
            foreign: Foreign::default(),
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
        let fixture = write_fixture(
            &dir,
            &[(60.0, 70.0, 0.0), (150.0, 160.0, 90.0)],
            |index, transform| {
                // item 0 at (60, 70, 0°) has translation (45, 45); shift x by +1 mm.
                if index == 0 {
                    transform.replacen(" 45 45 ", " 46 45 ", 1)
                } else {
                    transform
                }
            },
        );
        let Failure::Assertion(message) = run(&fixture, 0.5).unwrap_err() else {
            panic!("expected an assertion failure");
        };
        assert!(
            message.contains("item 0 XY translation mismatch"),
            "{message}"
        );
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
    fn placement_touching_the_bed_edge_passes_despite_float_noise() {
        // Spike 2026-11-01: a flat side exactly on the bed edge at 270° comes out at
        // x ≈ -1e-15 (cos 270° ≈ -1.8e-16), which a strict `covers` rejected as a margin
        // violation. The square is 10 mm wide, so x_mm = 5 puts its side on x = 0.
        let dir = TempDir::new().unwrap();
        let fixture = write_fixture(&dir, &[(5.0, 70.0, 270.0), (150.0, 160.0, 90.0)], |_, t| t);
        let report = run(&fixture, 0.5).unwrap();
        assert!(report.all_inside_printable_area);
        assert!(report.minimum_edge_clearance_mm < 1e-9);
    }

    #[test]
    fn placement_beyond_the_tolerance_still_fails() {
        let dir = TempDir::new().unwrap();
        let fixture = write_fixture(
            &dir,
            &[(4.999, 70.0, 270.0), (150.0, 160.0, 90.0)],
            |_, t| t,
        );
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
}

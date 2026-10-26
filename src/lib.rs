use geo::{Coord, LineString, Polygon, algorithm::bool_ops::unary_union};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io, path::Path};

mod packing_search;

pub mod ams_calibration;
pub mod assemble;
pub mod colored_packer;
pub mod embedded_mesh;
pub mod nozzle_clean;
pub mod plate_centre;
pub mod plate_preview;
pub mod print_time;
pub mod scheduler;
pub mod process_output;
pub mod slicer_cli;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Footprint {
    /// XY loops (even-odd: outer rings and holes), centered on the mesh bbox. Built either
    /// from the whole model's top-down projection or — better, when a probe slice exists —
    /// from the slice's actual first layer, which already includes tree supports wandering
    /// several mm past the body and any brim: the real patch two packed neighbours must not
    /// overlap with.
    pub loops: Vec<Vec<Point>>,
    /// The model-body subset of `loops` (everything printed by non-support extrusions, plus
    /// brim). Two-zone rule: a body must keep clearance from anything foreign, while supports
    /// may overlap other supports. Empty = the whole footprint is body (mesh-projection
    /// branch, or a first layer without supports).
    pub body_loops: Vec<Vec<Point>>,
    /// What the bed bounds and exclusions are checked against: the first-layer patch UNION the
    /// whole-model silhouette. The slicer's bed-collision warnings measure the full model
    /// extent, which hangs below the printed patch on models with overhangs (a lying animal's
    /// tail), silently eating the edge margin otherwise. `None` = use `loops` as before.
    pub bed_loops: Option<Vec<Vec<Point>>>,
    pub width: f64,
    pub height: f64,
    pub min_z: f64,
    pub max_z: f64,
    pub triangle_count: usize,
}

fn read_f32(bytes: &[u8]) -> f32 {
    f32::from_le_bytes(bytes.try_into().expect("four bytes"))
}

pub fn extract_footprint(path: &Path) -> io::Result<Footprint> {
    let data = fs::read(path)?;
    if data.len() < 84 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "STL header is truncated",
        ));
    }
    let triangle_count = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
    let expected = 84usize
        .checked_add(triangle_count.checked_mul(50).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "STL triangle count overflows")
        })?)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "STL size overflows"))?;
    if data.len() != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("binary STL has {} bytes; expected {expected}", data.len()),
        ));
    }

    let mut triangles = Vec::with_capacity(triangle_count);
    let mut mins = [f64::INFINITY; 3];
    let mut maxs = [f64::NEG_INFINITY; 3];
    for index in 0..triangle_count {
        let base = 84 + index * 50 + 12;
        let mut triangle = [[0.0f32; 3]; 3];
        for (vertex_index, vertex) in triangle.iter_mut().enumerate() {
            let offset = base + vertex_index * 12;
            for axis in 0..3 {
                vertex[axis] = read_f32(&data[offset + axis * 4..offset + axis * 4 + 4]);
                if !vertex[axis].is_finite() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "STL contains non-finite coordinates",
                    ));
                }
                mins[axis] = mins[axis].min(f64::from(vertex[axis]));
                maxs[axis] = maxs[axis].max(f64::from(vertex[axis]));
            }
        }
        triangles.push(triangle);
    }

    let z_tolerance = ((maxs[2] - mins[2]) * 1e-6).max(1e-5);
    let center_x = (mins[0] + maxs[0]) * 0.5;
    let center_y = (mins[1] + maxs[1]) * 0.5;

    // The footprint is the union of every triangle's XY projection — the silhouette seen from
    // above. Two earlier designs were both less honest: an edge-parity walk over the flat
    // minimum-Z triangles (which real meshes break with T-junctions and self-touching contact
    // strips, failing as "bottom boundary is non-manifold" on a rounded-belly model), and even
    // a correct flat-bottom union still ignored overhangs — a lying animal whose head and ears
    // reach past its resting strip collided with neighbours that the sole said were clear.
    // The full projection also covers support bases, which stand under overhangs.
    // Degenerate triangles (zero XY area, e.g. vertical walls) add nothing to a union.
    let mut projections: Vec<Polygon> = Vec::with_capacity(triangles.len());
    for triangle in &triangles {
        let [a, b, c] = triangle.map(|vertex| Coord {
            x: f64::from(vertex[0]),
            y: f64::from(vertex[1]),
        });
        // STL gives no orientation guarantee between meshes; normalize to counter-clockwise
        // so the union's winding-based fill rule adds every patch instead of subtracting.
        let ordered = if (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x) >= 0.0 {
            [a, b, c]
        } else {
            [a, c, b]
        };
        if ordered[0] == ordered[1] || ordered[1] == ordered[2] {
            continue;
        }
        projections.push(Polygon::new(LineString::from(ordered.to_vec()), Vec::new()));
    }
    if projections.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "no footprint could be built: all {} triangles project to degenerate (zero-area) \
                 XY shapes (z range {:.3}..{:.3}, z tolerance {:.1e})",
                triangles.len(),
                mins[2],
                maxs[2],
                z_tolerance
            ),
        ));
    }
    let union = unary_union(&projections);

    let mut loops: Vec<Vec<Point>> = Vec::new();
    let push_ring = |ring: &LineString<f64>, loops: &mut Vec<Vec<Point>>| {
        let points: Vec<Point> = ring
            .coords()
            .map(|coord| Point {
                x: coord.x - center_x,
                y: coord.y - center_y,
            })
            .collect();
        if points.len() < 3 {
            return;
        }
        let mut loop_ = points;
        let first = loop_[0];
        if loop_.last() != Some(&first) {
            loop_.push(first);
        }
        loops.push(loop_);
    };
    for polygon in &union.0 {
        push_ring(polygon.exterior(), &mut loops);
        for interior in polygon.interiors() {
            push_ring(interior, &mut loops);
        }
    }
    if loops.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "no footprint could be built: {} triangles projected, but their union is empty \
                 (all are degenerate slivers)",
                projections.len()
            ),
        ));
    }
    // Micron-scale polygons survive the union as separate specks — triangulation noise, not
    // geometry the packer can collide with. Anything smaller than a printed feature (a 0.1 mm
    // square is already below a 0.4 mm nozzle) is dropped.
    const MIN_LOOP_AREA_MM2: f64 = 0.01;
    loops.retain(|loop_| polygon_area(loop_).abs() >= MIN_LOOP_AREA_MM2);
    if loops.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "STL has no printable footprint after removing degenerate rings",
        ));
    }

    loops.sort_by(|a, b| polygon_area(b).abs().total_cmp(&polygon_area(a).abs()));
    Ok(Footprint {
        // The mesh projection branch: everything is body — supports, if any, sit inside it.
        body_loops: loops.clone(),
        bed_loops: None,
        loops,
        width: maxs[0] - mins[0],
        height: maxs[1] - mins[1],
        min_z: mins[2],
        max_z: maxs[2],
        triangle_count,
    })
}

/// Builds the true contact patch from a probe slice's first layer: body walls, infill,
/// supports and brim exactly as the slicer lays them down. The loops are centered on the
/// body's bbox center — walls run along the model surface, so that center equals the mesh
/// bbox center the placements are built against, keeping the patch and the placed model in
/// the same coordinate frame.
///
/// Both slicer dialects are parsed. OrcaSlicer marks layers with `;LAYER_CHANGE`, features
/// with `;TYPE:` and widths with `;WIDTH:`. Bambu Studio marks layers with
/// `; CHANGE_LAYER` + `; Z_HEIGHT:` (its `; layer #N` lines number the prime tower's own
/// empty-grid pattern inside the tower section and are NOT layer boundaries), features with
/// `; FEATURE:` and widths with `; LINE_WIDTH:`. Extrusions in `;TYPE:`/`; FEATURE:` blocks
/// mentioning "tower" are skipped (the prime tower is packed around via its own keepout), as
/// is everything between `; WIPE_TOWER_START` and `; WIPE_TOWER_END` — Bambu emits no
/// feature marker for the tower's empty-grid infill, only the section wrapper — and so are
/// tiny-E moves after a travel — those are un-retractions, not laid-down lines, and would
/// draw phantom strips across the plate. Extrusions before the first layer marker (machine
/// start gcode) never touch the patch. Returns `None` when the first layer holds no object
/// extrusions to union.
pub fn first_layer_footprint(gcode: &[u8], model_center: Option<(f64, f64)>) -> Option<Footprint> {
    let text = String::from_utf8_lossy(gcode);
    let mut x = 0.0f64;
    let mut y = 0.0f64;
    let mut layer = 0usize;
    let mut extrusion_type = String::new();
    let mut line_width = 0.5f64;
    let mut in_tower_section = false;
    let mut sink = StripSink::default();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line == ";LAYER_CHANGE" || line == "; CHANGE_LAYER" {
            layer += 1;
            if layer > 1 {
                break;
            }
            continue;
        }
        if line.contains("WIPE_TOWER_END") {
            in_tower_section = false;
            continue;
        }
        if line.contains("WIPE_TOWER_START") {
            in_tower_section = true;
            continue;
        }
        if let Some(kind) = line
            .strip_prefix(";TYPE:")
            .or_else(|| line.strip_prefix("; FEATURE:"))
            .or_else(|| line.strip_prefix(";FEATURE:"))
        {
            extrusion_type = kind.trim().to_owned();
            continue;
        }
        if let Some(value) = line
            .strip_prefix(";WIDTH:")
            .or_else(|| line.strip_prefix("; LINE_WIDTH:"))
            .or_else(|| line.strip_prefix(";LINE_WIDTH:"))
        {
            if let Ok(width) = value.trim().parse::<f64>() {
                line_width = width;
            }
            continue;
        }
        if line.starts_with("G2") || line.starts_with("G3") {
            // Arc-fitted gcode (Bambu Studio's `enable_arc_fitting`) draws whole walls as
            // G2/G3 sweeps with an I/J centre. Skipping the line like any other comment
            // froze the tracked position at the last G1, so the next G1 inked a phantom
            // chord across whatever the arcs went around — on the magnet goat one 16.7 mm
            // chord over the head put a single stray pixel past the mesh silhouette,
            // `patch_within_silhouette` failed, and a bed that packs 18 rotated objects
            // locked to one orientation (15). Follow the arc: track its endpoint and ink
            // the sweep.
            let parsed_x = gcode_axis_token(line, 'X');
            let parsed_y = gcode_axis_token(line, 'Y');
            let next_x = parsed_x.unwrap_or(x);
            let next_y = parsed_y.unwrap_or(y);
            let filament = gcode_axis_token(line, 'E').unwrap_or(0.0);
            let in_layer_1 = layer >= 1
                && !in_tower_section
                && !extrusion_type.to_lowercase().contains("tower")
                && (parsed_x.is_some() || parsed_y.is_some());
            if filament > 0.0 && in_layer_1 {
                let is_body = !extrusion_type.to_lowercase().contains("support");
                if let (Some(i), Some(j)) =
                    (gcode_axis_token(line, 'I'), gcode_axis_token(line, 'J'))
                {
                    let center_x = x + i;
                    let center_y = y + j;
                    let radius = (i * i + j * j).sqrt();
                    if radius > 1e-9 {
                        let start_angle = (y - center_y).atan2(x - center_x);
                        let end_angle = (next_y - center_y).atan2(next_x - center_x);
                        // G2 sweeps clockwise (angle decreasing), G3 counter-clockwise;
                        // slicers never emit full-turn arcs, so unwrap to |sweep| ≤ 2π.
                        let mut sweep = end_angle - start_angle;
                        if line.starts_with("G2") {
                            if sweep >= 0.0 {
                                sweep -= std::f64::consts::TAU;
                            }
                        } else if sweep <= 0.0 {
                            sweep += std::f64::consts::TAU;
                        }
                        let length = radius * sweep.abs();
                        // Same density cut as straight moves, judged over the whole sweep:
                        // Arachne thin-tip arcs print far under a normal line's flow but
                        // an order of magnitude above a travel-smeared un-retraction.
                        if length > 0.3 && filament / length > 0.001 {
                            let steps = (((length / 0.5).ceil()) as usize).max(2);
                            let (mut from_x, mut from_y) = (x, y);
                            for step in 1..=steps {
                                let angle = start_angle + sweep * (step as f64) / (steps as f64);
                                let to_x = center_x + radius * angle.cos();
                                let to_y = center_y + radius * angle.sin();
                                sink.record((from_x, from_y), (to_x, to_y), line_width, is_body);
                                from_x = to_x;
                                from_y = to_y;
                            }
                        }
                    }
                }
                // Arcs without an I/J centre (R-form) carry no centre to rebuild the sweep
                // from; slicer output does not use them, so they only move the head.
            }
            x = next_x;
            y = next_y;
            continue;
        }
        if !(line.starts_with("G0") || line.starts_with("G1")) {
            continue;
        }
        let parsed_x = gcode_axis_token(line, 'X');
        let parsed_y = gcode_axis_token(line, 'Y');
        let parsed_e = gcode_axis_token(line, 'E');
        let next_x = parsed_x.unwrap_or(x);
        let next_y = parsed_y.unwrap_or(y);
        let extruding = parsed_e.unwrap_or(0.0) > 0.0;
        if extruding
            && layer >= 1
            && !in_tower_section
            && !extrusion_type.to_lowercase().contains("tower")
            && (parsed_x.is_some() || parsed_y.is_some())
        {
            let length = ((next_x - x).powi(2) + (next_y - y).powi(2)).sqrt();
            let filament = parsed_e.unwrap_or(0.0);
            // A 0.5 x 0.25 line spends ~0.05 mm of filament per mm of travel, but Arachne
            // thins real walls far below that: the long tip lines of a P1S goat print at
            // 0.001-0.004 filament-mm per mm — exactly the segments that stick out past the
            // mesh silhouette. Real Bambu/Orca un-retractions carry no X/Y and are already
            // filtered above; only a travel-smeared smear (measured ~0.0002/mm in the
            // fixture) stays below this cut, an order of magnitude under the real lines.
            if length > 0.3 && filament / length > 0.001 {
                let is_body = !extrusion_type.to_lowercase().contains("support");
                sink.record((x, y), (next_x, next_y), line_width, is_body);
            }
        }
        x = next_x;
        y = next_y;
    }
    let (patch_min_x, patch_min_y, patch_max_x, patch_max_y) = sink.patch_bounds?;
    if sink.strips.is_empty() {
        return None;
    }

    // The patch must share its origin with the model-bbox center the assembler places clones
    // by; for non-prism models the contact strip's own center sits elsewhere (a lying
    // animal's strip is low in its bbox), which silently shifts the reserved area.
    let (center_x, center_y) = match model_center {
        Some((x, y)) => (x, y),
        None => match sink.body_bounds {
            Some((min_x, min_y, max_x, max_y)) => ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5),
            // No body extrusions in layer 1 (a supports-only layer): fall back to the patch's
            // own bbox center and let the caller's clearance absorb the small mismatch.
            None => (
                (patch_min_x + patch_max_x) * 0.5,
                (patch_min_y + patch_max_y) * 0.5,
            ),
        },
    };
    let full_polygons: Vec<&Polygon> = sink.strips.iter().map(|(polygon, _)| polygon).collect();
    let union = unary_union(full_polygons.iter().copied());
    let body_polygons: Vec<&Polygon> = sink
        .strips
        .iter()
        .filter(|(_, is_body)| *is_body)
        .map(|(polygon, _)| polygon)
        .collect();
    let body_union = unary_union(body_polygons.iter().copied());
    let mut loops: Vec<Vec<Point>> = Vec::new();
    let push_ring = |ring: &LineString<f64>, loops: &mut Vec<Vec<Point>>| {
        let points: Vec<Point> = ring
            .coords()
            .map(|coord| Point {
                x: coord.x - center_x,
                y: coord.y - center_y,
            })
            .collect();
        if points.len() < 3 {
            return;
        }
        let mut loop_ = points;
        let first = loop_[0];
        if loop_.last() != Some(&first) {
            loop_.push(first);
        }
        loops.push(loop_);
    };
    // Only exterior rings: first-layer walls form a frame around the (unprinted) infill
    // interior, and an interior ring would make even-odd collision checks treat the middle of
    // a body as free space a neighbour could occupy.
    for polygon in &union.0 {
        push_ring(polygon.exterior(), &mut loops);
    }
    let mut body_loops: Vec<Vec<Point>> = Vec::new();
    for polygon in &body_union.0 {
        push_ring(polygon.exterior(), &mut body_loops);
    }
    const MIN_LOOP_AREA_MM2: f64 = 0.01;
    loops.retain(|loop_| polygon_area(loop_).abs() >= MIN_LOOP_AREA_MM2);
    body_loops.retain(|loop_| polygon_area(loop_).abs() >= MIN_LOOP_AREA_MM2);
    if loops.is_empty() {
        return None;
    }
    loops.sort_by(|a, b| polygon_area(b).abs().total_cmp(&polygon_area(a).abs()));
    Some(Footprint {
        body_loops,
        bed_loops: None, // the caller unions the silhouette in when overhangs demand it
        loops,
        width: patch_max_x - patch_min_x,
        height: patch_max_y - patch_min_y,
        min_z: 0.0,
        max_z: 0.0,
        triangle_count: 0,
    })
}

/// A printed line as geometry: the rectangle swept by the nozzle along the move.
fn strip_polygon(x0: f64, y0: f64, x1: f64, y1: f64, width: f64) -> Polygon {
    let length = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    let (dx, dy) = if length > 0.0 {
        ((x1 - x0) / length, (y1 - y0) / length)
    } else {
        (1.0, 0.0)
    };
    let half = width * 0.5;
    let px = -dy * half;
    let py = dx * half;
    let corners = [
        Coord {
            x: x0 + px,
            y: y0 + py,
        },
        Coord {
            x: x1 + px,
            y: y1 + py,
        },
        Coord {
            x: x1 - px,
            y: y1 - py,
        },
        Coord {
            x: x0 - px,
            y: y0 - py,
        },
        Coord {
            x: x0 + px,
            y: y0 + py,
        },
    ];
    Polygon::new(LineString::from(corners.to_vec()), Vec::new())
}

/// Ink collected from layer 1: the strip polygons plus the patch/body bounds they reach.
/// Straight G1 moves and the sampled points of a G2/G3 arc sweep land here alike.
#[derive(Default)]
struct StripSink {
    strips: Vec<(Polygon, bool)>,
    patch_bounds: Option<(f64, f64, f64, f64)>,
    body_bounds: Option<(f64, f64, f64, f64)>,
}

impl StripSink {
    fn record(&mut self, from: (f64, f64), to: (f64, f64), line_width: f64, is_body: bool) {
        let (x0, y0) = from;
        let (x1, y1) = to;
        self.strips
            .push((strip_polygon(x0, y0, x1, y1, line_width), is_body));
        self.patch_bounds = Some(grow_bounds(self.patch_bounds, x0, y0, x1, y1));
        if is_body {
            self.body_bounds = Some(grow_bounds(self.body_bounds, x0, y0, x1, y1));
        }
    }
}

fn grow_bounds(
    bounds: Option<(f64, f64, f64, f64)>,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
) -> (f64, f64, f64, f64) {
    let (min_x, min_y, max_x, max_y) = bounds.unwrap_or((
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ));
    (
        min_x.min(x0).min(x1),
        min_y.min(y0).min(y1),
        max_x.max(x0).max(x1),
        max_y.max(y0).max(y1),
    )
}

/// Numeric value of an `X`/`Y`/`E` token in a G0/G1 line (no sign restrictions).
fn gcode_axis_token(line: &str, axis: char) -> Option<f64> {
    line.split_whitespace().find_map(|token| {
        let token = token.trim_start_matches(';');
        token
            .strip_prefix(axis)
            .and_then(|value| value.parse::<f64>().ok())
    })
}

pub fn polygon_area(points: &[Point]) -> f64 {
    points
        .windows(2)
        .map(|pair| pair[0].x * pair[1].y - pair[1].x * pair[0].y)
        .sum::<f64>()
        * 0.5
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Rect {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Rect {
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    pub fn inset(self, margin: f64) -> Result<Self, String> {
        if margin < 0.0 {
            return Err("edge margin must be non-negative".to_owned());
        }
        let inset = Self::new(
            self.min_x + margin,
            self.min_y + margin,
            self.max_x - margin,
            self.max_y - margin,
        );
        if inset.min_x >= inset.max_x || inset.min_y >= inset.max_y {
            return Err(format!(
                "edge margin {margin} mm consumes printable rectangle {self:?}"
            ));
        }
        Ok(inset)
    }
}

pub fn axis_aligned_line_keepout(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    line_width: f64,
) -> Result<Rect, String> {
    if line_width <= 0.0 {
        return Err("purge-line width must be positive".to_owned());
    }
    let horizontal = (y0 - y1).abs() <= 1e-9;
    let vertical = (x0 - x1).abs() <= 1e-9;
    if !horizontal && !vertical {
        return Err("only axis-aligned purge-line segments are supported".to_owned());
    }
    if horizontal && vertical {
        return Err("purge-line segment must have non-zero length".to_owned());
    }
    let radius = line_width * 0.5;
    Ok(Rect::new(
        x0.min(x1) - radius,
        y0.min(y1) - radius,
        x0.max(x1) + radius,
        y0.max(y1) + radius,
    ))
}

#[derive(Clone, Debug)]
pub struct PixelMask {
    pub min_x: i32,
    pub min_y: i32,
    pub width: usize,
    pub height: usize,
    pub pixel_count: usize,
    rows: Vec<Vec<u64>>,
}

impl PixelMask {
    /// Packs span rows into bit rows, trimmed to the pixels' tight bounds. An empty raster
    /// becomes a 1x1 mask with no pixels set (it collides with nothing).
    fn from_spans(raster: &SpanRaster) -> Self {
        let mut min_x = i32::MAX;
        let mut max_x = i32::MIN;
        let mut first_row = None;
        let mut last_row = 0usize;
        let mut pixel_count = 0usize;
        for (index, row) in raster.rows.iter().enumerate() {
            if row.is_empty() {
                continue;
            }
            first_row.get_or_insert(index);
            last_row = index;
            min_x = min_x.min(row[0].0);
            max_x = max_x.max(row[row.len() - 1].1 - 1);
            pixel_count += row.iter().map(|&(a, b)| (b - a) as usize).sum::<usize>();
        }
        let Some(first_row) = first_row else {
            return Self {
                min_x: 0,
                min_y: 0,
                width: 1,
                height: 1,
                pixel_count: 0,
                rows: vec![vec![0u64]],
            };
        };
        let width = (max_x - min_x + 1) as usize;
        let height = last_row - first_row + 1;
        let words = width.div_ceil(64);
        let mut rows = vec![vec![0u64; words]; height];
        for (local_y, row) in raster.rows[first_row..=last_row].iter().enumerate() {
            for &(start, end) in row {
                set_bit_range(
                    &mut rows[local_y],
                    (start - min_x) as usize,
                    (end - min_x) as usize,
                );
            }
        }
        Self {
            min_x,
            min_y: raster.min_y + first_row as i32,
            width,
            height,
            pixel_count,
            rows,
        }
    }

    /// Row `local_y` as half-open spans of set bits, in mask-local columns.
    fn row_spans(&self, local_y: usize) -> Vec<(i32, i32)> {
        let row = &self.rows[local_y];
        let mut spans = Vec::new();
        let mut x = 0usize;
        while let Some(start) = next_bit(row, x, true, self.width) {
            let end = next_bit(row, start, false, self.width).unwrap_or(self.width);
            spans.push((start as i32, end as i32));
            x = end;
        }
        spans
    }

    pub fn contains_local(&self, x: i32, y: i32) -> bool {
        if x < self.min_x
            || y < self.min_y
            || x >= self.min_x + self.width as i32
            || y >= self.min_y + self.height as i32
        {
            return false;
        }
        let local_x = (x - self.min_x) as usize;
        let local_y = (y - self.min_y) as usize;
        (self.rows[local_y][local_x / 64] & (1u64 << (local_x % 64))) != 0
    }

    pub fn pixels(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.rows
            .iter()
            .enumerate()
            .flat_map(move |(row_index, row)| {
                row.iter().enumerate().flat_map(move |(word_index, &word)| {
                    (0..64).filter_map(move |bit| {
                        let local_x = word_index * 64 + bit;
                        if local_x < self.width && (word & (1u64 << bit)) != 0 {
                            Some((self.min_x + local_x as i32, self.min_y + row_index as i32))
                        } else {
                            None
                        }
                    })
                })
            })
    }
}

/// First bit at or after `from` equal to `value`, below `limit`. Bits past `limit` in the
/// last word are zero, so a search for a clear bit there reports `limit` as `None`.
fn next_bit(row: &[u64], from: usize, value: bool, limit: usize) -> Option<usize> {
    if from >= limit {
        return None;
    }
    let load = |index: usize| if value { row[index] } else { !row[index] };
    let mut index = from / 64;
    let mut word = load(index) & (!0u64 << (from % 64));
    loop {
        if word != 0 {
            let position = index * 64 + word.trailing_zeros() as usize;
            return (position < limit).then_some(position);
        }
        index += 1;
        if index >= row.len() {
            return None;
        }
        word = load(index);
    }
}

/// Sets bits `[start, end)` of a bit row.
fn set_bit_range(row: &mut [u64], start: usize, end: usize) {
    if start >= end {
        return;
    }
    let (first, last) = (start / 64, (end - 1) / 64);
    let low = !0u64 << (start % 64);
    let high = !0u64 >> (63 - (end - 1) % 64);
    if first == last {
        row[first] |= low & high;
        return;
    }
    row[first] |= low;
    for word in &mut row[first + 1..last] {
        *word = !0;
    }
    row[last] |= high;
}

/// The 64 bits `[start, start + 64)` of a bit row; positions outside the row read as clear.
fn bits_at(row: &[u64], start: i64) -> u64 {
    let word_at = |index: i64| -> u64 {
        if index < 0 || index as usize >= row.len() {
            0
        } else {
            row[index as usize]
        }
    };
    let index = start.div_euclid(64);
    let shift = start.rem_euclid(64) as u32;
    if shift == 0 {
        word_at(index)
    } else {
        (word_at(index) >> shift) | (word_at(index + 1) << (64 - shift))
    }
}

/// A raster as sorted, disjoint, half-open column spans per row (`rows[0]` is row `min_y`).
/// Every mask is built through spans: scanline fill and span dilation cost O(rows × edges)
/// instead of the O(pixels × edges) point-in-polygon sampling they replaced — an identical
/// pixel set (the scanline uses the exact crossing formula of `point_in_loops`), without
/// a hash allocation and point-in-polygon evaluation per pixel.
#[derive(Clone, Debug, Default)]
struct SpanRaster {
    min_y: i32,
    rows: Vec<Vec<(i32, i32)>>,
}

impl SpanRaster {
    /// Pixels whose centres `((ix + 0.5) * res, (iy + 0.5) * res)` fall inside `loops` under
    /// the even-odd rule, restricted to `ix ∈ [min_ix, max_ix]`, `iy ∈ [min_iy, max_iy]` —
    /// exactly the set the per-pixel `point_in_loops` sampling produced.
    fn scanline(
        loops: &[Vec<Point>],
        resolution: f64,
        (min_ix, max_ix, min_iy, max_iy): (i32, i32, i32, i32),
    ) -> Self {
        let mut rows = Vec::with_capacity((max_iy - min_iy + 1).max(0) as usize);
        let mut crossings: Vec<f64> = Vec::new();
        for iy in min_iy..=max_iy {
            let y = (f64::from(iy) + 0.5) * resolution;
            crossings.clear();
            for loop_ in loops {
                for edge in loop_.windows(2) {
                    let (a, b) = (edge[0], edge[1]);
                    if (a.y > y) != (b.y > y) {
                        crossings.push((b.x - a.x) * (y - a.y) / (b.y - a.y) + a.x);
                    }
                }
            }
            crossings.sort_unstable_by(f64::total_cmp);
            // A centre x is inside iff an odd number of crossings c satisfy x < c, i.e. an
            // odd number satisfy c <= x: x ∈ [c[2k], c[2k+1]).
            let mut row: Vec<(i32, i32)> = Vec::new();
            for pair in crossings.chunks_exact(2) {
                let first = first_centre_at_or_after(pair[0], resolution).max(min_ix);
                let end = first_centre_at_or_after(pair[1], resolution).min(max_ix + 1);
                if first < end {
                    push_span(&mut row, first, end);
                }
            }
            rows.push(row);
        }
        Self {
            min_y: min_iy,
            rows,
        }
    }

    fn row(&self, y: i32) -> &[(i32, i32)] {
        let index = y - self.min_y;
        if index < 0 || index as usize >= self.rows.len() {
            &[]
        } else {
            &self.rows[index as usize]
        }
    }

    fn max_y(&self) -> i32 {
        self.min_y + self.rows.len() as i32
    }

    fn union(&self, other: &SpanRaster) -> SpanRaster {
        if self.rows.is_empty() {
            return other.clone();
        }
        if other.rows.is_empty() {
            return self.clone();
        }
        let min_y = self.min_y.min(other.min_y);
        let max_y = self.max_y().max(other.max_y());
        let rows = (min_y..max_y)
            .map(|y| {
                let mut spans: Vec<(i32, i32)> =
                    self.row(y).iter().chain(other.row(y)).copied().collect();
                merge_spans(&mut spans);
                spans
            })
            .collect();
        SpanRaster { min_y, rows }
    }

    /// Dilation by the discrete disc `dx² + dy² <= radius²` — the structuring element the
    /// per-pixel dilation used: row `y` of the result is the union over `dy` of row `y + dy`
    /// widened by `isqrt(radius² - dy²)` on both sides.
    fn dilate(&self, radius: i32) -> SpanRaster {
        if radius <= 0 || self.rows.is_empty() {
            return self.clone();
        }
        let widths: Vec<i32> = (-radius..=radius)
            .map(|dy| disc_half_width(radius, dy))
            .collect();
        let min_y = self.min_y - radius;
        let max_y = self.max_y() + radius;
        let rows = (min_y..max_y)
            .map(|y| {
                let mut spans: Vec<(i32, i32)> = Vec::new();
                for (offset, &w) in widths.iter().enumerate() {
                    let dy = offset as i32 - radius;
                    spans.extend(self.row(y + dy).iter().map(|&(a, b)| (a - w, b + w)));
                }
                merge_spans(&mut spans);
                spans
            })
            .collect();
        SpanRaster { min_y, rows }
    }

    /// Erosion by the same discrete disc `dilate` uses: a pixel survives iff the whole disc
    /// around it is set. Row `y` is the intersection over `dy` of row `y + dy` narrowed by
    /// `isqrt(radius² - dy²)` on both sides (spans are merged, so a run of set pixels always
    /// lies inside a single span).
    fn erode(&self, radius: i32) -> SpanRaster {
        if radius <= 0 || self.rows.is_empty() {
            return self.clone();
        }
        let min_y = self.min_y + radius;
        let max_y = self.max_y() - radius;
        let rows = (min_y..max_y.max(min_y))
            .map(|y| {
                let mut acc: Option<Vec<(i32, i32)>> = None;
                for dy in -radius..=radius {
                    let w = disc_half_width(radius, dy);
                    let narrowed: Vec<(i32, i32)> = self
                        .row(y + dy)
                        .iter()
                        .map(|&(a, b)| (a + w, b - w))
                        .filter(|&(a, b)| a < b)
                        .collect();
                    acc = Some(match acc {
                        None => narrowed,
                        Some(previous) => intersect_spans(&previous, &narrowed),
                    });
                    if acc.as_ref().is_some_and(Vec::is_empty) {
                        break;
                    }
                }
                acc.unwrap_or_default()
            })
            .collect();
        SpanRaster { min_y, rows }
    }
}

/// `isqrt(radius² - dy²)`: half-width of row `dy` of the discrete disc `dx² + dy² <= radius²`.
fn disc_half_width(radius: i32, dy: i32) -> i32 {
    let mut w = (f64::from(radius * radius - dy * dy)).sqrt() as i32;
    while w * w + dy * dy > radius * radius {
        w -= 1;
    }
    while (w + 1) * (w + 1) + dy * dy <= radius * radius {
        w += 1;
    }
    w
}

/// Intersection of two sorted, disjoint span lists.
fn intersect_spans(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() && j < b.len() {
        let start = a[i].0.max(b[j].0);
        let end = a[i].1.min(b[j].1);
        if start < end {
            out.push((start, end));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Smallest `ix` whose pixel centre `(ix + 0.5) * resolution` is `>= c`, evaluated with the
/// same float expression the per-pixel test uses so boundary pixels match bit for bit.
fn first_centre_at_or_after(c: f64, resolution: f64) -> i32 {
    let mut ix = (c / resolution - 0.5).ceil() as i32;
    while (f64::from(ix - 1) + 0.5) * resolution >= c {
        ix -= 1;
    }
    while (f64::from(ix) + 0.5) * resolution < c {
        ix += 1;
    }
    ix
}

/// Appends `[start, end)` to a row built left to right, merging with the previous span.
fn push_span(row: &mut Vec<(i32, i32)>, start: i32, end: i32) {
    if let Some(last) = row.last_mut()
        && start <= last.1
    {
        last.1 = last.1.max(end);
        return;
    }
    row.push((start, end));
}

/// Sorts spans and merges overlapping or adjacent ones in place.
fn merge_spans(spans: &mut Vec<(i32, i32)>) {
    if spans.len() <= 1 {
        return;
    }
    spans.sort_unstable();
    let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for &(start, end) in spans.iter() {
        push_span(&mut merged, start, end);
    }
    *spans = merged;
}

#[derive(Clone, Debug)]
pub struct RasterMask {
    pub angle_deg: f64,
    pub resolution: f64,
    pub physical: PixelMask,
    pub clearance: PixelMask,
    /// The body subset of the patch, dilated by the same half-clearance as `clearance`.
    /// Two-zone packing rule: a body keeps clearance from anything foreign, while support
    /// structures may touch other supports. Equals `clearance` when the footprint carries no
    /// separate body loops (mesh projection, or a support-free first layer).
    pub body_clearance: PixelMask,
    /// Bed/exclusion variant of the patch (patch + model silhouette when overhangs extend
    /// past the first layer). Collisions between objects still use the patch above.
    pub bed_physical: PixelMask,
    pub bed_clearance: PixelMask,
    pub bed_physical_bounds: Rect,
    pub physical_bounds: Rect,
    /// Support-overlap rule (`PackOptions::support_overlap_mm = Some(X)`): the full patch
    /// dilated by `(clearance - X) / 2` (eroded when negative) — the `clearance` zone shrunk
    /// by X/2 per side. Two cores must not overlap, so the clearance zones of two supports
    /// overlap by at most X. `None` without the option, and when the footprint carries no
    /// support zone (body == full): the two-zone rule then already forbids any overlap of
    /// the clearance zones, which contain the cores.
    pub support_core: Option<PixelMask>,
}

pub fn rasterize_footprint(
    footprint: &Footprint,
    angle_deg: f64,
    resolution: f64,
    clearance: f64,
) -> RasterMask {
    rasterize_footprint_with_support(footprint, angle_deg, resolution, clearance, None)
}

/// Pixel radius of the support core: `ceil((clearance - X) / 2 / resolution)`. Rounding up
/// dilates generously and erodes sparingly, so the raster never allows more than X; at X = 0
/// it is exactly the `clearance` radius.
fn support_core_radius(clearance: f64, support_overlap_mm: f64, resolution: f64) -> i32 {
    ((clearance - support_overlap_mm) * 0.5 / resolution).ceil() as i32
}

/// `rasterize_footprint` plus the support core for `PackOptions::support_overlap_mm`.
pub fn rasterize_footprint_with_support(
    footprint: &Footprint,
    angle_deg: f64,
    resolution: f64,
    clearance: f64,
    support_overlap_mm: Option<f64>,
) -> RasterMask {
    assert!(resolution.is_finite() && resolution > 0.0);
    assert!(angle_deg.is_finite());
    assert!(clearance.is_finite() && clearance >= 0.0);
    let radians = angle_deg.to_radians();
    let (sin, cos) = radians.sin_cos();
    let rotated: Vec<Vec<Point>> = footprint
        .loops
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
        .collect();
    let mut bounds = Rect::new(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for point in rotated.iter().flatten() {
        bounds.min_x = bounds.min_x.min(point.x);
        bounds.min_y = bounds.min_y.min(point.y);
        bounds.max_x = bounds.max_x.max(point.x);
        bounds.max_y = bounds.max_y.max(point.y);
    }

    let pixel_range = |bounds: &Rect| {
        (
            (bounds.min_x / resolution - 0.5).floor() as i32,
            (bounds.max_x / resolution - 0.5).ceil() as i32,
            (bounds.min_y / resolution - 0.5).floor() as i32,
            (bounds.max_y / resolution - 0.5).ceil() as i32,
        )
    };
    let range = pixel_range(&bounds);
    let mut physical_pixels = SpanRaster::scanline(&rotated, resolution, range);
    if physical_pixels.rows.iter().all(Vec::is_empty) {
        let min_x = (bounds.min_x / resolution).floor() as i32;
        let max_x = (bounds.max_x / resolution).ceil() as i32;
        let min_y = (bounds.min_y / resolution).floor() as i32;
        let max_y = (bounds.max_y / resolution).ceil() as i32;
        physical_pixels = SpanRaster {
            min_y,
            rows: vec![vec![(min_x, max_x.max(min_x + 1))]; (max_y - min_y).max(1) as usize],
        };
    }

    let radius = (clearance * 0.5 / resolution).ceil() as i32;
    let clearance_pixels = physical_pixels.dilate(radius);

    let body_clearance_pixels = if footprint.body_loops.is_empty() {
        clearance_pixels.clone()
    } else {
        let rotated_body: Vec<Vec<Point>> = footprint
            .body_loops
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
            .collect();
        let body = SpanRaster::scanline(&rotated_body, resolution, range);
        if body.rows.iter().all(Vec::is_empty) {
            clearance_pixels.clone()
        } else {
            body.dilate(radius)
        }
    };

    let (bed_physical_pixels, bed_physical_bounds) = match &footprint.bed_loops {
        None => (physical_pixels.clone(), bounds),
        Some(bed_loops) => {
            let rotated_bed: Vec<Vec<Point>> = bed_loops
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
                .collect();
            let mut bed_bounds = Rect::new(
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            );
            for point in rotated_bed.iter().flatten() {
                bed_bounds.min_x = bed_bounds.min_x.min(point.x);
                bed_bounds.min_y = bed_bounds.min_y.min(point.y);
                bed_bounds.max_x = bed_bounds.max_x.max(point.x);
                bed_bounds.max_y = bed_bounds.max_y.max(point.y);
            }
            let bed_only = SpanRaster::scanline(&rotated_bed, resolution, pixel_range(&bed_bounds));
            bed_bounds.min_x = bed_bounds.min_x.min(bounds.min_x);
            bed_bounds.min_y = bed_bounds.min_y.min(bounds.min_y);
            bed_bounds.max_x = bed_bounds.max_x.max(bounds.max_x);
            bed_bounds.max_y = bed_bounds.max_y.max(bounds.max_y);
            (physical_pixels.union(&bed_only), bed_bounds)
        }
    };
    let bed_clearance_pixels = bed_physical_pixels.dilate(radius);

    let support_core = support_overlap_mm
        .filter(|_| {
            body_clearance_pixels.min_y != clearance_pixels.min_y
                || body_clearance_pixels.rows != clearance_pixels.rows
        })
        .map(|overlap| {
            assert!(overlap.is_finite() && overlap >= 0.0);
            let core_radius = support_core_radius(clearance, overlap, resolution);
            let core = if core_radius >= 0 {
                physical_pixels.dilate(core_radius)
            } else {
                physical_pixels.erode(-core_radius)
            };
            PixelMask::from_spans(&core)
        });

    RasterMask {
        angle_deg,
        resolution,
        physical: PixelMask::from_spans(&physical_pixels),
        clearance: PixelMask::from_spans(&clearance_pixels),
        body_clearance: PixelMask::from_spans(&body_clearance_pixels),
        bed_physical: PixelMask::from_spans(&bed_physical_pixels),
        bed_clearance: PixelMask::from_spans(&bed_clearance_pixels),
        bed_physical_bounds,
        physical_bounds: bounds,
        support_core,
    }
}

/// Support-overlap pair: the support cores of two placements (shifted by `(dx, dy)` pixels
/// of `b` relative to `a`) overlap. Never true when either side carries no core.
fn support_cores_overlap(a: &RasterMask, b: &RasterMask, dx: i32, dy: i32) -> bool {
    match (&a.support_core, &b.support_core) {
        (Some(core_a), Some(core_b)) => masks_overlap(core_a, 0, 0, core_b, dx, dy),
        _ => false,
    }
}

fn point_in_loops(point: Point, loops: &[Vec<Point>]) -> bool {
    let mut inside = false;
    for loop_ in loops {
        for edge in loop_.windows(2) {
            let a = edge[0];
            let b = edge[1];
            if (a.y > point.y) != (b.y > point.y)
                && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
            {
                inside = !inside;
            }
        }
    }
    inside
}

/// Even-odd containment of a point in a footprint's loops — public for diagnostic tools.
pub fn point_in_footprint(loops: &[Vec<Point>], x: f64, y: f64) -> bool {
    point_in_loops(Point { x, y }, loops)
}

pub fn masks_overlap(a: &PixelMask, a_x: i32, a_y: i32, b: &PixelMask, b_x: i32, b_y: i32) -> bool {
    let a_min_x = a.min_x + a_x;
    let a_min_y = a.min_y + a_y;
    let a_max_x = a_min_x + a.width as i32;
    let a_max_y = a_min_y + a.height as i32;
    let b_min_x = b.min_x + b_x;
    let b_min_y = b.min_y + b_y;
    let b_max_x = b_min_x + b.width as i32;
    let b_max_y = b_min_y + b.height as i32;
    if a_max_x <= b_min_x || b_max_x <= a_min_x || a_max_y <= b_min_y || b_max_y <= a_min_y {
        return false;
    }
    // Word-parallel: for each shared row, compare every word of `a` against the 64 bits of
    // `b` that sit under it.
    let first_row = a_min_y.max(b_min_y);
    let last_row = a_max_y.min(b_max_y);
    let offset = i64::from(a_min_x - b_min_x);
    for y in first_row..last_row {
        let a_row = &a.rows[(y - a_min_y) as usize];
        let b_row = &b.rows[(y - b_min_y) as usize];
        for (index, &word) in a_row.iter().enumerate() {
            if word != 0 && word & bits_at(b_row, offset + index as i64 * 64) != 0 {
                return true;
            }
        }
    }
    false
}

pub fn placement_is_allowed(
    mask: &RasterMask,
    center_x_pixels: i32,
    center_y_pixels: i32,
    bed: Rect,
    exclusions: &[Rect],
) -> bool {
    let center_x = f64::from(center_x_pixels) * mask.resolution;
    let center_y = f64::from(center_y_pixels) * mask.resolution;
    let bounds = mask.bed_physical_bounds;
    if center_x + bounds.min_x < bed.min_x
        || center_y + bounds.min_y < bed.min_y
        || center_x + bounds.max_x > bed.max_x
        || center_y + bounds.max_y > bed.max_y
    {
        return false;
    }
    !bounds_hit_exclusion(bounds, center_x, center_y, 0.0, exclusions)
        && !mask.bed_clearance.pixels().any(|(x, y)| {
            let wx = center_x + (f64::from(x) + 0.5) * mask.resolution;
            let wy = center_y + (f64::from(y) + 0.5) * mask.resolution;
            exclusions
                .iter()
                .any(|r| wx >= r.min_x && wx <= r.max_x && wy >= r.min_y && wy <= r.max_y)
        })
}

/// BambuStudio's plate validation rejects an object whose AXIS-ALIGNED bounding box comes
/// near a bed exclusion area — the silhouette is irrelevant there. A rotated goat can hold
/// its silhouette 6 mm off the exclusion corner while its bbox still covers the zone, and
/// the final slice then dies with "too close to exclusion area" after minutes of packing
/// (a real 17-object P1S magnet plate, twice). Exclusion feasibility therefore measures the
/// placement's rotated bbox — the patch ∪ silhouette extent, `bed_physical_bounds` — padded
/// by the same clearance the pixel-based check used. Bed bounds stay silhouette-accurate;
/// only exclusion checks take the bbox semantics. `x_px`/`y_px` are bed pixel coordinates
/// (origin at `bed.min`, the coordinate space of the packing loops).
fn aabb_hits_exclusion(options: &PackOptions, mask: &RasterMask, x_px: i32, y_px: i32) -> bool {
    let origin_x = options.bed.min_x + f64::from(x_px) * options.resolution;
    let origin_y = options.bed.min_y + f64::from(y_px) * options.resolution;
    bounds_hit_exclusion(
        mask.bed_physical_bounds,
        origin_x,
        origin_y,
        options.clearance,
        &options.exclusions,
    )
}

fn bounds_hit_exclusion(bounds: Rect, x: f64, y: f64, pad: f64, exclusions: &[Rect]) -> bool {
    exclusions.iter().any(|r| {
        x + bounds.min_x - pad < r.max_x
            && x + bounds.max_x + pad > r.min_x
            && y + bounds.min_y - pad < r.max_y
            && y + bounds.max_y + pad > r.min_y
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackOptions {
    pub bed: Rect,
    pub exclusions: Vec<Rect>,
    pub resolution: f64,
    pub clearance: f64,
    pub angle_step_deg: f64,
    /// Deterministic search effort. All orientation pairs get a baseline pass; higher values
    /// explore additional sweep directions and starting orientations, up to the finite portfolio.
    pub attempts: usize,
    /// Compatibility field for older clients; free centers are now searched exhaustively.
    pub candidates_per_step: usize,
    /// Compatibility field. The finite deterministic portfolio does not use racy stall stopping.
    pub stall_attempts: usize,
    /// Hard cap on objects per layout (0 = unlimited). The search itself stops when the bed
    /// is full; a cap is only for deliberately sparse plates or bounded slicing time.
    pub max_objects: usize,
    /// Pack every object at 0°. Tree supports are rebuilt by the slicer in table coordinates,
    /// so a footprint measured at one orientation silently stops matching the printed patch
    /// once the model rotates; a locked orientation keeps every clone identical to the probe.
    pub lock_orientation: bool,
    /// Retained in reports for compatibility; geometry, not a random seed, determines layouts.
    pub seed: u64,
    /// Supports (full patch minus body) of different objects may overlap by at most this many
    /// mm, measured on their clearance zones: the supports themselves keep `clearance - X`
    /// (negative = overlap depth). `0` = supports keep the clearance a body keeps; `None` =
    /// unlimited (the plain two-zone rule). Bodies are unaffected. A no-op for footprints
    /// without a separate body zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support_overlap_mm: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackedPlacement {
    pub x_mm: f64,
    pub y_mm: f64,
    pub angle_deg: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackResult {
    pub placements: Vec<PackedPlacement>,
    pub resolution_mm: f64,
    pub requested_clearance_mm: f64,
    pub angle_step_deg: f64,
    pub attempts: usize,
    /// Number of deterministic sweep layouts evaluated (including the mandatory angle pass).
    pub attempts_run: usize,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug)]
struct GridPlacement {
    mask: usize,
    x: i32,
    y: i32,
}

fn check_pack_input(footprint: &Footprint, options: &PackOptions) -> Result<(), String> {
    let valid_rect = |r: &Rect| {
        [r.min_x, r.min_y, r.max_x, r.max_y]
            .iter()
            .all(|v| v.is_finite())
            && r.min_x < r.max_x
            && r.min_y < r.max_y
    };
    if !options.resolution.is_finite()
        || options.resolution <= 0.0
        || !options.clearance.is_finite()
        || options.clearance < 0.0
        || !options.angle_step_deg.is_finite()
        || options.angle_step_deg <= 0.0
        || options.angle_step_deg > 360.0
        || !valid_rect(&options.bed)
        || options.exclusions.iter().any(|r| !valid_rect(r))
        || options
            .support_overlap_mm
            .is_some_and(|x| !x.is_finite() || x < 0.0)
    {
        return Err(
            "invalid packing resolution, clearance, angle step, support overlap or bed/exclusion \
             bounds"
                .to_owned(),
        );
    }
    if footprint.loops.is_empty()
        || footprint
            .loops
            .iter()
            .chain(&footprint.body_loops)
            .chain(footprint.bed_loops.iter().flatten())
            .any(|l| {
                l.len() < 4
                    || l.first() != l.last()
                    || l.iter().any(|p| !p.x.is_finite() || !p.y.is_finite())
            })
    {
        return Err("footprint must contain finite, closed polygon rings".to_owned());
    }
    if footprint
        .loops
        .iter()
        .all(|l| polygon_area(l).abs() < 1e-12)
    {
        return Err("footprint has zero area".to_owned());
    }
    Ok(())
}

pub fn pack_footprint(footprint: &Footprint, options: &PackOptions) -> PackResult {
    assert!(options.resolution.is_finite() && options.resolution > 0.0);
    assert!(
        options.angle_step_deg.is_finite()
            && options.angle_step_deg > 0.0
            && options.angle_step_deg <= 360.0
    );
    check_pack_input(footprint, options).expect("invalid packing input");
    packing_search::search(footprint, options)
}

/// Deterministic rectangular-grid baseline in the same canonical frame as the search.
/// Uses two-zone pitches and checks every placed neighbour (including distant islands).
/// The physical extent must stay on the bed; clearance is only between foreign structures.
pub fn grid_layout(footprint: &Footprint, options: &PackOptions) -> PackResult {
    check_pack_input(footprint, options).expect("invalid packing input");
    let frame = packing_search::Frame::new(footprint, options.lock_orientation);
    let mut result = grid_layout_inner(&frame.footprint, options);
    result.placements = result
        .placements
        .iter()
        .map(|p| frame.to_input(p))
        .collect();
    result
}

fn grid_layout_inner(footprint: &Footprint, options: &PackOptions) -> PackResult {
    assert!(options.resolution > 0.0);
    let mut best: Vec<GridPlacement> = Vec::new();
    let mut best_angle = 0.0f64;
    let orientations: &[f64] =
        if options.lock_orientation || (footprint.width - footprint.height).abs() < 1e-9 {
            &[0.0]
        } else {
            &[0.0, 90.0]
        };
    for &angle in orientations {
        let mask = rasterize_footprint_with_support(
            footprint,
            angle,
            options.resolution,
            options.clearance,
            options.support_overlap_mm,
        );
        // The lattice pitch per axis is the SMALLEST whole-pixel shift where two copies of
        // the mask satisfy the two-zone rule — not the full bounding box. Distant lattice
        // neighbours can still conflict for disconnected shapes; every cell is checked below.
        let pair_collides = |dx: i32, dy: i32| {
            masks_overlap(&mask.clearance, 0, 0, &mask.body_clearance, dx, dy)
                || masks_overlap(&mask.body_clearance, 0, 0, &mask.clearance, dx, dy)
                || support_cores_overlap(&mask, &mask, dx, dy)
        };
        let pitch_x = (1..)
            .find(|&s| !pair_collides(s, 0))
            .unwrap_or(mask.bed_clearance.width as i32 + 1);
        let mut pitch_y = (1..)
            .find(|&s| !pair_collides(0, s))
            .unwrap_or(mask.bed_clearance.height as i32 + 1);
        // Diagonal neighbours sit at (pitch_x, pitch_y); grow the row pitch until those clear
        // too (rare: only for diagonal support tendrils).
        while pair_collides(pitch_x, pitch_y) || pair_collides(-pitch_x, pitch_y) {
            pitch_y += 1;
        }

        let mut placed: Vec<GridPlacement> = Vec::new();
        let min_x = (-mask.bed_physical_bounds.min_x / options.resolution - 1e-9).ceil() as i32;
        let min_y = (-mask.bed_physical_bounds.min_y / options.resolution - 1e-9).ceil() as i32;
        let max_x = ((options.bed.max_x - options.bed.min_x - mask.bed_physical_bounds.max_x)
            / options.resolution
            + 1e-9)
            .floor() as i32;
        let max_y = ((options.bed.max_y - options.bed.min_y - mask.bed_physical_bounds.max_y)
            / options.resolution
            + 1e-9)
            .floor() as i32;
        let mut y = min_y;
        while y <= max_y {
            let mut x = min_x;
            while x <= max_x {
                // Bed membership follows the patch+silhouette extent; the dilated mask may
                // poke past the bed edge, the physical model may not.
                let on_bed = mask.bed_physical_bounds.min_x + f64::from(x) * options.resolution
                    >= -1e-7
                    && mask.bed_physical_bounds.max_x + f64::from(x) * options.resolution
                        <= options.bed.max_x - options.bed.min_x + 1e-7
                    && mask.bed_physical_bounds.min_y + f64::from(y) * options.resolution >= -1e-7
                    && mask.bed_physical_bounds.max_y + f64::from(y) * options.resolution
                        <= options.bed.max_y - options.bed.min_y + 1e-7;
                // Exclusions follow the placement's bbox (see aabb_hits_exclusion); bed
                // bounds stay on the patch+silhouette extent above.
                if on_bed
                    && !aabb_hits_exclusion(options, &mask, x, y)
                    && !placed.iter().any(|p| pair_collides(x - p.x, y - p.y))
                {
                    placed.push(GridPlacement { mask: 0, x, y });
                }
                x += pitch_x;
                if cap_reached(placed.len(), options) {
                    break;
                }
            }
            if cap_reached(placed.len(), options) {
                break;
            }
            y += pitch_y;
        }
        if placed.len() > best.len() {
            best = placed;
            best_angle = angle;
        }
    }
    PackResult {
        placements: best
            .iter()
            .map(|p| PackedPlacement {
                x_mm: options.bed.min_x + f64::from(p.x) * options.resolution,
                y_mm: options.bed.min_y + f64::from(p.y) * options.resolution,
                angle_deg: best_angle,
            })
            .collect(),
        resolution_mm: options.resolution,
        requested_clearance_mm: options.clearance,
        angle_step_deg: options.angle_step_deg,
        attempts: 1,
        attempts_run: 1,
        seed: options.seed,
    }
}

fn better_layout(
    a: Vec<GridPlacement>,
    b: Vec<GridPlacement>,
    masks: &[RasterMask],
) -> Vec<GridPlacement> {
    if b.len() > a.len() {
        return b;
    }
    if a.len() > b.len() {
        return a;
    }
    if layout_bbox_area(&b, masks) < layout_bbox_area(&a, masks) {
        b
    } else {
        a
    }
}

fn layout_bbox_area(layout: &[GridPlacement], masks: &[RasterMask]) -> f64 {
    if layout.is_empty() {
        return f64::INFINITY;
    }
    let mut bounds = Rect::new(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in layout {
        let mask = &masks[p.mask];
        bounds.min_x = bounds
            .min_x
            .min(f64::from(p.x) + mask.physical_bounds.min_x / mask.resolution);
        bounds.min_y = bounds
            .min_y
            .min(f64::from(p.y) + mask.physical_bounds.min_y / mask.resolution);
        bounds.max_x = bounds
            .max_x
            .max(f64::from(p.x) + mask.physical_bounds.max_x / mask.resolution);
        bounds.max_y = bounds
            .max_y
            .max(f64::from(p.y) + mask.physical_bounds.max_y / mask.resolution);
    }
    (bounds.max_x - bounds.min_x) * (bounds.max_y - bounds.min_y)
}

pub fn validate_result(
    footprint: &Footprint,
    options: &PackOptions,
    result: &PackResult,
) -> Result<(), String> {
    check_pack_input(footprint, options)?;
    if options.max_objects > 0 && result.placements.len() > options.max_objects {
        return Err("layout exceeds max_objects".to_owned());
    }
    if result
        .placements
        .iter()
        .any(|p| !p.x_mm.is_finite() || !p.y_mm.is_finite() || !p.angle_deg.is_finite())
    {
        return Err("layout contains non-finite placement coordinates".to_owned());
    }
    if options.lock_orientation
        && result
            .placements
            .iter()
            .any(|p| p.angle_deg.rem_euclid(360.0).abs() > 1e-7)
    {
        return Err("layout violates locked orientation".to_owned());
    }
    let frame = packing_search::Frame::new(footprint, options.lock_orientation);
    let footprint = &frame.footprint;
    let mut canonical_result = result.clone();
    canonical_result.placements = result
        .placements
        .iter()
        .map(|p| frame.to_canonical(p))
        .collect();
    let result = &canonical_result;
    // Keep the fractional phase of imported/manual placements. Rounding every centre
    // independently can turn a real overlap into two merely touching raster masks.
    let centers: Vec<_> = result
        .placements
        .iter()
        .map(|p| {
            (
                ((p.x_mm - options.bed.min_x) / options.resolution).round() as i32,
                ((p.y_mm - options.bed.min_y) / options.resolution).round() as i32,
            )
        })
        .collect();
    let masks: Vec<RasterMask> = result
        .placements
        .iter()
        .zip(&centers)
        .map(|(p, &(x, y))| {
            let dx = p.x_mm - options.bed.min_x - f64::from(x) * options.resolution;
            let dy = p.y_mm - options.bed.min_y - f64::from(y) * options.resolution;
            if dx.abs() < 1e-7 && dy.abs() < 1e-7 {
                return rasterize_footprint_with_support(
                    footprint,
                    p.angle_deg,
                    options.resolution,
                    options.clearance,
                    options.support_overlap_mm,
                );
            }
            let (s, c) = p.angle_deg.to_radians().sin_cos();
            let offset = Point {
                x: dx * c + dy * s,
                y: -dx * s + dy * c,
            };
            let shift = |loops: &[Vec<Point>]| {
                loops
                    .iter()
                    .map(|l| {
                        l.iter()
                            .map(|p| Point {
                                x: p.x + offset.x,
                                y: p.y + offset.y,
                            })
                            .collect()
                    })
                    .collect()
            };
            let mut shifted = footprint.clone();
            shifted.loops = shift(&footprint.loops);
            shifted.body_loops = shift(&footprint.body_loops);
            shifted.bed_loops = footprint.bed_loops.as_ref().map(|l| shift(l));
            rasterize_footprint_with_support(
                &shifted,
                p.angle_deg,
                options.resolution,
                options.clearance,
                options.support_overlap_mm,
            )
        })
        .collect();
    for (index, mask) in masks.iter().enumerate() {
        let (px, py) = centers[index];
        let x_mm = options.bed.min_x + f64::from(px) * options.resolution;
        let y_mm = options.bed.min_y + f64::from(py) * options.resolution;
        let translated_bounds = Rect::new(
            x_mm + mask.bed_physical_bounds.min_x,
            y_mm + mask.bed_physical_bounds.min_y,
            x_mm + mask.bed_physical_bounds.max_x,
            y_mm + mask.bed_physical_bounds.max_y,
        );
        if translated_bounds.min_x < options.bed.min_x - 1e-6
            || translated_bounds.min_y < options.bed.min_y - 1e-6
            || translated_bounds.max_x > options.bed.max_x + 1e-6
            || translated_bounds.max_y > options.bed.max_y + 1e-6
        {
            return Err(format!(
                "placement {index} leaves bed: {translated_bounds:?}"
            ));
        }
        // Exclusion feasibility follows the placement's bbox — the semantics BambuStudio's
        // own validation uses (see aabb_hits_exclusion).
        if bounds_hit_exclusion(
            mask.bed_physical_bounds,
            x_mm,
            y_mm,
            options.clearance,
            &options.exclusions,
        ) {
            return Err(format!("placement {index} intersects exclusion (bbox)"));
        }
        for (previous, previous_mask) in masks.iter().enumerate().take(index) {
            let (previous_px, previous_py) = centers[previous];
            // Two-zone rule: a body must clear anything foreign, while supports may touch
            // other supports — the same predicate the packing strategies enforce.
            if masks_overlap(
                &previous_mask.body_clearance,
                previous_px,
                previous_py,
                &mask.clearance,
                px,
                py,
            ) || masks_overlap(
                &previous_mask.clearance,
                previous_px,
                previous_py,
                &mask.body_clearance,
                px,
                py,
            ) {
                return Err(format!(
                    "placements {previous} and {index} overlap: a body touches a foreign \
                     structure"
                ));
            }
            if support_cores_overlap(
                previous_mask,
                mask,
                px - previous_px,
                py - previous_py,
            ) {
                return Err(format!(
                    "placements {previous} and {index}: supports overlap by more than {} mm",
                    options.support_overlap_mm.unwrap_or_default()
                ));
            }
        }
    }
    Ok(())
}

/// Unions two footprints' loops into exterior rings (bed-extent loops: first-layer patch
/// plus whole-model silhouette). Both must share the same center — the assembler's model
/// center — which the caller guarantees.
pub fn union_loops(first: &Footprint, second: &Footprint) -> Vec<Vec<Point>> {
    let owned: Vec<Polygon> = first
        .loops
        .iter()
        .chain(second.loops.iter())
        .map(|loop_| polygon_from_loop(loop_))
        .collect();
    let polygons: Vec<&Polygon> = owned.iter().collect();
    let union = unary_union(polygons);
    let mut loops: Vec<Vec<Point>> = Vec::new();
    for polygon in &union.0 {
        let points: Vec<Point> = polygon
            .exterior()
            .coords()
            .map(|coord| Point {
                x: coord.x,
                y: coord.y,
            })
            .collect();
        if points.len() < 3 {
            continue;
        }
        let mut loop_ = points;
        let first_point = loop_[0];
        if loop_.last() != Some(&first_point) {
            loop_.push(first_point);
        }
        loops.push(loop_);
    }
    const MIN_LOOP_AREA_MM2: f64 = 0.01;
    loops.retain(|loop_| polygon_area(loop_).abs() >= MIN_LOOP_AREA_MM2);
    loops.sort_by(|a, b| polygon_area(b).abs().total_cmp(&polygon_area(a).abs()));
    loops
}

/// The loops as hole-free exterior rings: each loop becomes a polygon and their union covers
/// the holes (a hole ring's own polygon sits inside the body's). Comparisons against a
/// footprint's holes treat them as filled, matching the collision masks.
fn filled_exterior_loops(loops: &[Vec<Point>]) -> Vec<Vec<Point>> {
    let polygons: Vec<Polygon> = loops.iter().map(|loop_| polygon_from_loop(loop_)).collect();
    let union = unary_union(polygons.iter());
    union
        .0
        .iter()
        .map(|polygon| {
            polygon
                .exterior()
                .coords()
                .map(|coord| Point {
                    x: coord.x,
                    y: coord.y,
                })
                .collect()
        })
        .filter(|loop_: &Vec<Point>| loop_.len() >= 3)
        .collect()
}

/// Whether every printed pixel of the first-layer `patch` lies within the mesh `silhouette`
/// plus `tolerance_mm`. A prismatic model's walls run along its outline, so its patch only
/// bulges half a line width (~0.25 mm) past it — up to ~1 mm on sharp convex tips, where the
/// nozzle bead miter extends width/2 / sin(θ/2) — while supports and brim wander millimetres
/// beyond. The silhouette's own holes (a keyring hole) don't count as sticking out: the
/// patch's collision mask deliberately fills them, so the comparison runs against the
/// hole-filled silhouette. Contained ⇒ the silhouette is the honest footprint AND rotations
/// stay allowed (packing a locked patch once cost a flat goat plate a full object: 29 instead
/// of 30); sticking out ⇒ only the patch keeps neighbours off the printed supports.
pub fn patch_within_silhouette(
    patch: &Footprint,
    silhouette: &Footprint,
    resolution: f64,
    tolerance_mm: f64,
) -> bool {
    assert!(resolution > 0.0, "resolution must be positive");
    // Fill the silhouette's holes: each loop becomes a hole-free polygon, and their union
    // covers the holes (a hole ring's own polygon sits inside the body's).
    let filled_silhouette_loops: Vec<Vec<Point>> = filled_exterior_loops(&silhouette.loops);
    let bounds_of = |loops: &[Vec<Point>]| {
        let mut bounds = Rect::new(
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for point in loops.iter().flatten() {
            bounds.min_x = bounds.min_x.min(point.x);
            bounds.min_y = bounds.min_y.min(point.y);
            bounds.max_x = bounds.max_x.max(point.x);
            bounds.max_y = bounds.max_y.max(point.y);
        }
        bounds
    };
    let patch_bounds = bounds_of(&patch.loops);
    let silhouette_bounds = bounds_of(&filled_silhouette_loops);
    let min_ix =
        ((patch_bounds.min_x.min(silhouette_bounds.min_x)) / resolution - 0.5).floor() as i32;
    let max_ix =
        ((patch_bounds.max_x.max(silhouette_bounds.max_x)) / resolution - 0.5).ceil() as i32;
    let min_iy =
        ((patch_bounds.min_y.min(silhouette_bounds.min_y)) / resolution - 0.5).floor() as i32;
    let max_iy =
        ((patch_bounds.max_y.max(silhouette_bounds.max_y)) / resolution - 0.5).ceil() as i32;

    let radius = (tolerance_mm / resolution).round() as i32;
    let mut silhouette_pixels: HashSet<(i32, i32)> = HashSet::new();
    for iy in min_iy..=max_iy {
        let y = (f64::from(iy) + 0.5) * resolution;
        for ix in min_ix..=max_ix {
            let x = (f64::from(ix) + 0.5) * resolution;
            if point_in_loops(Point { x, y }, &filled_silhouette_loops) {
                silhouette_pixels.insert((ix, iy));
            }
        }
    }
    let mut tolerated: HashSet<(i32, i32)> = silhouette_pixels.clone();
    if radius > 0 {
        for &(x, y) in &silhouette_pixels {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    tolerated.insert((x + dx, y + dy));
                }
            }
        }
    }
    for iy in min_iy..=max_iy {
        let y = (f64::from(iy) + 0.5) * resolution;
        for ix in min_ix..=max_ix {
            let x = (f64::from(ix) + 0.5) * resolution;
            if point_in_loops(Point { x, y }, &patch.loops) && !tolerated.contains(&(ix, iy)) {
                return false;
            }
        }
    }
    true
}

/// How far the probe-slice `patch` extends past the mesh `silhouette` at thin tips, in mm
/// (0.0 when the patch stays inside). Bed bounds and exclusion keepouts are checked against
/// the silhouette dilated by exactly this amount: the wall-bead miter and Arachne's
/// flow-preserving spikes print past the mesh outline (measured 1.66 mm at a P1S goat's leg
/// tips), and nothing routes the prime line or the bed edge around them. Both footprints
/// must share the model center, which the caller guarantees. Distances grow in square
/// (Chebyshev) rings quantized to `resolution` — the same kernel `patch_within_silhouette`
/// tolerates.
pub fn patch_overshoot_mm(patch: &Footprint, silhouette: &Footprint, resolution: f64) -> f64 {
    assert!(resolution > 0.0, "resolution must be positive");
    let filled_silhouette_loops = filled_exterior_loops(&silhouette.loops);
    let bounds_of = |loops: &[Vec<Point>]| {
        let mut bounds = Rect::new(
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for point in loops.iter().flatten() {
            bounds.min_x = bounds.min_x.min(point.x);
            bounds.min_y = bounds.min_y.min(point.y);
            bounds.max_x = bounds.max_x.max(point.x);
            bounds.max_y = bounds.max_y.max(point.y);
        }
        bounds
    };
    let patch_bounds = bounds_of(&patch.loops);
    let silhouette_bounds = bounds_of(&filled_silhouette_loops);
    let min_ix =
        ((patch_bounds.min_x.min(silhouette_bounds.min_x)) / resolution - 0.5).floor() as i32;
    let max_ix =
        ((patch_bounds.max_x.max(silhouette_bounds.max_x)) / resolution - 0.5).ceil() as i32;
    let min_iy =
        ((patch_bounds.min_y.min(silhouette_bounds.min_y)) / resolution - 0.5).floor() as i32;
    let max_iy =
        ((patch_bounds.max_y.max(silhouette_bounds.max_y)) / resolution - 0.5).ceil() as i32;

    let mut silhouette_pixels: HashSet<(i32, i32)> = HashSet::new();
    let mut patch_pixels: HashSet<(i32, i32)> = HashSet::new();
    for iy in min_iy..=max_iy {
        let y = (f64::from(iy) + 0.5) * resolution;
        for ix in min_ix..=max_ix {
            let x = (f64::from(ix) + 0.5) * resolution;
            if point_in_loops(Point { x, y }, &filled_silhouette_loops) {
                silhouette_pixels.insert((ix, iy));
            }
            if point_in_loops(Point { x, y }, &patch.loops) {
                patch_pixels.insert((ix, iy));
            }
        }
    }
    // Ring-grow the silhouette one Chebyshev pixel at a time until it covers every patch
    // pixel: the ring count at which the last spike is swallowed is the overshoot.
    let mut uncovered: HashSet<(i32, i32)> = patch_pixels
        .difference(&silhouette_pixels)
        .copied()
        .collect();
    if uncovered.is_empty() {
        return 0.0;
    }
    let mut covered = silhouette_pixels;
    let mut rings = 0u32;
    while !uncovered.is_empty() {
        rings += 1;
        let mut grown: HashSet<(i32, i32)> = covered.clone();
        for &(x, y) in &covered {
            for (dx, dy) in [
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                grown.insert((x + dx, y + dy));
            }
        }
        covered = grown;
        uncovered.retain(|pixel| !covered.contains(pixel));
    }
    f64::from(rings) * resolution
}

/// Whether footprint loops physically touch an axis-aligned `rect` (a tower keepout): any
/// loop vertex inside the rect, or any sampled point of the rect inside the loops. The rect
/// is sampled on a ≤2 mm grid — both shapes in the tower-growth check are tens of
/// millimetres across, so a thinner miss is not physical.
pub fn loops_touch_rect(loops: &[Vec<Point>], rect: Rect) -> bool {
    for point in loops.iter().flatten() {
        if point.x >= rect.min_x
            && point.x <= rect.max_x
            && point.y >= rect.min_y
            && point.y <= rect.max_y
        {
            return true;
        }
    }
    let width = rect.max_x - rect.min_x;
    let height = rect.max_y - rect.min_y;
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    let columns = ((width / 2.0).ceil() as usize).max(1);
    let rows = ((height / 2.0).ceil() as usize).max(1);
    for row in 0..=rows {
        let y = rect.min_y + height * f64::from(row as u32) / f64::from(rows as u32);
        for column in 0..=columns {
            let x = rect.min_x + width * f64::from(column as u32) / f64::from(columns as u32);
            if point_in_loops(Point { x, y }, loops) {
                return true;
            }
        }
    }
    false
}

fn polygon_from_loop(loop_: &[Point]) -> Polygon {
    let coords: Vec<Coord> = loop_
        .iter()
        .map(|point| Coord {
            x: point.x,
            y: point.y,
        })
        .collect();
    Polygon::new(LineString::from(coords), Vec::new())
}

/// True when the optional per-plate object cap is set and reached (0 = unlimited).
fn cap_reached(placed: usize, options: &PackOptions) -> bool {
    options.max_objects > 0 && placed >= options.max_objects
}

use geo::{Coord, LineString, Polygon, algorithm::bool_ops::unary_union};
use rand::{Rng, SeedableRng, rngs::SmallRng};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io, path::Path};

pub mod scheduler;
pub mod colored_packer;
pub mod embedded_mesh;
pub mod plate_preview;
pub mod assemble;
pub mod nozzle_clean;

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
        projections.push(Polygon::new(
            LineString::from(ordered.to_vec()),
            Vec::new(),
        ));
    }
    if projections.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "no footprint could be built: all {} triangles project to degenerate (zero-area) \
                 XY shapes (z range {:.3}..{:.3}, z tolerance {:.1e})",
                triangles.len(), mins[2], maxs[2], z_tolerance
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
                                sink.record(
                                    (from_x, from_y),
                                    (to_x, to_y),
                                    line_width,
                                    is_body,
                                );
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
            None => ((patch_min_x + patch_max_x) * 0.5, (patch_min_y + patch_max_y) * 0.5),
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
        Coord { x: x0 + px, y: y0 + py },
        Coord { x: x1 + px, y: y1 + py },
        Coord { x: x1 - px, y: y1 - py },
        Coord { x: x0 - px, y: y0 - py },
        Coord { x: x0 + px, y: y0 + py },
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
    let (min_x, min_y, max_x, max_y) =
        bounds.unwrap_or((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY));
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
    fn from_pixels(pixels: &HashSet<(i32, i32)>) -> Self {
        let min_x = pixels.iter().map(|p| p.0).min().unwrap_or(0);
        let max_x = pixels.iter().map(|p| p.0).max().unwrap_or(0);
        let min_y = pixels.iter().map(|p| p.1).min().unwrap_or(0);
        let max_y = pixels.iter().map(|p| p.1).max().unwrap_or(0);
        let width = (max_x - min_x + 1) as usize;
        let height = (max_y - min_y + 1) as usize;
        let words = width.div_ceil(64);
        let mut rows = vec![vec![0u64; words]; height];
        for &(x, y) in pixels {
            let local_x = (x - min_x) as usize;
            let local_y = (y - min_y) as usize;
            rows[local_y][local_x / 64] |= 1u64 << (local_x % 64);
        }
        Self {
            min_x,
            min_y,
            width,
            height,
            pixel_count: pixels.len(),
            rows,
        }
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
}

pub fn rasterize_footprint(
    footprint: &Footprint,
    angle_deg: f64,
    resolution: f64,
    clearance: f64,
) -> RasterMask {
    assert!(resolution > 0.0);
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

    let min_ix = (bounds.min_x / resolution - 0.5).floor() as i32;
    let max_ix = (bounds.max_x / resolution - 0.5).ceil() as i32;
    let min_iy = (bounds.min_y / resolution - 0.5).floor() as i32;
    let max_iy = (bounds.max_y / resolution - 0.5).ceil() as i32;
    let mut physical_pixels = HashSet::new();
    for iy in min_iy..=max_iy {
        let y = (f64::from(iy) + 0.5) * resolution;
        for ix in min_ix..=max_ix {
            let x = (f64::from(ix) + 0.5) * resolution;
            if point_in_loops(Point { x, y }, &rotated) {
                physical_pixels.insert((ix, iy));
            }
        }
    }

    let radius = (clearance * 0.5 / resolution).ceil() as i32;
    let dilate = |pixels: &HashSet<(i32, i32)>| {
        let mut grown = pixels.clone();
        if radius > 0 {
            for &(x, y) in pixels {
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        if dx * dx + dy * dy <= radius * radius {
                            grown.insert((x + dx, y + dy));
                        }
                    }
                }
            }
        }
        grown
    };
    let clearance_pixels = dilate(&physical_pixels);

    let body_pixels: HashSet<(i32, i32)> = if footprint.body_loops.is_empty() {
        physical_pixels.clone()
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
        let mut body = HashSet::new();
        for iy in min_iy..=max_iy {
            let y = (f64::from(iy) + 0.5) * resolution;
            for ix in min_ix..=max_ix {
                let x = (f64::from(ix) + 0.5) * resolution;
                if point_in_loops(Point { x, y }, &rotated_body) {
                    body.insert((ix, iy));
                }
            }
        }
        if body.is_empty() {
            physical_pixels.clone()
        } else {
            body
        }
    };
    let body_clearance_pixels = dilate(&body_pixels);

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
            let bed_min_ix = (bed_bounds.min_x / resolution - 0.5).floor() as i32;
            let bed_max_ix = (bed_bounds.max_x / resolution - 0.5).ceil() as i32;
            let bed_min_iy = (bed_bounds.min_y / resolution - 0.5).floor() as i32;
            let bed_max_iy = (bed_bounds.max_y / resolution - 0.5).ceil() as i32;
            let mut pixels = physical_pixels.clone();
            for iy in bed_min_iy..=bed_max_iy {
                let y = (f64::from(iy) + 0.5) * resolution;
                for ix in bed_min_ix..=bed_max_ix {
                    let x = (f64::from(ix) + 0.5) * resolution;
                    if pixels.contains(&(ix, iy))
                        || point_in_loops(Point { x, y }, &rotated_bed)
                    {
                        pixels.insert((ix, iy));
                    }
                }
            }
            (pixels, bed_bounds)
        }
    };
    let bed_clearance_pixels = dilate(&bed_physical_pixels);

    RasterMask {
        angle_deg,
        resolution,
        physical: PixelMask::from_pixels(&physical_pixels),
        clearance: PixelMask::from_pixels(&clearance_pixels),
        body_clearance: PixelMask::from_pixels(&body_clearance_pixels),
        bed_physical: PixelMask::from_pixels(&bed_physical_pixels),
        bed_clearance: PixelMask::from_pixels(&bed_clearance_pixels),
        bed_physical_bounds,
        physical_bounds: bounds,
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

/// Occupancy of an in-progress layout under the two-zone rule: a body keeps clearance from
/// anything foreign, while support structures may touch other supports. Placement collides
/// when its full patch (dilated) meets a placed body, or its own body meets any placed full
/// patch — symmetric, so support-on-support contact passes while body-on-anything fails.
struct TwoZoneOccupancy {
    bodies: BitGrid,
    fulls: BitGrid,
}

impl TwoZoneOccupancy {
    fn new(width: usize, height: usize) -> Self {
        Self {
            bodies: BitGrid::new(width, height),
            fulls: BitGrid::new(width, height),
        }
    }

    fn collides(&self, mask: &RasterMask, x: i32, y: i32) -> bool {
        self.bodies.collides(&mask.clearance, x, y)
            || self.fulls.collides(&mask.body_clearance, x, y)
    }

    fn add(&mut self, mask: &RasterMask, x: i32, y: i32) {
        self.bodies.add(&mask.body_clearance, x, y);
        self.fulls.add(&mask.clearance, x, y);
    }

    /// Full-patch contact, used for the greedy heuristic's "hug your neighbours" score.
    fn full_touches(&self, mask: &RasterMask, x: i32, y: i32) -> bool {
        self.fulls.collides(&mask.clearance, x, y)
    }
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
    let (small, sx, sy, other, ox, oy) = if a.pixel_count <= b.pixel_count {
        (a, a_x, a_y, b, b_x, b_y)
    } else {
        (b, b_x, b_y, a, a_x, a_y)
    };
    small
        .pixels()
        .any(|(x, y)| other.contains_local(x + sx - ox, y + sy - oy))
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
    !mask.bed_clearance.pixels().any(|(x, y)| {
        let world_x = center_x + (f64::from(x) + 0.5) * mask.resolution;
        let world_y = center_y + (f64::from(y) + 0.5) * mask.resolution;
        exclusions.iter().any(|r| {
            world_x >= r.min_x && world_x <= r.max_x && world_y >= r.min_y && world_y <= r.max_y
        })
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackOptions {
    pub bed: Rect,
    pub exclusions: Vec<Rect>,
    pub resolution: f64,
    pub clearance: f64,
    pub angle_step_deg: f64,
    /// Number of independent randomized restarts. Each restart packs a full layout from scratch;
    /// `candidates_per_step` positions are sampled per placed object *within* one restart, so
    /// total search cost scales with `attempts × candidates_per_step`, not their ratio.
    pub attempts: usize,
    pub candidates_per_step: usize,
    /// Stop once this many consecutive completed restarts have not improved the best object
    /// count (0 = run all attempts). Dense plates usually plateau long before `attempts` runs
    /// out, so this skips the tail of the search at no quality cost.
    pub stall_attempts: usize,
    /// Hard cap on objects per layout (0 = unlimited). The search itself stops when the bed
    /// is full; a cap is only for deliberately sparse plates or bounded slicing time.
    pub max_objects: usize,
    /// Pack every object at 0°. Tree supports are rebuilt by the slicer in table coordinates,
    /// so a footprint measured at one orientation silently stops matching the printed patch
    /// once the model rotates; a locked orientation keeps every clone identical to the probe.
    pub lock_orientation: bool,
    pub seed: u64,
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
    /// How many restarts actually ran: fewer than `attempts` when the stall limit stopped the
    /// search early.
    pub attempts_run: usize,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug)]
struct GridPlacement {
    mask: usize,
    x: i32,
    y: i32,
}

#[derive(Clone)]
struct BitGrid {
    width: usize,
    height: usize,
    words_per_row: usize,
    rows: Vec<u64>,
}

impl BitGrid {
    fn new(width: usize, height: usize) -> Self {
        let words_per_row = width.div_ceil(64);
        Self {
            width,
            height,
            words_per_row,
            rows: vec![0; words_per_row * height],
        }
    }

    fn collides(&self, mask: &PixelMask, center_x: i32, center_y: i32) -> bool {
        let start_x = center_x + mask.min_x;
        let start_y = center_y + mask.min_y;
        if start_x < 0
            || start_y < 0
            || start_x + mask.width as i32 > self.width as i32
            || start_y + mask.height as i32 > self.height as i32
        {
            return true;
        }
        let destination_word = start_x as usize / 64;
        let shift = start_x as usize % 64;
        for (source_row_index, source_row) in mask.rows.iter().enumerate() {
            let destination_row = (start_y as usize + source_row_index) * self.words_per_row;
            for (source_word_index, &source) in source_row.iter().enumerate() {
                if source == 0 {
                    continue;
                }
                let target = destination_row + destination_word + source_word_index;
                if self.rows[target] & (source << shift) != 0 {
                    return true;
                }
                if shift != 0 {
                    let high = source >> (64 - shift);
                    if high != 0 && self.rows[target + 1] & high != 0 {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn add(&mut self, mask: &PixelMask, center_x: i32, center_y: i32) {
        let start_x = center_x + mask.min_x;
        let start_y = center_y + mask.min_y;
        debug_assert!(start_x >= 0 && start_y >= 0);
        let destination_word = start_x as usize / 64;
        let shift = start_x as usize % 64;
        for (source_row_index, source_row) in mask.rows.iter().enumerate() {
            let destination_row = (start_y as usize + source_row_index) * self.words_per_row;
            for (source_word_index, &source) in source_row.iter().enumerate() {
                if source == 0 {
                    continue;
                }
                let target = destination_row + destination_word + source_word_index;
                self.rows[target] |= source << shift;
                if shift != 0 {
                    let high = source >> (64 - shift);
                    if high != 0 {
                        self.rows[target + 1] |= high;
                    }
                }
            }
        }
    }

    fn fill_rect(&mut self, min_x: i32, min_y: i32, max_x: i32, max_y: i32) {
        for y in min_y.max(0)..max_y.min(self.height as i32) {
            for x in min_x.max(0)..max_x.min(self.width as i32) {
                let index = y as usize * self.words_per_row + x as usize / 64;
                self.rows[index] |= 1u64 << (x as usize % 64);
            }
        }
    }
}

pub fn pack_footprint(footprint: &Footprint, options: &PackOptions) -> PackResult {
    assert!(options.resolution > 0.0);
    assert!(options.angle_step_deg > 0.0 && options.angle_step_deg <= 360.0);
    let angle_count = if options.lock_orientation {
        1
    } else {
        (360.0 / options.angle_step_deg).round() as usize
    };
    let masks: Vec<RasterMask> = (0..angle_count)
        .into_par_iter()
        .map(|index| {
            rasterize_footprint(
                footprint,
                index as f64 * options.angle_step_deg,
                options.resolution,
                options.clearance,
            )
        })
        .collect();
    let attempts = options.attempts.max(1);
    let stall = options.stall_attempts;
    let completed = std::sync::atomic::AtomicUsize::new(0);
    let best_seen = std::sync::atomic::AtomicUsize::new(0);
    let best_attempt = std::sync::atomic::AtomicUsize::new(0);
    let since_improvement = std::sync::atomic::AtomicUsize::new(0);
    let stop_requested = std::sync::atomic::AtomicBool::new(false);
    let progress_interval = (attempts / 20).max(1);
    let best = (0..attempts)
        .into_par_iter()
        .map(|attempt| {
            if stall > 0 && stop_requested.load(std::sync::atomic::Ordering::Relaxed) {
                return Vec::new();
            }
            let result = if attempt % 2 == 0 {
                drop_attempt(&masks, options, attempt)
            } else {
                greedy_attempt(&masks, options, attempt)
            };
            // Object count is the only objective (bbox area is a tiebreak inside
            // better_layout), so an equal-count restart counts as "no improvement" for the
            // stall limit. Attempts complete out of order under rayon; a racy reset/increment
            // pair can only shift the stop by one in-flight restart, which is fine for a
            // heuristic whose tail is being skipped.
            if result.len() > best_seen.fetch_max(result.len(), std::sync::atomic::Ordering::Relaxed)
            {
                best_attempt.store(attempt, std::sync::atomic::Ordering::Relaxed);
                since_improvement.store(0, std::sync::atomic::Ordering::Relaxed);
            } else if stall > 0 {
                since_improvement.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let done = completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if done % progress_interval == 0 || done == attempts {
                eprintln!(
                    "pack_footprint: {done}/{attempts} attempts, best so far: {} objects",
                    best_seen.load(std::sync::atomic::Ordering::Relaxed)
                );
            }
            if stall > 0
                && since_improvement.load(std::sync::atomic::Ordering::Relaxed) >= stall
                && !stop_requested.swap(true, std::sync::atomic::Ordering::Relaxed)
            {
                eprintln!(
                    "pack_footprint: no improvement in the last {stall} attempts; stopping early \
                     (best so far: {} objects)",
                    best_seen.load(std::sync::atomic::Ordering::Relaxed)
                );
            }
            result
        })
        .reduce(Vec::new, |a, b| better_layout(a, b, &masks));
    let attempts_run = completed.load(std::sync::atomic::Ordering::Relaxed);
    eprintln!(
        "pack_footprint: best layout: {} objects (found at attempt {}, {attempts_run}/{attempts} \
         attempts run)",
        best.len(),
        best_attempt.load(std::sync::atomic::Ordering::Relaxed) + 1,
    );

    PackResult {
        placements: best
            .iter()
            .map(|p| PackedPlacement {
                x_mm: options.bed.min_x + f64::from(p.x) * options.resolution,
                y_mm: options.bed.min_y + f64::from(p.y) * options.resolution,
                angle_deg: masks[p.mask].angle_deg,
            })
            .collect(),
        resolution_mm: options.resolution,
        requested_clearance_mm: options.clearance,
        angle_step_deg: options.angle_step_deg,
        attempts,
        attempts_run,
        seed: options.seed,
    }
}

/// Deterministic rectangular-grid baseline: tiles the bed row by row with the footprint's
/// clearance-rasterized bounding box, nudging one pixel at a time around exclusions. Tries the
/// silhouette both ways up when it is not square and keeps the orientation that fits more
/// objects. Small models without intricate silhouettes pack at least as densely as a grid of
/// their boxes — callers run this first and keep whichever of grid/random search holds more
/// objects, which also makes results reproducible whenever the grid wins.
pub fn grid_layout(footprint: &Footprint, options: &PackOptions) -> PackResult {
    assert!(options.resolution > 0.0);
    let mut best: Vec<GridPlacement> = Vec::new();
    let mut best_angle = 0.0f64;
    let orientations: &[f64] = if options.lock_orientation || (footprint.width - footprint.height).abs() < 1e-9 {
        &[0.0]
    } else {
        &[0.0, 90.0]
    };
    for &angle in orientations {
        let mask = rasterize_footprint(footprint, angle, options.resolution, options.clearance);
        let bed_width_pixels =
            ((options.bed.max_x - options.bed.min_x) / options.resolution).ceil() as usize;
        let bed_height_pixels =
            ((options.bed.max_y - options.bed.min_y) / options.resolution).ceil() as usize;
        let margin = [
            mask.bed_clearance.min_x.unsigned_abs(),
            (mask.bed_clearance.min_x + mask.bed_clearance.width as i32).unsigned_abs(),
            mask.bed_clearance.min_y.unsigned_abs(),
            (mask.bed_clearance.min_y + mask.bed_clearance.height as i32).unsigned_abs(),
        ]
        .into_iter()
        .max()
        .unwrap_or(0) as i32;
        let grid_width = bed_width_pixels + 2 * margin as usize;
        let grid_height = bed_height_pixels + 2 * margin as usize;
        let excluded = exclusion_grid(options, grid_width, grid_height, margin);

        // The lattice pitch per axis is the SMALLEST whole-pixel shift where two copies of
        // the mask satisfy the two-zone rule — not the full bounding box. Identical cells on
        // that pitch cannot conflict anywhere on the lattice, but supports of neighbouring
        // cells may legally interlock, which is where the density comes from.
        let pair_collides = |dx: i32, dy: i32| {
            masks_overlap(&mask.clearance, 0, 0, &mask.body_clearance, dx, dy)
                || masks_overlap(&mask.body_clearance, 0, 0, &mask.clearance, dx, dy)
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
        let mut y = -mask.bed_clearance.min_y;
        while y + mask.bed_clearance.min_y + mask.bed_clearance.height as i32
            <= bed_height_pixels as i32
        {
            let mut x = -mask.bed_clearance.min_x;
            while x + mask.bed_clearance.min_x + mask.bed_clearance.width as i32
                <= bed_width_pixels as i32
            {
                // Bed membership follows the patch+silhouette extent; the dilated mask may
                // poke past the bed edge, the physical model may not.
                let on_bed = mask.bed_physical_bounds.min_x + f64::from(x) * options.resolution
                    >= -f64::EPSILON
                    && mask.bed_physical_bounds.max_x + f64::from(x) * options.resolution
                        <= options.bed.max_x - options.bed.min_x + f64::EPSILON
                    && mask.bed_physical_bounds.min_y + f64::from(y) * options.resolution
                        >= -f64::EPSILON
                    && mask.bed_physical_bounds.max_y + f64::from(y) * options.resolution
                        <= options.bed.max_y - options.bed.min_y + f64::EPSILON;
                let (grid_x, grid_y) = (x + margin, y + margin);
                if on_bed && !excluded.collides(&mask.bed_clearance, grid_x, grid_y) {
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

fn grid_dimensions_and_margin(masks: &[RasterMask], options: &PackOptions) -> (usize, usize, i32) {
    let bed_width_pixels =
        ((options.bed.max_x - options.bed.min_x) / options.resolution).ceil() as usize;
    let bed_height_pixels =
        ((options.bed.max_y - options.bed.min_y) / options.resolution).ceil() as usize;
    let margin = masks
        .iter()
        .map(|m| {
            m.clearance
                .min_x
                .unsigned_abs()
                .max((m.clearance.min_x + m.clearance.width as i32).unsigned_abs())
                .max(m.clearance.min_y.unsigned_abs())
                .max((m.clearance.min_y + m.clearance.height as i32).unsigned_abs())
        })
        .max()
        .unwrap_or(8) as i32
        + 4;
    (bed_width_pixels, bed_height_pixels, margin)
}

fn exclusion_grid(options: &PackOptions, width: usize, height: usize, margin: i32) -> BitGrid {
    let mut excluded = BitGrid::new(width, height);
    for rect in &options.exclusions {
        let min_x = ((rect.min_x - options.bed.min_x) / options.resolution).floor() as i32 + margin;
        let min_y = ((rect.min_y - options.bed.min_y) / options.resolution).floor() as i32 + margin;
        let max_x = ((rect.max_x - options.bed.min_x) / options.resolution).ceil() as i32 + margin;
        let max_y = ((rect.max_y - options.bed.min_y) / options.resolution).ceil() as i32 + margin;
        excluded.fill_rect(min_x, min_y, max_x, max_y);
    }
    excluded
}

fn drop_attempt(masks: &[RasterMask], options: &PackOptions, attempt: usize) -> Vec<GridPlacement> {
    let (bed_width_pixels, bed_height_pixels, margin) = grid_dimensions_and_margin(masks, options);
    let grid_width = bed_width_pixels + 2 * margin as usize;
    let grid_height = bed_height_pixels + 2 * margin as usize;
    let mut occupied = TwoZoneOccupancy::new(grid_width, grid_height);
    let excluded = exclusion_grid(options, grid_width, grid_height, margin);
    let mut rng = SmallRng::seed_from_u64(
        options.seed.wrapping_add(0xd1b5_4a32_d192_ed03)
            ^ (attempt as u64).wrapping_mul(0x94d0_49bb_1331_11eb),
    );
    let mut placed: Vec<GridPlacement> = Vec::new();
    loop {
        let mut candidates: Vec<(i64, GridPlacement)> = Vec::new();
        let samples = (options.candidates_per_step / 4).max(120);
        for _ in 0..samples {
            let mask_index = rng.random_range(0..masks.len());
            let mask = &masks[mask_index];
            let min_x = ((-mask.bed_physical_bounds.min_x) / options.resolution).ceil() as i32;
            let max_x = ((options.bed.max_x - options.bed.min_x - mask.bed_physical_bounds.max_x)
                / options.resolution)
                .floor() as i32;
            let min_y = ((-mask.bed_physical_bounds.min_y) / options.resolution).ceil() as i32;
            let max_y = ((options.bed.max_y - options.bed.min_y - mask.bed_physical_bounds.max_y)
                / options.resolution)
                .floor() as i32;
            if min_x > max_x || min_y > max_y {
                continue;
            }
            let x = if rng.random_bool(0.25) {
                let step = rng.random_range(0..=4).min(max_x - min_x);
                if rng.random_bool(0.5) {
                    min_x + step
                } else {
                    max_x - step
                }
            } else {
                rng.random_range(min_x..=max_x)
            };
            let from_top = rng.random_bool(0.12);
            let y_iter: Box<dyn Iterator<Item = i32>> = if from_top {
                Box::new((min_y..=max_y).rev())
            } else {
                Box::new(min_y..=max_y)
            };
            let mut found = None;
            for y in y_iter {
                let gx = x + margin;
                let gy = y + margin;
                if !occupied.collides(mask, gx, gy)
                    && !excluded.collides(&mask.bed_clearance, gx, gy)
                {
                    found = Some(y);
                    break;
                }
            }
            let Some(y) = found else { continue };
            let gx = x + margin;
            let gy = y + margin;
            let contact = [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (1, 1),
                (-1, 1),
                (1, -1),
            ]
            .into_iter()
            .filter(|&(dx, dy)| occupied.full_touches(mask, gx + dx, gy + dy))
            .count() as i64;
            let new_top = placed
                .iter()
                .map(|p| f64::from(p.y) + masks[p.mask].physical_bounds.max_y / options.resolution)
                .chain(std::iter::once(
                    f64::from(y) + mask.physical_bounds.max_y / options.resolution,
                ))
                .fold(f64::NEG_INFINITY, f64::max);
            let edge = i64::from((x - min_x).min(max_x - x));
            let score = -(new_top * 1_000_000.0) as i64 + contact * 50_000 - edge * 20
                + rng.random_range(0..20_000i64);
            candidates.push((
                score,
                GridPlacement {
                    mask: mask_index,
                    x,
                    y,
                },
            ));
        }
        if candidates.is_empty() {
            break;
        }
        candidates.sort_unstable_by_key(|candidate| std::cmp::Reverse(candidate.0));
        let choice_limit = candidates.len().min(12);
        let choice = if placed.len() < 3 {
            rng.random_range(0..choice_limit)
        } else if rng.random_bool(0.75) {
            0
        } else {
            rng.random_range(0..choice_limit)
        };
        let candidate = candidates[choice].1;
        occupied.add(
            &masks[candidate.mask],
            candidate.x + margin,
            candidate.y + margin,
        );
        placed.push(candidate);
        if placed.len() >= 128 {
            break;
        }
    }
    placed
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

fn greedy_attempt(
    masks: &[RasterMask],
    options: &PackOptions,
    attempt: usize,
) -> Vec<GridPlacement> {
    let (bed_width_pixels, bed_height_pixels, margin) = grid_dimensions_and_margin(masks, options);
    let mut occupied = TwoZoneOccupancy::new(
        bed_width_pixels + 2 * margin as usize,
        bed_height_pixels + 2 * margin as usize,
    );
    let excluded = exclusion_grid(
        options,
        bed_width_pixels + 2 * margin as usize,
        bed_height_pixels + 2 * margin as usize,
        margin,
    );
    let mut rng = SmallRng::seed_from_u64(
        options.seed ^ (attempt as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
    );
    let mut placed: Vec<GridPlacement> = Vec::new();
    loop {
        let mut best: Option<(i64, GridPlacement)> = None;
        let samples = options.candidates_per_step.max(32);
        for _ in 0..samples {
            let mask_index = rng.random_range(0..masks.len());
            let mask = &masks[mask_index];
            let min_x = ((-mask.bed_physical_bounds.min_x) / options.resolution).ceil() as i32;
            let max_x = (((options.bed.max_x - options.bed.min_x) - mask.bed_physical_bounds.max_x)
                / options.resolution)
                .floor() as i32;
            let min_y = ((-mask.bed_physical_bounds.min_y) / options.resolution).ceil() as i32;
            let max_y = (((options.bed.max_y - options.bed.min_y) - mask.bed_physical_bounds.max_y)
                / options.resolution)
                .floor() as i32;
            if min_x > max_x || min_y > max_y {
                continue;
            }
            let (x, y) =
                sample_position(&mut rng, mask, masks, &placed, min_x, max_x, min_y, max_y);
            let grid_x = x + margin;
            let grid_y = y + margin;
            if occupied.collides(mask, grid_x, grid_y)
                || excluded.collides(&mask.bed_clearance, grid_x, grid_y)
            {
                continue;
            }
            let contact = [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (1, 1),
                (-1, 1),
                (1, -1),
            ]
            .into_iter()
            .filter(|&(dx, dy)| occupied.full_touches(mask, grid_x + dx, grid_y + dy))
            .count() as i64;
            let center_dx = i64::from(x - (bed_width_pixels as i32 / 2));
            let center_dy = i64::from(y - (bed_height_pixels as i32 / 2));
            let border_distance =
                i64::from((x - min_x).min(max_x - x).min(y - min_y).min(max_y - y));
            let noise = rng.random_range(0..10_000i64);
            let score = contact * 10_000_000
                - border_distance * 2_000
                - (center_dx.abs() + center_dy.abs())
                + noise;
            let candidate = GridPlacement {
                mask: mask_index,
                x,
                y,
            };
            if best
                .as_ref()
                .is_none_or(|(best_score, _)| score > *best_score)
            {
                best = Some((score, candidate));
            }
        }
        let Some((_, candidate)) = best else {
            break;
        };
        occupied.add(
            &masks[candidate.mask],
            candidate.x + margin,
            candidate.y + margin,
        );
        placed.push(candidate);
        if placed.len() >= 128 {
            break;
        }
    }
    placed
}

#[allow(clippy::too_many_arguments)]
fn sample_position(
    rng: &mut SmallRng,
    candidate: &RasterMask,
    masks: &[RasterMask],
    placed: &[GridPlacement],
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
) -> (i32, i32) {
    if placed.is_empty() || rng.random_bool(0.45) {
        if rng.random_bool(0.35) {
            match rng.random_range(0..4) {
                0 => (
                    min_x + rng.random_range(0..=3).min(max_x - min_x),
                    rng.random_range(min_y..=max_y),
                ),
                1 => (
                    max_x - rng.random_range(0..=3).min(max_x - min_x),
                    rng.random_range(min_y..=max_y),
                ),
                2 => (
                    rng.random_range(min_x..=max_x),
                    min_y + rng.random_range(0..=3).min(max_y - min_y),
                ),
                _ => (
                    rng.random_range(min_x..=max_x),
                    max_y - rng.random_range(0..=3).min(max_y - min_y),
                ),
            }
        } else {
            (
                rng.random_range(min_x..=max_x),
                rng.random_range(min_y..=max_y),
            )
        }
    } else {
        let anchor = placed[rng.random_range(0..placed.len())];
        let anchor_mask = &masks[anchor.mask];
        let intrusion = rng.random_range(-12..=40);
        let (x, y) = match rng.random_range(0..4) {
            0 => (
                anchor.x + anchor_mask.clearance.min_x
                    - (candidate.clearance.min_x + candidate.clearance.width as i32)
                    + intrusion,
                anchor.y
                    + rng.random_range(
                        -(candidate.clearance.height as i32)..=anchor_mask.clearance.height as i32,
                    ),
            ),
            1 => (
                anchor.x + anchor_mask.clearance.min_x + anchor_mask.clearance.width as i32
                    - candidate.clearance.min_x
                    - intrusion,
                anchor.y
                    + rng.random_range(
                        -(candidate.clearance.height as i32)..=anchor_mask.clearance.height as i32,
                    ),
            ),
            2 => (
                anchor.x
                    + rng.random_range(
                        -(candidate.clearance.width as i32)..=anchor_mask.clearance.width as i32,
                    ),
                anchor.y + anchor_mask.clearance.min_y
                    - (candidate.clearance.min_y + candidate.clearance.height as i32)
                    + intrusion,
            ),
            _ => (
                anchor.x
                    + rng.random_range(
                        -(candidate.clearance.width as i32)..=anchor_mask.clearance.width as i32,
                    ),
                anchor.y + anchor_mask.clearance.min_y + anchor_mask.clearance.height as i32
                    - candidate.clearance.min_y
                    - intrusion,
            ),
        };
        (x.clamp(min_x, max_x), y.clamp(min_y, max_y))
    }
}

pub fn validate_result(
    footprint: &Footprint,
    options: &PackOptions,
    result: &PackResult,
) -> Result<(), String> {
    let masks: Vec<RasterMask> = result
        .placements
        .iter()
        .map(|p| {
            rasterize_footprint(
                footprint,
                p.angle_deg,
                options.resolution,
                options.clearance,
            )
        })
        .collect();
    for (index, (placement, mask)) in result.placements.iter().zip(&masks).enumerate() {
        let px = ((placement.x_mm - options.bed.min_x) / options.resolution).round() as i32;
        let py = ((placement.y_mm - options.bed.min_y) / options.resolution).round() as i32;
        let translated_bounds = Rect::new(
            placement.x_mm + mask.bed_physical_bounds.min_x,
            placement.y_mm + mask.bed_physical_bounds.min_y,
            placement.x_mm + mask.bed_physical_bounds.max_x,
            placement.y_mm + mask.bed_physical_bounds.max_y,
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
        for exclusion in &options.exclusions {
            if mask.bed_clearance.pixels().any(|(x, y)| {
                let wx = placement.x_mm + (f64::from(x) + 0.5) * options.resolution;
                let wy = placement.y_mm + (f64::from(y) + 0.5) * options.resolution;
                wx >= exclusion.min_x
                    && wx <= exclusion.max_x
                    && wy >= exclusion.min_y
                    && wy <= exclusion.max_y
            }) {
                return Err(format!("placement {index} intersects exclusion"));
            }
        }
        for (previous, previous_mask) in masks.iter().enumerate().take(index) {
            let previous_placement = &result.placements[previous];
            let previous_px =
                ((previous_placement.x_mm - options.bed.min_x) / options.resolution).round() as i32;
            let previous_py =
                ((previous_placement.y_mm - options.bed.min_y) / options.resolution).round() as i32;
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
    let min_ix = ((patch_bounds.min_x.min(silhouette_bounds.min_x)) / resolution - 0.5).floor()
        as i32;
    let max_ix = ((patch_bounds.max_x.max(silhouette_bounds.max_x)) / resolution - 0.5).ceil()
        as i32;
    let min_iy = ((patch_bounds.min_y.min(silhouette_bounds.min_y)) / resolution - 0.5).floor()
        as i32;
    let max_iy = ((patch_bounds.max_y.max(silhouette_bounds.max_y)) / resolution - 0.5).ceil()
        as i32;

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
    let min_ix = ((patch_bounds.min_x.min(silhouette_bounds.min_x)) / resolution - 0.5).floor()
        as i32;
    let max_ix = ((patch_bounds.max_x.max(silhouette_bounds.max_x)) / resolution - 0.5).ceil()
        as i32;
    let min_iy = ((patch_bounds.min_y.min(silhouette_bounds.min_y)) / resolution - 0.5).floor()
        as i32;
    let max_iy = ((patch_bounds.max_y.max(silhouette_bounds.max_y)) / resolution - 0.5).ceil()
        as i32;

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
    let mut uncovered: HashSet<(i32, i32)> =
        patch_pixels.difference(&silhouette_pixels).copied().collect();
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

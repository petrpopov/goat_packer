use rand::{Rng, SeedableRng, rngs::SmallRng};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::Path,
};

pub mod scheduler;
pub mod colored_packer;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Footprint {
    pub loops: Vec<Vec<Point>>,
    pub width: f64,
    pub height: f64,
    pub min_z: f64,
    pub max_z: f64,
    pub triangle_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
struct Key(i64, i64);

impl Key {
    const SCALE: f64 = 100_000.0;

    fn new(x: f32, y: f32) -> Self {
        Self(
            (f64::from(x) * Self::SCALE).round() as i64,
            (f64::from(y) * Self::SCALE).round() as i64,
        )
    }

    fn point_centered(self, center_x: f64, center_y: f64) -> Point {
        Point {
            x: self.0 as f64 / Self::SCALE - center_x,
            y: self.1 as f64 / Self::SCALE - center_y,
        }
    }
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
    let mut edge_counts: HashMap<(Key, Key), usize> = HashMap::new();
    for triangle in &triangles {
        if triangle
            .iter()
            .all(|vertex| (f64::from(vertex[2]) - mins[2]).abs() <= z_tolerance)
        {
            let vertices = triangle.map(|v| Key::new(v[0], v[1]));
            for edge_index in 0..3 {
                let a = vertices[edge_index];
                let b = vertices[(edge_index + 1) % 3];
                let edge = if a <= b { (a, b) } else { (b, a) };
                *edge_counts.entry(edge).or_default() += 1;
            }
        }
    }

    let boundary: Vec<(Key, Key)> = edge_counts
        .into_iter()
        .filter_map(|(edge, count)| (count % 2 == 1).then_some(edge))
        .collect();
    if boundary.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no flat bottom boundary found at minimum Z",
        ));
    }

    let mut adjacency: HashMap<Key, Vec<Key>> = HashMap::new();
    for &(a, b) in &boundary {
        adjacency.entry(a).or_default().push(b);
        adjacency.entry(b).or_default().push(a);
    }
    if let Some((vertex, neighbours)) = adjacency.iter().find(|(_, n)| n.len() != 2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "bottom boundary is non-manifold near ({}, {}): degree {}",
                vertex.0,
                vertex.1,
                neighbours.len()
            ),
        ));
    }

    let mut unused: HashSet<(Key, Key)> = boundary.into_iter().collect();
    let center_x = (mins[0] + maxs[0]) * 0.5;
    let center_y = (mins[1] + maxs[1]) * 0.5;
    let mut loops: Vec<Vec<Point>> = Vec::new();
    while let Some(&(start_a, start_b)) = unused.iter().next() {
        let mut keys = vec![start_a];
        let mut previous = start_a;
        let mut current = start_b;
        remove_edge(&mut unused, start_a, start_b);
        while current != start_a {
            keys.push(current);
            let neighbours = &adjacency[&current];
            let next = if neighbours[0] != previous {
                neighbours[0]
            } else {
                neighbours[1]
            };
            if !remove_edge(&mut unused, current, next) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "bottom boundary contains a broken or self-repeated loop",
                ));
            }
            previous = current;
            current = next;
        }
        keys.push(start_a);
        loops.push(
            keys.into_iter()
                .map(|key| key.point_centered(center_x, center_y))
                .collect::<Vec<Point>>(),
        );
    }

    loops.sort_by(|a, b| polygon_area(b).abs().total_cmp(&polygon_area(a).abs()));
    Ok(Footprint {
        loops,
        width: maxs[0] - mins[0],
        height: maxs[1] - mins[1],
        min_z: mins[2],
        max_z: maxs[2],
        triangle_count,
    })
}

fn remove_edge(unused: &mut HashSet<(Key, Key)>, a: Key, b: Key) -> bool {
    let edge = if a <= b { (a, b) } else { (b, a) };
    unused.remove(&edge)
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
    let mut clearance_pixels = physical_pixels.clone();
    if radius > 0 {
        for &(x, y) in &physical_pixels {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx * dx + dy * dy <= radius * radius {
                        clearance_pixels.insert((x + dx, y + dy));
                    }
                }
            }
        }
    }

    RasterMask {
        angle_deg,
        resolution,
        physical: PixelMask::from_pixels(&physical_pixels),
        clearance: PixelMask::from_pixels(&clearance_pixels),
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
    let bounds = mask.physical_bounds;
    if center_x + bounds.min_x < bed.min_x
        || center_y + bounds.min_y < bed.min_y
        || center_x + bounds.max_x > bed.max_x
        || center_y + bounds.max_y > bed.max_y
    {
        return false;
    }
    !mask.clearance.pixels().any(|(x, y)| {
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
    let angle_count = (360.0 / options.angle_step_deg).round() as usize;
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
    let mut occupied = BitGrid::new(grid_width, grid_height);
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
            let min_x = ((-mask.physical_bounds.min_x) / options.resolution).ceil() as i32;
            let max_x = ((options.bed.max_x - options.bed.min_x - mask.physical_bounds.max_x)
                / options.resolution)
                .floor() as i32;
            let min_y = ((-mask.physical_bounds.min_y) / options.resolution).ceil() as i32;
            let max_y = ((options.bed.max_y - options.bed.min_y - mask.physical_bounds.max_y)
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
                if !occupied.collides(&mask.clearance, gx, gy)
                    && !excluded.collides(&mask.clearance, gx, gy)
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
            .filter(|&(dx, dy)| occupied.collides(&mask.clearance, gx + dx, gy + dy))
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
            &masks[candidate.mask].clearance,
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
    let mut occupied = BitGrid::new(
        bed_width_pixels + 2 * margin as usize,
        bed_height_pixels + 2 * margin as usize,
    );
    let excluded = exclusion_grid(options, occupied.width, occupied.height, margin);
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
            let min_x = ((-mask.physical_bounds.min_x) / options.resolution).ceil() as i32;
            let max_x = (((options.bed.max_x - options.bed.min_x) - mask.physical_bounds.max_x)
                / options.resolution)
                .floor() as i32;
            let min_y = ((-mask.physical_bounds.min_y) / options.resolution).ceil() as i32;
            let max_y = (((options.bed.max_y - options.bed.min_y) - mask.physical_bounds.max_y)
                / options.resolution)
                .floor() as i32;
            if min_x > max_x || min_y > max_y {
                continue;
            }
            let (x, y) =
                sample_position(&mut rng, mask, masks, &placed, min_x, max_x, min_y, max_y);
            let grid_x = x + margin;
            let grid_y = y + margin;
            if occupied.collides(&mask.clearance, grid_x, grid_y)
                || excluded.collides(&mask.clearance, grid_x, grid_y)
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
            .filter(|&(dx, dy)| occupied.collides(&mask.clearance, grid_x + dx, grid_y + dy))
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
            &masks[candidate.mask].clearance,
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
            placement.x_mm + mask.physical_bounds.min_x,
            placement.y_mm + mask.physical_bounds.min_y,
            placement.x_mm + mask.physical_bounds.max_x,
            placement.y_mm + mask.physical_bounds.max_y,
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
            if mask.clearance.pixels().any(|(x, y)| {
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
            if masks_overlap(
                &previous_mask.clearance,
                previous_px,
                previous_py,
                &mask.clearance,
                px,
                py,
            ) {
                return Err(format!("placements {previous} and {index} overlap"));
            }
        }
    }
    Ok(())
}

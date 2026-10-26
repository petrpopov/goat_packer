//! Deterministic packing in a geometry-derived frame. No-fit rasters describe every
//! forbidden relative integer translation, so a search scans free *bits*, not collisions.
use super::*;
use geo::{ConvexHull, MultiPoint};

pub(super) struct Frame {
    pub footprint: Footprint,
    angle: f64,
    center: Point,
}

fn rotate(p: Point, angle: f64) -> Point {
    let (s, c) = angle.to_radians().sin_cos();
    Point {
        x: p.x * c - p.y * s,
        y: p.x * s + p.y * c,
    }
}

impl Frame {
    pub fn new(fp: &Footprint, locked: bool) -> Self {
        if locked {
            return Self {
                footprint: fp.clone(),
                angle: 0.0,
                center: Point { x: 0.0, y: 0.0 },
            };
        }
        let points: Vec<Point> = fp
            .loops
            .iter()
            .chain(&fp.body_loops)
            .chain(fp.bed_loops.iter().flatten())
            .flatten()
            .copied()
            .collect();
        let hull =
            MultiPoint::from(points.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>()).convex_hull();
        let vertices: Vec<Point> = hull
            .exterior()
            .0
            .iter()
            .map(|p| Point { x: p.x, y: p.y })
            .collect();
        let quant = |v: f64| (v * 1e6).round() as i64;
        let mut best = None;
        for edge in vertices.windows(2) {
            let angle = -(edge[1].y - edge[0].y)
                .atan2(edge[1].x - edge[0].x)
                .to_degrees();
            let rotated: Vec<_> = vertices.iter().map(|&p| rotate(p, angle)).collect();
            let min_x = rotated.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
            let max_x = rotated
                .iter()
                .map(|p| p.x)
                .fold(f64::NEG_INFINITY, f64::max);
            let min_y = rotated.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
            let max_y = rotated
                .iter()
                .map(|p| p.y)
                .fold(f64::NEG_INFINITY, f64::max);
            let (w, h) = (max_x - min_x, max_y - min_y);
            let center = Point {
                x: (min_x + max_x) / 2.0,
                y: (min_y + max_y) / 2.0,
            };
            // Prefer the long side horizontal; resolve half-turn/symmetry ties using geometry,
            // never vertex order, winding, absolute coordinates, or the input angle.
            let mut signature: Vec<_> = points
                .iter()
                .map(|&p| {
                    let q = rotate(p, angle);
                    (quant(q.x - center.x), quant(q.y - center.y))
                })
                .collect();
            signature.sort_unstable();
            signature.dedup();
            let key = (quant(w * h), quant(-w), signature);
            if best.as_ref().is_none_or(|(old, _, _)| key < *old) {
                best = Some((key, angle, center));
            }
        }
        let (_, angle, rotated_center) = best.expect("nondegenerate footprint");
        let center = rotate(rotated_center, -angle);
        let transform = |loops: &[Vec<Point>]| {
            loops
                .iter()
                .map(|ring| {
                    ring.iter()
                        .map(|&p| {
                            let q = rotate(
                                Point {
                                    x: p.x - center.x,
                                    y: p.y - center.y,
                                },
                                angle,
                            );
                            // Numerical noise below a tenth of a micron must not move a raster boundary.
                            Point {
                                x: (q.x * 1e7).round() / 1e7,
                                y: (q.y * 1e7).round() / 1e7,
                            }
                        })
                        .collect()
                })
                .collect()
        };
        let mut footprint = fp.clone();
        footprint.loops = transform(&fp.loops);
        footprint.body_loops = transform(&fp.body_loops);
        footprint.bed_loops = fp.bed_loops.as_ref().map(|l| transform(l));
        footprint.width = footprint
            .loops
            .iter()
            .flatten()
            .map(|p| p.x)
            .fold(f64::NEG_INFINITY, f64::max)
            - footprint
                .loops
                .iter()
                .flatten()
                .map(|p| p.x)
                .fold(f64::INFINITY, f64::min);
        footprint.height = footprint
            .loops
            .iter()
            .flatten()
            .map(|p| p.y)
            .fold(f64::NEG_INFINITY, f64::max)
            - footprint
                .loops
                .iter()
                .flatten()
                .map(|p| p.y)
                .fold(f64::INFINITY, f64::min);
        Self {
            footprint,
            angle,
            center,
        }
    }

    pub fn to_input(&self, p: &PackedPlacement) -> PackedPlacement {
        let angle = (p.angle_deg + self.angle).rem_euclid(360.0);
        let offset = rotate(self.center, angle);
        PackedPlacement {
            x_mm: p.x_mm - offset.x,
            y_mm: p.y_mm - offset.y,
            angle_deg: angle,
        }
    }

    pub fn to_canonical(&self, p: &PackedPlacement) -> PackedPlacement {
        let offset = rotate(self.center, p.angle_deg);
        PackedPlacement {
            x_mm: p.x_mm + offset.x,
            y_mm: p.y_mm + offset.y,
            angle_deg: ((p.angle_deg - self.angle).rem_euclid(360.0) * 1e9).round() / 1e9,
        }
    }
}

/// All integer translations of B relative to A that overlap at least one occupied pixel.
/// Span difference: [a,b) - [c,d) = [a-d+1,b-c). Holes remain holes.
/// The third pair (support cores) exists only with `support_overlap_mm`; a side without a
/// core has no support zone, and the two-zone pairs already cover it.
fn no_fit(a: &RasterMask, b: &RasterMask) -> SpanRaster {
    let mut pairs = vec![
        (&a.clearance, &b.body_clearance),
        (&a.body_clearance, &b.clearance),
    ];
    if let (Some(core_a), Some(core_b)) = (&a.support_core, &b.support_core) {
        pairs.push((core_a, core_b));
    }
    let min_y = pairs
        .iter()
        .map(|(a, b)| a.min_y - b.min_y - b.height as i32 + 1)
        .min()
        .unwrap();
    let max_y = pairs
        .iter()
        .map(|(a, b)| a.min_y + a.height as i32 - b.min_y)
        .max()
        .unwrap();
    let mut rows = vec![Vec::new(); (max_y - min_y) as usize];
    for (index, &(a, b)) in pairs.iter().enumerate() {
        if index == 1
            && pairs[0].0.min_x == a.min_x
            && pairs[0].0.min_y == a.min_y
            && pairs[0].1.min_x == b.min_x
            && pairs[0].1.min_y == b.min_y
            && pairs[0].0.rows == a.rows
            && pairs[0].1.rows == b.rows
        {
            continue;
        }
        let ar: Vec<_> = (0..a.height).map(|y| a.row_spans(y)).collect();
        let br: Vec<_> = (0..b.height).map(|y| b.row_spans(y)).collect();
        for (ay, spans_a) in ar.iter().enumerate() {
            for (by, spans_b) in br.iter().enumerate() {
                let row = &mut rows[(a.min_y + ay as i32 - b.min_y - by as i32 - min_y) as usize];
                for &(a0, a1) in spans_a {
                    for &(b0, b1) in spans_b {
                        row.push((a.min_x + a0 - b.min_x - b1 + 1, a.min_x + a1 - b.min_x - b0));
                    }
                }
            }
        }
    }
    for row in &mut rows {
        merge_spans(row);
    }
    SpanRaster { min_y, rows }
}

#[derive(Clone)]
struct FreeCenters {
    min_x: i32,
    min_y: i32,
    width: usize,
    rows: Vec<Vec<u64>>,
}

impl FreeCenters {
    fn new(mask: &RasterMask, options: &PackOptions) -> Self {
        let r = options.resolution;
        let b = mask.bed_physical_bounds;
        let min_x = (-b.min_x / r - 1e-9).ceil() as i32;
        let min_y = (-b.min_y / r - 1e-9).ceil() as i32;
        let max_x = ((options.bed.max_x - options.bed.min_x - b.max_x) / r + 1e-9).floor() as i32;
        let max_y = ((options.bed.max_y - options.bed.min_y - b.max_y) / r + 1e-9).floor() as i32;
        let width = (max_x - min_x + 1).max(0) as usize;
        let height = (max_y - min_y + 1).max(0) as usize;
        let mut this = Self {
            min_x,
            min_y,
            width,
            rows: vec![vec![0; width.div_ceil(64)]; height],
        };
        for row in &mut this.rows {
            set_bit_range(row, 0, width);
        }
        for exclusion in &options.exclusions {
            let lo_x = ((exclusion.min_x - options.bed.min_x - b.max_x - options.clearance) / r)
                .floor() as i32
                + 1;
            let hi_x = ((exclusion.max_x - options.bed.min_x - b.min_x + options.clearance) / r)
                .ceil() as i32;
            let lo_y = ((exclusion.min_y - options.bed.min_y - b.max_y - options.clearance) / r)
                .floor() as i32
                + 1;
            let hi_y = ((exclusion.max_y - options.bed.min_y - b.min_y + options.clearance) / r)
                .ceil() as i32;
            for y in lo_y.max(min_y)..hi_y.min(max_y + 1) {
                this.clear(y, lo_x, hi_x);
            }
        }
        this
    }

    fn clear(&mut self, y: i32, start: i32, end: i32) {
        let index = y - self.min_y;
        if index < 0 || index as usize >= self.rows.len() {
            return;
        }
        let start = (start - self.min_x).clamp(0, self.width as i32) as usize;
        let end = (end - self.min_x).clamp(0, self.width as i32) as usize;
        if start >= end {
            return;
        }
        let row = &mut self.rows[index as usize];
        let first = start / 64;
        let last = (end - 1) / 64;
        let low = !0u64 << (start % 64);
        let high = !0u64 >> (63 - (end - 1) % 64);
        if first == last {
            row[first] &= !(low & high);
        } else {
            row[first] &= !low;
            row[first + 1..last].fill(0);
            row[last] &= !high;
        }
    }

    fn exclude(&mut self, nfp: &SpanRaster, x: i32, y: i32) {
        for (i, spans) in nfp.rows.iter().enumerate() {
            for &(a, b) in spans {
                self.clear(y + nfp.min_y + i as i32, x + a, x + b);
            }
        }
    }

    fn first(&self, reverse_x: bool, reverse_y: bool, phase: usize) -> Option<(i32, i32)> {
        for k in 0..self.rows.len() {
            let y = if reverse_y {
                self.rows.len() - 1 - k
            } else {
                k
            };
            let row = &self.rows[y];
            for j in 0..row.len() {
                let index = if reverse_x { row.len() - 1 - j } else { j };
                let skip = [0, 2, 1, 3][phase % 4] * self.width / 4;
                let lo = if reverse_x { 0 } else { skip };
                let hi = if reverse_x {
                    self.width - skip
                } else {
                    self.width
                };
                let word_lo = lo.saturating_sub(index * 64).min(64);
                let word_hi = hi.saturating_sub(index * 64).min(64);
                let word = if word_lo >= word_hi {
                    0
                } else {
                    row[index] & (!0u64 << word_lo) & (!0u64 >> (64 - word_hi))
                };
                if word != 0 {
                    let bit = if reverse_x {
                        63 - word.leading_zeros()
                    } else {
                        word.trailing_zeros()
                    };
                    return Some((
                        self.min_x + (index * 64 + bit as usize) as i32,
                        self.min_y + y as i32,
                    ));
                }
            }
        }
        None
    }
}

fn pair_search(
    masks: &[RasterMask],
    options: &PackOptions,
    variants: usize,
) -> (Vec<GridPlacement>, (usize, f64)) {
    let nfps: Vec<Vec<_>> = masks
        .par_iter()
        .map(|a| masks.iter().map(|b| no_fit(a, b)).collect())
        .collect();
    let initial: Vec<_> = masks.iter().map(|m| FreeCenters::new(m, options)).collect();
    let mut best = Vec::new();
    let mut baseline = (0, f64::INFINITY);
    for variant in 0..variants {
        let mut free = initial.clone();
        let mut placed = Vec::new();
        loop {
            // Sweep from a fixed corner, or cycle around the four corners. The latter avoids
            // greedy height traps such as four 6x4 rectangles around a 2x2 central hole.
            let corner = if (variant / (4 * masks.len())).is_multiple_of(2) {
                variant % 4
            } else {
                (variant % 4) ^ [0, 1, 3, 2][placed.len() % 4]
            };
            let reverse_x = corner & 1 != 0;
            let reverse_y = corner & 2 != 0;
            let candidate = free
                .iter()
                .enumerate()
                .filter_map(|(i, f)| {
                    let phase = if placed.is_empty() {
                        variant / (8 * masks.len())
                    } else {
                        0
                    };
                    let (x, y) = f.first(reverse_x, reverse_y, phase)?;
                    let b = masks[i].bed_physical_bounds;
                    let top = if reverse_y {
                        -f64::from(y) - b.min_y / options.resolution
                    } else {
                        f64::from(y) + b.max_y / options.resolution
                    };
                    let side = if reverse_x { -x } else { x };
                    Some((
                        (top * 1e6).round() as i64,
                        side,
                        i,
                        GridPlacement { mask: i, x, y },
                    ))
                })
                .min_by(|a, b| {
                    if placed.is_empty() {
                        let first = (variant / 4) % masks.len();
                        let rank = |i| (i + masks.len() - first) % masks.len();
                        rank(a.2)
                            .cmp(&rank(b.2))
                            .then(a.0.cmp(&b.0))
                            .then(a.1.cmp(&b.1))
                    } else {
                        (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2))
                    }
                });
            let Some((_, _, _, p)) = candidate else {
                break;
            };
            for (i, f) in free.iter_mut().enumerate() {
                f.exclude(&nfps[p.mask][i], p.x, p.y);
            }
            placed.push(p);
            if cap_reached(placed.len(), options) {
                break;
            }
        }
        if variant == 0 {
            baseline = (placed.len(), layout_bbox_area(&placed, masks));
        }
        best = better_layout(best, placed, masks);
    }
    (best, baseline)
}

pub(super) fn search(fp: &Footprint, options: &PackOptions) -> PackResult {
    let frame = Frame::new(fp, options.lock_orientation);
    let count = if options.lock_orientation {
        1
    } else {
        (180.0 / options.angle_step_deg).ceil().max(1.0) as usize
    };
    let variants = options.attempts.max(1).div_ceil(count).clamp(1, 64);
    let results: Vec<_> = (0..count)
        .into_par_iter()
        .map(|i| {
            let angle = i as f64 * options.angle_step_deg;
            let mut masks = vec![rasterize_footprint_with_support(
                &frame.footprint,
                angle,
                options.resolution,
                options.clearance,
                options.support_overlap_mm,
            )];
            if !options.lock_orientation {
                masks.push(rasterize_footprint_with_support(
                    &frame.footprint,
                    angle + 180.0,
                    options.resolution,
                    options.clearance,
                    options.support_overlap_mm,
                ));
            }
            let (best, baseline) = pair_search(&masks, options, variants);
            let area = layout_bbox_area(&best, &masks);
            let placements: Vec<_> = best
                .iter()
                .map(|p| PackedPlacement {
                    x_mm: options.bed.min_x + f64::from(p.x) * options.resolution,
                    y_mm: options.bed.min_y + f64::from(p.y) * options.resolution,
                    angle_deg: masks[p.mask].angle_deg,
                })
                .collect();
            (placements, area, baseline)
        })
        .collect();
    let mut ranked: Vec<_> = results.iter().enumerate().collect();
    ranked.sort_by(|(ia, a), (ib, b)| {
        b.0.len()
            .cmp(&a.0.len())
            .then(a.1.total_cmp(&b.1))
            .then(ia.cmp(ib))
    });
    let mut placements = ranked[0].1.0.clone();
    // Shortlisting uses the first sweep only, so increasing effort never removes a previously
    // searched mixed palette. Together with prefix-ordered sweeps this makes effort monotone.
    ranked.sort_by(|(ia, a), (ib, b)| {
        b.2.0
            .cmp(&a.2.0)
            .then(a.2.1.total_cmp(&b.2.1))
            .then(ia.cmp(ib))
    });
    let mut attempts_run = count * variants;
    // Let the strongest families intermix rather than restricting the whole plate to a
    // half-turn pair. Also include orthogonal orientations: rectangles can need a windmill
    // layout even when all single-orientation runs tie. Only a bounded local palette needs
    // quadratic no-fit maps; the full angle grid never incurs an O(angles^2) allocation.
    if !options.lock_orientation && options.attempts > count {
        let mut angles: Vec<f64> = Vec::new();
        for (index, _) in ranked.iter().take(4) {
            let base = *index as f64 * options.angle_step_deg;
            for offset in [0.0, 90.0, 180.0, 270.0] {
                let angle = (base + offset).rem_euclid(360.0);
                if !angles.iter().any(|&a| (a - angle).abs() < 1e-7) {
                    angles.push(angle);
                }
            }
        }
        let masks: Vec<_> = angles
            .iter()
            .map(|&a| {
                rasterize_footprint_with_support(
                    &frame.footprint,
                    a,
                    options.resolution,
                    options.clearance,
                    options.support_overlap_mm,
                )
            })
            .collect();
        let mixed_variants = (options.attempts - count).min(8 * masks.len());
        let (mixed, _) = pair_search(&masks, options, mixed_variants);
        attempts_run += mixed_variants;
        if mixed.len() > placements.len() {
            placements = mixed
                .iter()
                .map(|p| PackedPlacement {
                    x_mm: options.bed.min_x + f64::from(p.x) * options.resolution,
                    y_mm: options.bed.min_y + f64::from(p.y) * options.resolution,
                    angle_deg: masks[p.mask].angle_deg,
                })
                .collect();
        }
    }
    let grid = grid_layout_inner(&frame.footprint, options);
    if grid.placements.len() > placements.len() {
        placements = grid.placements;
    }
    eprintln!(
        "pack_footprint: {attempts_run}/{attempts_run} attempts, best so far: {} objects",
        placements.len()
    );
    PackResult {
        placements: placements.iter().map(|p| frame.to_input(p)).collect(),
        resolution_mm: options.resolution,
        requested_clearance_mm: options.clearance,
        angle_step_deg: options.angle_step_deg,
        attempts: options.attempts,
        attempts_run,
        seed: options.seed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> Footprint {
        let ring = |points: &[(f64, f64)]| {
            let mut ring: Vec<_> = points.iter().map(|&(x, y)| Point { x, y }).collect();
            ring.push(ring[0]);
            ring
        };
        Footprint {
            loops: vec![
                ring(&[
                    (-3., -2.),
                    (3., -2.),
                    (3., -1.),
                    (0., -1.),
                    (0., 2.),
                    (-3., 2.),
                ]),
                ring(&[(-2.5, -0.5), (-1., -0.5), (-1., 1.), (-2.5, 1.)]),
            ],
            body_loops: vec![ring(&[(-3., -2.), (3., -2.), (3., -1.5), (-3., -1.5)])],
            bed_loops: None,
            width: 6.,
            height: 4.,
            min_z: 0.,
            max_z: 1.,
            triangle_count: 2,
        }
    }

    #[test]
    fn no_fit_matches_brute_force_two_zone_collisions_at_every_shift() {
        let fp = shape();
        for angle in [0., 13., 90., 180.] {
            let a = rasterize_footprint(&fp, 0., 0.5, 0.5);
            let b = rasterize_footprint(&fp, angle, 0.5, 0.5);
            let nfp = no_fit(&a, &b);
            for y in -24..=24 {
                for x in -24..=24 {
                    let expected = masks_overlap(&a.clearance, 0, 0, &b.body_clearance, x, y)
                        || masks_overlap(&a.body_clearance, 0, 0, &b.clearance, x, y);
                    let actual = nfp.row(y).iter().any(|&(lo, hi)| lo <= x && x < hi);
                    assert_eq!(actual, expected, "angle={angle}, shift={x},{y}");
                }
            }
        }
    }

    #[test]
    fn scanline_and_dilation_match_original_point_sampler() {
        let fp = shape();
        for angle in [0., 2., 13., 45., 90., 137., 180.] {
            let loops: Vec<Vec<_>> = fp
                .loops
                .iter()
                .map(|l| l.iter().map(|&p| rotate(p, angle)).collect())
                .collect();
            for resolution in [0.25, 0.5, 1.] {
                let pixels: HashSet<_> = (-32..32)
                    .flat_map(|y| (-32..32).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        point_in_loops(
                            Point {
                                x: (f64::from(x) + 0.5) * resolution,
                                y: (f64::from(y) + 0.5) * resolution,
                            },
                            &loops,
                        )
                    })
                    .collect();
                let spans = SpanRaster::scanline(&loops, resolution, (-32, 31, -32, 31));
                assert_eq!(
                    PixelMask::from_spans(&spans)
                        .pixels()
                        .collect::<HashSet<_>>(),
                    pixels
                );
                for radius in [0i32, 1, 2, 3] {
                    let mut grown = HashSet::new();
                    for &(x, y) in &pixels {
                        for dy in -radius..=radius {
                            for dx in -radius..=radius {
                                if dx * dx + dy * dy <= radius * radius {
                                    grown.insert((x + dx, y + dy));
                                }
                            }
                        }
                    }
                    assert_eq!(
                        PixelMask::from_spans(&spans.dilate(radius))
                            .pixels()
                            .collect::<HashSet<_>>(),
                        grown
                    );
                }
            }
        }
    }

    #[test]
    fn no_fit_with_support_cores_matches_brute_force_three_pair_collisions() {
        let fp = shape();
        for overlap in [0.0, 0.5, 1.5] {
            for angle in [0., 13., 90., 180.] {
                let a = rasterize_footprint_with_support(&fp, 0., 0.5, 0.5, Some(overlap));
                let b = rasterize_footprint_with_support(&fp, angle, 0.5, 0.5, Some(overlap));
                assert!(a.support_core.is_some() && b.support_core.is_some());
                let nfp = no_fit(&a, &b);
                for y in -24..=24 {
                    for x in -24..=24 {
                        let expected = masks_overlap(&a.clearance, 0, 0, &b.body_clearance, x, y)
                            || masks_overlap(&a.body_clearance, 0, 0, &b.clearance, x, y)
                            || masks_overlap(
                                a.support_core.as_ref().unwrap(),
                                0,
                                0,
                                b.support_core.as_ref().unwrap(),
                                x,
                                y,
                            );
                        let actual = nfp.row(y).iter().any(|&(lo, hi)| lo <= x && x < hi);
                        assert_eq!(actual, expected, "X={overlap} angle={angle} shift={x},{y}");
                    }
                }
            }
        }
    }

    #[test]
    fn support_core_is_absent_without_the_option_or_a_support_zone() {
        let fp = shape();
        assert!(rasterize_footprint(&fp, 0., 0.5, 0.5).support_core.is_none());
        let mut bodyless = fp.clone();
        bodyless.body_loops.clear();
        assert!(
            rasterize_footprint_with_support(&bodyless, 0., 0.5, 0.5, Some(0.0))
                .support_core
                .is_none()
        );
        // X = 0: the core is exactly the printed patch (supports may touch, not overlap).
        let mask = rasterize_footprint_with_support(&fp, 0., 0.5, 0.5, Some(0.0));
        let core = mask.support_core.unwrap();
        assert_eq!(
            core.pixels().collect::<HashSet<_>>(),
            mask.physical.pixels().collect::<HashSet<_>>()
        );
        // X = 1 at 0.5 mm pixels: eroded by exactly one pixel.
        let eroded = rasterize_footprint_with_support(&fp, 0., 0.5, 0.5, Some(1.0));
        assert!(eroded.support_core.unwrap().pixel_count < mask.physical.pixel_count);
    }

    #[test]
    fn erosion_matches_pixel_oracle() {
        let fp = shape();
        for angle in [0., 13., 45., 137.] {
            let loops: Vec<Vec<_>> = fp
                .loops
                .iter()
                .map(|l| l.iter().map(|&p| rotate(p, angle)).collect())
                .collect();
            for resolution in [0.125, 0.25] {
                let spans = SpanRaster::scanline(&loops, resolution, (-64, 63, -64, 63));
                let pixels: HashSet<_> = PixelMask::from_spans(&spans).pixels().collect();
                for radius in [0i32, 1, 2, 3, 5] {
                    let eroded: HashSet<_> = pixels
                        .iter()
                        .copied()
                        .filter(|&(x, y)| {
                            (-radius..=radius).all(|dy| {
                                (-radius..=radius).all(|dx| {
                                    dx * dx + dy * dy > radius * radius
                                        || pixels.contains(&(x + dx, y + dy))
                                })
                            })
                        })
                        .collect();
                    assert_eq!(
                        PixelMask::from_spans(&spans.erode(radius))
                            .pixels()
                            .collect::<HashSet<_>>(),
                        eroded,
                        "angle={angle} resolution={resolution} radius={radius}"
                    );
                }
            }
        }
    }

    #[test]
    fn word_overlap_matches_pixel_oracle_across_word_boundaries() {
        let a = PixelMask::from_spans(&SpanRaster {
            min_y: -2,
            rows: vec![vec![(-70, -63), (-1, 2), (63, 70)], vec![(-35, 41)]],
        });
        let b = PixelMask::from_spans(&SpanRaster {
            min_y: 1,
            rows: vec![vec![(-65, -62), (0, 65)], vec![(-3, 7)]],
        });
        for y in -5..5 {
            for x in -140..140 {
                let expected = a.pixels().any(|(ax, ay)| b.contains_local(ax - x, ay - y));
                assert_eq!(masks_overlap(&a, 0, 0, &b, x, y), expected, "{x},{y}");
            }
        }
    }
}

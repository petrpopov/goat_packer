//! Deterministic packing in a geometry-derived frame. No-fit rasters describe every
//! forbidden relative integer translation, so a search scans free *bits*, not collisions.
use crate::collision::{origin_range, zone_pairs};
use crate::footprint::Footprint;
use crate::frame::Frame;
use crate::grid::grid_layout_inner;
use crate::layout::{PixelPlacement, better_layout, cap_reached, layout_bbox_area};
use crate::options::{PackOptions, PackResult, PackedPlacement};
use crate::pixel_mask::set_bit_range;
use crate::raster_mask::{RasterMask, rasterize_footprint_with_support};
use crate::span_raster::{SpanRaster, merge_spans};
use rayon::prelude::*;

/// All integer translations of B relative to A that overlap at least one occupied pixel.
/// Span difference: [a,b) - [c,d) = [a-d+1,b-c). Holes remain holes.
/// The pairs are `zone_pairs` plus the support cores, which exist only with
/// `support_overlap_mm`; a side without a core has no support zone, and the zone pairs
/// already cover it.
fn no_fit(a: &RasterMask, b: &RasterMask) -> SpanRaster {
    let mut pairs = zone_pairs(a, b);
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
        let (min_x, min_y, max_x, max_y) = origin_range(options, mask);
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
        // Full-extent exclusions against the patch ∪ silhouette bbox, body exclusions (the
        // tower) against the body bbox only — see `PackOptions::body_exclusions`.
        let zones = [
            (&options.exclusions, b),
            (&options.body_exclusions, mask.body_bounds),
        ];
        for (exclusions, b) in zones {
            for exclusion in exclusions {
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

/// Mixed palette: the strongest single-orientation families are intermixed rather than
/// restricting the whole plate to a half-turn pair, and orthogonal orientations are added as
/// well — rectangles can need a windmill layout even when all single-orientation runs tie.
const MIXED_FAMILIES: usize = 4;
/// Quarter turns tried around each shortlisted family's angle (degrees).
const MIXED_OFFSETS_DEG: [f64; 4] = [0.0, 90.0, 180.0, 270.0];
/// Sweep variants per palette angle in the mixed pass.
const MIXED_VARIANTS_PER_ANGLE: usize = 8;
/// Most sweep variants any single family runs, however high the requested effort.
const MAX_SWEEP_VARIANTS: usize = 64;

/// A candidate origin for the next object, ordered by the sweep's preference.
struct Candidate {
    /// Height of the placement's far edge in sweep direction, quantized so ties compare exactly.
    top: i64,
    side: i32,
    mask: usize,
    placement: PixelPlacement,
}

/// Which corner a sweep starts from (bit 0 = from the right, bit 1 = from the top): a fixed
/// corner, or cycling around the four corners once `variant` is far enough along. The latter
/// avoids greedy height traps such as four 6x4 rectangles around a 2x2 central hole.
fn sweep_corner(variant: usize, placed: usize, mask_count: usize) -> usize {
    if (variant / (4 * mask_count)).is_multiple_of(2) {
        variant % 4
    } else {
        (variant % 4) ^ [0, 1, 3, 2][placed % 4]
    }
}

/// The best next placement over every mask's free centers, or `None` when nothing fits.
fn next_placement(
    free: &[FreeCenters],
    masks: &[RasterMask],
    options: &PackOptions,
    variant: usize,
    placed: usize,
) -> Option<PixelPlacement> {
    let corner = sweep_corner(variant, placed, masks.len());
    let reverse_x = corner & 1 != 0;
    let reverse_y = corner & 2 != 0;
    let phase = if placed == 0 {
        variant / (8 * masks.len())
    } else {
        0
    };
    let candidates = free.iter().enumerate().filter_map(|(i, f)| {
        let (x, y) = f.first(reverse_x, reverse_y, phase)?;
        let b = masks[i].bed_physical_bounds;
        let top = if reverse_y {
            -f64::from(y) - b.min_y / options.resolution
        } else {
            f64::from(y) + b.max_y / options.resolution
        };
        Some(Candidate {
            top: (top * 1e6).round() as i64,
            side: if reverse_x { -x } else { x },
            mask: i,
            placement: PixelPlacement { mask: i, x, y },
        })
    });
    let best = candidates.min_by(|a, b| {
        if placed == 0 {
            let first = (variant / 4) % masks.len();
            let rank = |i| (i + masks.len() - first) % masks.len();
            rank(a.mask)
                .cmp(&rank(b.mask))
                .then(a.top.cmp(&b.top))
                .then(a.side.cmp(&b.side))
        } else {
            (a.top, a.side, a.mask).cmp(&(b.top, b.side, b.mask))
        }
    });
    best.map(|candidate| candidate.placement)
}

/// Greedy sweeps over `masks`, one per variant; returns the best layout and the first
/// sweep's (count, bbox area) as the family's baseline.
fn pair_search(
    masks: &[RasterMask],
    options: &PackOptions,
    variants: usize,
) -> (Vec<PixelPlacement>, (usize, f64)) {
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
        while let Some(p) = next_placement(&free, masks, options, variant, placed.len()) {
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

fn pack_layout(
    layout: &[PixelPlacement],
    masks: &[RasterMask],
    options: &PackOptions,
) -> Vec<PackedPlacement> {
    layout
        .iter()
        .map(|p| p.to_packed(masks[p.mask].angle_deg, options))
        .collect()
}

/// The best layout of one angle family: a mask at `angle` and (unless the orientation is
/// locked) its half-turn.
struct Family {
    placements: Vec<PackedPlacement>,
    area: f64,
    /// (objects, bbox area) of the first sweep alone.
    baseline: (usize, f64),
}

fn search_family(frame: &Frame, angle: f64, options: &PackOptions, variants: usize) -> Family {
    let rasterize = |angle: f64| {
        rasterize_footprint_with_support(
            &frame.footprint,
            angle,
            options.resolution,
            options.clearance,
            options.support_overlap_mm,
        )
    };
    let mut masks = vec![rasterize(angle)];
    if !options.lock_orientation {
        masks.push(rasterize(angle + 180.0));
    }
    let (best, baseline) = pair_search(&masks, options, variants);
    Family {
        area: layout_bbox_area(&best, &masks),
        placements: pack_layout(&best, &masks, options),
        baseline,
    }
}

/// Angle families ordered by their first sweep alone: increasing effort never removes a
/// previously searched mixed palette, and together with prefix-ordered sweeps this makes
/// effort monotone. Returns family indices, best first.
fn shortlist(families: &[Family]) -> Vec<usize> {
    let mut ranked: Vec<usize> = (0..families.len()).collect();
    ranked.sort_by(|&ia, &ib| {
        let (a, b) = (&families[ia], &families[ib]);
        b.baseline
            .0
            .cmp(&a.baseline.0)
            .then(a.baseline.1.total_cmp(&b.baseline.1))
            .then(ia.cmp(&ib))
    });
    ranked
}

/// The winning family: most objects, then smallest bbox, then the earliest angle.
fn best_family(families: &[Family]) -> &Family {
    let mut ranked: Vec<usize> = (0..families.len()).collect();
    ranked.sort_by(|&ia, &ib| {
        let (a, b) = (&families[ia], &families[ib]);
        b.placements
            .len()
            .cmp(&a.placements.len())
            .then(a.area.total_cmp(&b.area))
            .then(ia.cmp(&ib))
    });
    &families[ranked[0]]
}

/// Angles (degrees) of the mixed palette: the shortlisted families' angles and their
/// quarter turns, without duplicates. Only a bounded local palette needs quadratic no-fit
/// maps; the full angle grid never incurs an O(angles^2) allocation.
fn mixed_palette(shortlist: &[usize], options: &PackOptions) -> Vec<f64> {
    let mut angles: Vec<f64> = Vec::new();
    for &index in shortlist.iter().take(MIXED_FAMILIES) {
        let base = index as f64 * options.angle_step_deg;
        for offset in MIXED_OFFSETS_DEG {
            let angle = (base + offset).rem_euclid(360.0);
            if !angles.iter().any(|&a| (a - angle).abs() < 1e-7) {
                angles.push(angle);
            }
        }
    }
    angles
}

pub(crate) fn search(fp: &Footprint, options: &PackOptions) -> PackResult {
    let frame = Frame::new(fp, options.lock_orientation);
    let count = if options.lock_orientation {
        1
    } else {
        (180.0 / options.angle_step_deg).ceil().max(1.0) as usize
    };
    let variants = options
        .attempts
        .max(1)
        .div_ceil(count)
        .clamp(1, MAX_SWEEP_VARIANTS);
    let families: Vec<Family> = (0..count)
        .into_par_iter()
        .map(|i| search_family(&frame, i as f64 * options.angle_step_deg, options, variants))
        .collect();
    let mut placements = best_family(&families).placements.clone();
    let mut attempts_run = count * variants;
    if !options.lock_orientation && options.attempts > count {
        let angles = mixed_palette(&shortlist(&families), options);
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
        let mixed_variants = (options.attempts - count).min(MIXED_VARIANTS_PER_ANGLE * masks.len());
        let (mixed, _) = pair_search(&masks, options, mixed_variants);
        attempts_run += mixed_variants;
        if mixed.len() > placements.len() {
            placements = pack_layout(&mixed, &masks, options);
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
    use crate::collision::placements_collide;
    use crate::footprint::LayerZone;
    use crate::geometry::{Point, point_in_loops, rotate_point};
    use crate::pixel_mask::{PixelMask, masks_overlap};
    use crate::raster_mask::rasterize_footprint;
    use std::collections::HashSet;

    fn shape() -> Footprint {
        let ring = |points: &[(f64, f64)]| {
            let mut ring: Vec<_> = points.iter().map(|&(x, y)| Point { x, y }).collect();
            ring.push(ring[0]);
            ring
        };
        Footprint {
            support_landing_loops: Vec::new(),
            layer_zones: Vec::new(),
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
    fn no_fit_matches_brute_force_layer_zone_collisions_at_every_shift() {
        // A band whose support reaches 3 mm right of the patch (an interface under an ear)
        // and whose body is the patch's left half: the no-fit raster must forbid exactly the
        // shifts where that support meets the other's band body, on top of the zone pairs.
        let mut fp = shape();
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| {
            vec![
                Point { x: x0, y: y0 },
                Point { x: x1, y: y0 },
                Point { x: x1, y: y1 },
                Point { x: x0, y: y1 },
                Point { x: x0, y: y0 },
            ]
        };
        fp.layer_zones = vec![LayerZone {
            support_loops: vec![rect(3.5, 0.0, 6.0, 1.0)],
            body_loops: vec![rect(-3.0, -2.0, 0.0, 2.0)],
        }];
        let a = rasterize_footprint(&fp, 0., 0.5, 0.5);
        let b = rasterize_footprint(&fp, 0., 0.5, 0.5);
        assert_eq!(a.layer_zones.len(), 1);
        let nfp = no_fit(&a, &b);
        let (support, body) = (&a.layer_zones[0].0, &b.layer_zones[0].1);
        let mut differs_from_zones = false;
        for y in -30..=30 {
            for x in -30..=30 {
                let zones = masks_overlap(&a.clearance, 0, 0, &b.body_clearance, x, y)
                    || masks_overlap(&a.body_clearance, 0, 0, &b.clearance, x, y);
                let layer = masks_overlap(support, 0, 0, body, x, y)
                    || masks_overlap(&a.layer_zones[0].1, 0, 0, &b.layer_zones[0].0, x, y);
                let actual = nfp.row(y).iter().any(|&(lo, hi)| lo <= x && x < hi);
                assert_eq!(actual, zones || layer, "shift={x},{y}");
                assert_eq!(actual, placements_collide(&a, &b, x, y));
                differs_from_zones |= actual != zones;
            }
        }
        assert!(differs_from_zones, "the layer zone changed nothing");
        // B 8 mm to the right (16 px): its band body (x 5..8) meets A's support (x 3.5..6).
        assert!(placements_collide(&a, &b, 16, 0));
    }

    #[test]
    fn no_fit_matches_brute_force_landing_collisions_at_every_shift() {
        // The body loop as the silhouette, a narrower strip of it as the landing zone: the
        // no-fit raster must forbid exactly the shifts where a patch meets a landing zone or
        // two silhouettes meet.
        let mut fp = shape();
        let landing = vec![
            Point { x: -2., y: -2. },
            Point { x: 2., y: -2. },
            Point { x: 2., y: -1.5 },
            Point { x: -2., y: -1.5 },
            Point { x: -2., y: -2. },
        ];
        fp.support_landing_loops = vec![landing];
        for angle in [0., 13., 90., 180.] {
            let a = rasterize_footprint(&fp, 0., 0.5, 0.5);
            let b = rasterize_footprint(&fp, angle, 0.5, 0.5);
            assert!(!a.landing_is_body);
            let nfp = no_fit(&a, &b);
            let mut differs_from_two_zone = false;
            for y in -24..=24 {
                for x in -24..=24 {
                    let expected = masks_overlap(&a.clearance, 0, 0, &b.landing_clearance, x, y)
                        || masks_overlap(&a.landing_clearance, 0, 0, &b.clearance, x, y)
                        || masks_overlap(&a.body_clearance, 0, 0, &b.body_clearance, x, y);
                    let actual = nfp.row(y).iter().any(|&(lo, hi)| lo <= x && x < hi);
                    assert_eq!(actual, expected, "angle={angle}, shift={x},{y}");
                    assert_eq!(actual, placements_collide(&a, &b, x, y));
                    differs_from_two_zone |= actual
                        != (masks_overlap(&a.clearance, 0, 0, &b.body_clearance, x, y)
                            || masks_overlap(&a.body_clearance, 0, 0, &b.clearance, x, y));
                }
            }
            assert!(
                differs_from_two_zone,
                "angle={angle}: landing changed nothing"
            );
        }
    }

    #[test]
    fn scanline_and_dilation_match_original_point_sampler() {
        let fp = shape();
        for angle in [0., 2., 13., 45., 90., 137., 180.] {
            let loops: Vec<Vec<_>> = fp
                .loops
                .iter()
                .map(|l| l.iter().map(|&p| rotate_point(p, angle)).collect())
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
        assert!(
            rasterize_footprint(&fp, 0., 0.5, 0.5)
                .support_core
                .is_none()
        );
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
                .map(|l| l.iter().map(|&p| rotate_point(p, angle)).collect())
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

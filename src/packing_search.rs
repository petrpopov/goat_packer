//! Deterministic packing in a geometry-derived frame. No-fit rasters describe every
//! forbidden relative integer translation, so a search scans free *bits*, not collisions.
use crate::collision::no_fit;
use crate::footprint::Footprint;
use crate::frame::Frame;
use crate::grid::grid_layout_inner;
use crate::kind_masks::FeasibleOrigins;
use crate::layout::{PixelPlacement, better_layout, cap_reached, layout_bbox_area};
use crate::options::{PackOptions, PackResult, PackedPlacement};
use crate::pixel_mask::set_bit_range;
use crate::raster_mask::{LegacyPrint, RasterMask};
use crate::span_raster::SpanRaster;
use rayon::prelude::*;

#[derive(Clone)]
struct FreeCenters {
    min_x: i32,
    min_y: i32,
    width: usize,
    rows: Vec<Vec<u64>>,
}

impl FreeCenters {
    /// The base map: every origin the plate zones allow (`kind_masks::feasible_origins`).
    fn new(origins: &FeasibleOrigins) -> Self {
        let Some((min_x, min_y, max_x, max_y)) = origins.bounds() else {
            return Self {
                min_x: 0,
                min_y: 0,
                width: 0,
                rows: Vec::new(),
            };
        };
        let width = (max_x - min_x + 1) as usize;
        let mut rows = vec![vec![0; width.div_ceil(64)]; (max_y - min_y + 1) as usize];
        for (index, row) in rows.iter_mut().enumerate() {
            for &(start, end) in origins.allowed.row(min_y + index as i32) {
                set_bit_range(row, (start - min_x) as usize, (end - min_x) as usize);
            }
        }
        Self {
            min_x,
            min_y,
            width,
            rows,
        }
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
                let skip = [0, 2, 1, 3][phase % START_PHASES] * self.width / START_PHASES;
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
/// A sweep starts from one of the bed's four corners. Variants run through the corners fastest,
/// then through which mask places the first object, then (every `START_PHASE_STRIDE` variants
/// per mask) through the start phase.
const CORNERS: usize = 4;
/// The first object of a plate may start a quarter, half or three quarters along the first row
/// (`START_PHASES` positions, in the order `FreeCenters::first` tries them).
const START_PHASES: usize = 4;
/// Variants per mask between two start phases: all corners, twice (straight and cycling).
const START_PHASE_STRIDE: usize = 2 * CORNERS;
/// Candidate heights are compared as integers at this many units per pixel, so float noise
/// cannot reorder two candidates that are equally high.
const HEIGHT_QUANTUM: f64 = 1e6;
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
    if (variant / (CORNERS * mask_count)).is_multiple_of(2) {
        variant % CORNERS
    } else {
        (variant % CORNERS) ^ [0, 1, 3, 2][placed % CORNERS]
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
        variant / (START_PHASE_STRIDE * masks.len())
    } else {
        0
    };
    let candidates = free.iter().enumerate().filter_map(|(i, f)| {
        let (x, y) = f.first(reverse_x, reverse_y, phase)?;
        let b = masks[i].extent_bounds;
        let top = if reverse_y {
            -f64::from(y) - b.min_y / options.resolution
        } else {
            f64::from(y) + b.max_y / options.resolution
        };
        Some(Candidate {
            top: (top * HEIGHT_QUANTUM).round() as i64,
            side: if reverse_x { -x } else { x },
            mask: i,
            placement: PixelPlacement { mask: i, x, y },
        })
    });
    let best = candidates.min_by(|a, b| {
        if placed == 0 {
            let first = (variant / CORNERS) % masks.len();
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
    print: &LegacyPrint,
    options: &PackOptions,
    variants: usize,
) -> (Vec<PixelPlacement>, (usize, f64)) {
    let nfps: Vec<Vec<_>> = masks
        .par_iter()
        .map(|a| masks.iter().map(|b| no_fit(a, b)).collect())
        .collect();
    let initial: Vec<_> = masks
        .iter()
        .map(|m| FreeCenters::new(&print.origins(m)))
        .collect();
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
        .map(|p| p.to_packed(masks[p.mask].rotation_deg, options))
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

fn search_family(
    print: &LegacyPrint,
    angle: f64,
    options: &PackOptions,
    variants: usize,
) -> Family {
    let mut masks = vec![print.masks_at(angle)];
    if !options.lock_orientation {
        masks.push(print.masks_at(angle + 180.0));
    }
    let (best, baseline) = pair_search(&masks, print, options, variants);
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
    let print = LegacyPrint::new(&frame.footprint, options).expect("invalid packing input");
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
        .map(|i| search_family(&print, i as f64 * options.angle_step_deg, options, variants))
        .collect();
    let mut placements = best_family(&families).placements.clone();
    let mut attempts_run = count * variants;
    if !options.lock_orientation && options.attempts > count {
        let angles = mixed_palette(&shortlist(&families), options);
        let masks: Vec<_> = angles.par_iter().map(|&a| print.masks_at(a)).collect();
        let mixed_variants = (options.attempts - count).min(MIXED_VARIANTS_PER_ANGLE * masks.len());
        let (mixed, _) = pair_search(&masks, &print, options, mixed_variants);
        attempts_run += mixed_variants;
        if mixed.len() > placements.len() {
            placements = pack_layout(&mixed, &masks, options);
        }
    }
    let grid = grid_layout_inner(&frame.footprint, &print, options);
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
    use crate::geometry::{Point, point_in_loops, rotate_point};
    use crate::pixel_mask::{PixelMask, masks_overlap};
    use crate::span_raster::SpanRaster;
    use std::collections::HashSet;

    fn shape() -> Vec<Vec<Point>> {
        let ring = |points: &[(f64, f64)]| {
            let mut ring: Vec<_> = points.iter().map(|&(x, y)| Point { x, y }).collect();
            ring.push(ring[0]);
            ring
        };
        vec![
            ring(&[
                (-3., -2.),
                (3., -2.),
                (3., -1.),
                (0., -1.),
                (0., 2.),
                (-3., 2.),
            ]),
            ring(&[(-2.5, -0.5), (-1., -0.5), (-1., 1.), (-2.5, 1.)]),
        ]
    }

    #[test]
    fn scanline_matches_original_point_sampler() {
        for angle in [0., 2., 13., 45., 90., 137., 180.] {
            let loops: Vec<Vec<_>> = shape()
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

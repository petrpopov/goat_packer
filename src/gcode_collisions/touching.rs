//! Which support lines touch which foreign body lines on one layer.

use std::collections::{BTreeMap, HashMap};

/// Edge gaps below this count as touching: absorbs float noise in the slicers' own
/// rounding (a wall and a support at exactly the touching distance).
pub(super) const TOUCH_EPSILON_MM: f64 = 0.01;
/// How far a support base may press into a neighbour's first layer before it counts. The
/// owner's A2L 70-goat plate (BambuStudio GUI G-code, printed fine) has base lines 0.088 mm
/// into the neighbours' first-layer walls on 58 pairs, and the CLI slices the same; the
/// packer's rule lets a base touch a foreign landing (`SUPPORT_BASE_GAP_MM` = 0). Above the
/// first layer a support on a foreign body fuses to it, so there `TOUCH_EPSILON_MM` stays.
pub(super) const FIRST_LAYER_BASE_OVERLAP_MM: f64 = 0.1;
/// How close a support above the first layer must come to a foreign body to enter the
/// reported minimum gap (`PlateAnalysis::min_support_gap_mm`): a statistic for judging the
/// packer's `ForeignGaps::band_mm`, never a failure threshold.
pub(super) const GAP_STATISTIC_REACH_MM: f64 = 3.0;
/// Spatial hash cell for the body lines of one layer.
const CELL_MM: f64 = 2.0;

pub(super) struct Line {
    pub(super) object: usize,
    pub(super) from: (f64, f64),
    pub(super) to: (f64, f64),
    pub(super) width: f64,
    pub(super) support: bool,
}

fn cell(point: (f64, f64)) -> (i32, i32) {
    (
        (point.0 / CELL_MM).floor() as i32,
        (point.1 / CELL_MM).floor() as i32,
    )
}

/// Segment-to-segment distance in the plane (0 when they cross).
fn segment_distance(a0: (f64, f64), a1: (f64, f64), b0: (f64, f64), b1: (f64, f64)) -> f64 {
    let cross = |o: (f64, f64), p: (f64, f64), q: (f64, f64)| {
        (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0)
    };
    let d1 = cross(b0, b1, a0);
    let d2 = cross(b0, b1, a1);
    let d3 = cross(a0, a1, b0);
    let d4 = cross(a0, a1, b1);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return 0.0;
    }
    point_segment_distance(a0, b0, b1)
        .min(point_segment_distance(a1, b0, b1))
        .min(point_segment_distance(b0, a0, a1))
        .min(point_segment_distance(b1, a0, a1))
}

fn point_segment_distance(p: (f64, f64), s0: (f64, f64), s1: (f64, f64)) -> f64 {
    let (dx, dy) = (s1.0 - s0.0, s1.1 - s0.1);
    let length2 = dx * dx + dy * dy;
    let t = if length2 < 1e-18 {
        0.0
    } else {
        (((p.0 - s0.0) * dx + (p.1 - s0.1) * dy) / length2).clamp(0.0, 1.0)
    };
    (p.0 - (s0.0 + t * dx)).hypot(p.1 - (s0.1 + t * dy))
}

/// (support object, body object, smallest edge gap) for every pair on one layer whose gap is
/// below `reach_mm` (at least `TOUCH_EPSILON_MM`: the touching pairs always come back).
pub(super) fn layer_collisions(lines: &[Line], reach_mm: f64) -> Vec<(usize, usize, f64)> {
    let reach_mm = reach_mm.max(TOUCH_EPSILON_MM);
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    let mut widest = 0.0f64;
    for (index, line) in lines.iter().enumerate().filter(|(_, line)| !line.support) {
        widest = widest.max(line.width);
        let (c0, c1) = (cell(line.from), cell(line.to));
        for cx in c0.0.min(c1.0)..=c0.0.max(c1.0) {
            for cy in c0.1.min(c1.1)..=c0.1.max(c1.1) {
                grid.entry((cx, cy)).or_default().push(index);
            }
        }
    }
    if grid.is_empty() {
        return Vec::new();
    }
    let mut found: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for support in lines.iter().filter(|line| line.support) {
        // Every body line within reach sits in a cell overlapping the support line's bbox
        // grown by the widest possible touching distance.
        let reach = (support.width + widest) * 0.5 + reach_mm;
        let lo = cell((
            support.from.0.min(support.to.0) - reach,
            support.from.1.min(support.to.1) - reach,
        ));
        let hi = cell((
            support.from.0.max(support.to.0) + reach,
            support.from.1.max(support.to.1) + reach,
        ));
        for cx in lo.0..=hi.0 {
            for cy in lo.1..=hi.1 {
                let Some(candidates) = grid.get(&(cx, cy)) else {
                    continue;
                };
                for &index in candidates {
                    let body = &lines[index];
                    if body.object == support.object {
                        continue;
                    }
                    let gap = segment_distance(support.from, support.to, body.from, body.to)
                        - (support.width + body.width) * 0.5;
                    if gap < reach_mm {
                        let entry = found
                            .entry((support.object, body.object))
                            .or_insert(f64::INFINITY);
                        *entry = entry.min(gap);
                    }
                }
            }
        }
    }
    found
        .into_iter()
        .map(|((support, body), gap)| (support, body, gap))
        .collect()
}

//! Print time versus batch size: measured points, interpolation and where to measure next.

use std::collections::HashSet;

use super::SchedulerResult;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BatchTiming {
    pub batch_size: usize,
    pub slicer_seconds: u64,
    pub preparation_seconds: u64,
    pub filament_grams: f64,
}

impl BatchTiming {
    pub fn new(
        batch_size: usize,
        slicer_seconds: u64,
        preparation_seconds: u64,
        filament_grams: f64,
    ) -> Self {
        Self {
            batch_size,
            slicer_seconds,
            preparation_seconds,
            filament_grams,
        }
    }

    pub fn job_seconds(&self) -> u64 {
        self.slicer_seconds + self.preparation_seconds
    }
}

/// Piecewise-linear fit of one per-batch-size quantity (slicer seconds, filament grams, model
/// seconds) over the batch sizes that were actually sliced. Print time versus batch size is very
/// close to affine (fixed startup + tower flush, plus a near-constant per-object cost), so a
/// handful of measured sizes with linear interpolation in between stays within a few percent of
/// the real value while cutting the benchmark sweep from `N` slices to ~5 — the exact trade the
/// adaptive benchmark mode in `daily_scheduler` makes. Interpolation is bounded by the measured
/// neighbors on both sides, so an interpolated batch can never overshoot the measured envelope.
#[derive(Clone, Debug)]
pub struct BatchCurve {
    points: Vec<(usize, f64)>,
}

impl BatchCurve {
    /// `measured` may arrive in any order and contain duplicate counts (the last one wins);
    /// it must contain at least one point.
    pub fn new(measured: &[(usize, f64)]) -> SchedulerResult<Self> {
        if measured.is_empty() {
            return Err("batch curve needs at least one measured point".into());
        }
        let mut points: Vec<(usize, f64)> = measured.to_vec();
        points.sort_by_key(|(count, _)| *count);
        points.dedup_by_key(|(count, _)| *count);
        Ok(Self { points })
    }

    pub fn value(&self, count: usize) -> f64 {
        let &(min_count, min_value) = &self.points[0];
        let &(max_count, max_value) = &self.points[self.points.len() - 1];
        if count <= min_count {
            return min_value;
        }
        if count >= max_count {
            return max_value;
        }
        for window in self.points.windows(2) {
            let (low_count, low_value) = window[0];
            let (high_count, high_value) = window[1];
            if (low_count..=high_count).contains(&count) {
                let span = (high_count - low_count) as f64;
                let share = (count - low_count) as f64 / span;
                return low_value + (high_value - low_value) * share;
            }
        }
        unreachable!("count is between the first and last measured points")
    }
}

/// Ordinary least-squares line `y = slope * count + intercept` over the measured points. Used as
/// the "what would a smooth affine model predict" second opinion when deciding whether the
/// piecewise-linear fit needs another measured point.
pub fn least_squares_line(points: &[(usize, f64)]) -> (f64, f64) {
    let n = points.len() as f64;
    let sum_x: f64 = points.iter().map(|(count, _)| *count as f64).sum();
    let sum_y: f64 = points.iter().map(|(_, value)| value).sum();
    let sum_xy: f64 = points
        .iter()
        .map(|(count, value)| *count as f64 * value)
        .sum();
    let sum_xx: f64 = points
        .iter()
        .map(|(count, _)| (*count as f64) * (*count as f64))
        .sum();
    let denominator = n * sum_xx - sum_x * sum_x;
    if denominator.abs() < f64::EPSILON {
        return (0.0, sum_y / n.max(1.0));
    }
    let slope = (n * sum_xy - sum_x * sum_y) / denominator;
    let intercept = (sum_y - slope * sum_x) / n;
    (slope, intercept)
}

/// Where the adaptive benchmark should slice next: the unmeasured interior batch size where the
/// piecewise-linear fit and the least-squares affine fit disagree most, relative to the affine
/// prediction at the full plate (normalizing by the *largest* time, not the per-point value, so
/// the search targets the biggest absolute error — the one that could actually flip a schedule
/// decision — instead of small relative wiggles at short batches). Returns `None` when every size
/// is already measured; the caller stops refining once the returned deviation drops under its
/// tolerance or its slice budget runs out — with genuinely affine data both fits agree and the
/// loop stops after the initial samples.
pub fn next_refinement_count(
    measured_counts: &[usize],
    max_count: usize,
    curve: &BatchCurve,
    line: (f64, f64),
) -> Option<(usize, f64)> {
    let known: HashSet<usize> = measured_counts.iter().copied().collect();
    let (slope, intercept) = line;
    let scale = slope * max_count as f64 + intercept;
    if scale <= 0.0 {
        return None;
    }
    let mut worst: Option<(usize, f64)> = None;
    for count in 1..=max_count {
        if known.contains(&count) {
            continue;
        }
        let affine = slope * count as f64 + intercept;
        if affine <= 0.0 {
            continue;
        }
        let deviation = (curve.value(count) - affine).abs() / scale;
        if worst.is_none_or(|(_, current)| deviation > current) {
            worst = Some((count, deviation));
        }
    }
    worst
}

//! The time window: how many objects of the packed plate fit into the requested hours.

use goat_packer::{
    print_time::{
        TimeCorrection, TimeSample, WindowFit, corrected_seconds, count_for_window, shrink_count,
    },
    scheduler::SliceEstimate,
};

use crate::AppResult;

const SECONDS_PER_HOUR: f64 = 3600.0;

/// Window slices allowed after the max plate before giving up; each costs a full slice, and
/// the affine aim plus the shrink step normally land within one or two.
pub const MAX_WINDOW_SLICES: usize = 4;

/// Outcome of the window search when the window binds: the chosen prefix length and the
/// window slices measured on the way (the last one is the chosen plate).
#[derive(Debug, PartialEq)]
pub struct WindowChoice {
    pub count: usize,
    pub samples: Vec<TimeSample>,
}

pub fn window_seconds(window_hours: f64) -> f64 {
    window_hours * SECONDS_PER_HOUR
}

pub fn hours(seconds: f64) -> f64 {
    seconds / SECONDS_PER_HOUR
}

pub fn even_one_exceeds(corrected_s: f64, window_s: f64) -> String {
    format!(
        "even one object exceeds the time window: {:.1} h > window {:.1} h",
        hours(corrected_s),
        hours(window_s)
    )
}

/// Spec steps 3-5: aim at the window with the affine line through the probe (1 object) and the
/// max plate, then measure that prefix and shrink while it is over. Under the window is
/// accepted without searching upward: the line already aimed at the boundary, and a
/// guaranteed fit beats one more object. `None` = the max plate fits (window does not bind).
/// `measure(n)` builds and slices the n-prefix and returns its corrected sample.
pub fn fit_window(
    probe: TimeSample,
    max: TimeSample,
    window_s: f64,
    mut measure: impl FnMut(usize) -> AppResult<TimeSample>,
) -> AppResult<Option<WindowChoice>> {
    if probe.corrected_s > window_s {
        return Err(even_one_exceeds(probe.corrected_s, window_s).into());
    }
    let mut count = match count_for_window(probe, max, window_s) {
        WindowFit::All => return Ok(None),
        // The probe fits (checked above), so the line through it cannot be over the window at
        // one object; NotEvenOne here means the max plate itself holds a single object and is
        // over. That measured plate is the evidence, not the probe.
        WindowFit::NotEvenOne => return Err(even_one_exceeds(max.corrected_s, window_s).into()),
        WindowFit::Count(count) => count,
    };
    let mut samples = Vec::new();
    loop {
        println!(
            "time window {:.1} h: max plate {} objects = {:.1} h corrected; trying {count}",
            hours(window_s),
            max.count,
            hours(max.corrected_s)
        );
        let sample = measure(count)?;
        samples.push(sample);
        if sample.corrected_s <= window_s {
            println!(
                "time window {:.1} h: {count} objects = {:.1} h corrected; fits",
                hours(window_s),
                hours(sample.corrected_s)
            );
            return Ok(Some(WindowChoice { count, samples }));
        }
        if count == 1 {
            return Err(even_one_exceeds(sample.corrected_s, window_s).into());
        }
        if samples.len() >= MAX_WINDOW_SLICES {
            return Err(format!(
                "time window {:.1} h not met after {MAX_WINDOW_SLICES} window slices; last: \
                 {count} objects = {:.1} h corrected",
                hours(window_s),
                hours(sample.corrected_s)
            )
            .into());
        }
        count = shrink_count(count, sample.corrected_s, window_s, shrink_slope(probe, max, sample));
    }
}

/// Seconds per object for the shrink step after a window slice overshot. The secant through
/// that window plate and the max plate, because those two were sliced with identical settings
/// (both from the assembled output: relocated tower, nozzle-clean wipe), while the probe is
/// sliced from the source (tower at the source corner, so every colour change travels
/// differently). Any per-plate offset the probe misses cancels in the secant. The probe line is
/// only the fallback when no such pair exists (the sample has the max plate's count).
pub fn shrink_slope(probe: TimeSample, max: TimeSample, sample: TimeSample) -> f64 {
    let secant = |a: TimeSample, b: TimeSample| {
        (a.corrected_s - b.corrected_s) / (a.count as f64 - b.count as f64)
    };
    if sample.count != max.count {
        secant(sample, max)
    } else if sample.count != probe.count {
        secant(sample, probe)
    } else {
        // No slope information at all; `shrink_count` steps by one on a non-positive slope.
        0.0
    }
}

pub fn time_sample(
    count: usize,
    estimate: &SliceEstimate,
    slicer_change_s: f64,
    correction: &TimeCorrection,
) -> TimeSample {
    let estimate_s = estimate.slicer_total_seconds as f64;
    TimeSample {
        count,
        estimate_s,
        changes: estimate.filament_changes,
        corrected_s: corrected_seconds(
            estimate_s,
            estimate.filament_changes,
            slicer_change_s,
            correction,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(count: usize, corrected_s: f64) -> TimeSample {
        TimeSample { count, estimate_s: corrected_s, changes: 0, corrected_s }
    }

    #[test]
    fn fit_window_keeps_everything_when_the_max_plate_fits() {
        let fit = fit_window(sample(1, 1_000.0), sample(30, 30_000.0), 40_000.0, |_| {
            panic!("no window slice expected")
        })
        .unwrap();
        assert_eq!(fit, None);
    }

    #[test]
    fn fit_window_rejects_a_single_object_over_the_window() {
        let error = fit_window(sample(1, 50_000.0), sample(30, 90_000.0), 43_200.0, |_| {
            panic!("no window slice expected")
        })
        .unwrap_err();
        assert!(error.to_string().contains("even one object exceeds the time window"));
    }

    #[test]
    fn fit_window_reports_the_max_plate_time_when_a_single_object_max_plate_is_over() {
        // The probe fits, but the bed holds only one object and that real plate (relocated
        // tower, wipe) is over the window: the error must name the measured 13.9 h, not the
        // probe's 0.3 h.
        let error = fit_window(sample(1, 1_000.0), sample(1, 50_000.0), 43_200.0, |_| {
            panic!("no window slice expected")
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("even one object exceeds the time window"), "{error}");
        assert!(error.contains("13.9 h > window 12.0 h"), "{error}");
    }

    #[test]
    fn fit_window_accepts_the_affine_count_when_it_fits() {
        // line(N) = 1000 * N; window 15500 -> 15.
        let mut sliced = Vec::new();
        let fit = fit_window(sample(1, 1_000.0), sample(41, 41_000.0), 15_500.0, |n| {
            sliced.push(n);
            Ok(sample(n, 1_000.0 * n as f64))
        })
        .unwrap()
        .unwrap();
        assert_eq!(sliced, vec![15]);
        assert_eq!(fit.count, 15);
        assert_eq!(fit.samples.len(), 1);
    }

    #[test]
    fn fit_window_shrinks_along_the_secant_through_the_window_plate_and_the_max_plate() {
        // The probe is not comparable to window plates (no relocated tower, possibly no
        // nozzle-clean wipe): real plates lie on the line through (15, 17800) and (41, 41000),
        // i.e. 892.3 s per object, well off the probe line.
        let real = |n: usize| 41_000.0 - (41_000.0 - 17_800.0) / 26.0 * (41 - n) as f64;
        let mut sliced = Vec::new();
        let fit = fit_window(sample(1, 1_000.0), sample(41, 41_000.0), 15_500.0, |n| {
            sliced.push(n);
            Ok(sample(n, real(n)))
        })
        .unwrap()
        .unwrap();
        // 15 -> 17800 s, excess 2300 over the secant slope 892.3 -> step 3 -> 12 (15123 s)
        // fits. The probe slope (17800 - 1000) / 14 = 1200 would step 2 to 13 (16015 s) and
        // overshoot again.
        assert_eq!(sliced, vec![15, 12]);
        assert_eq!(fit.count, 12);
        assert_eq!(fit.samples.last().unwrap().count, 12);
    }

    #[test]
    fn shrink_slope_prefers_the_secant_and_falls_back_to_the_probe_line() {
        let probe = sample(1, 1_000.0);
        let max = sample(41, 41_000.0);
        assert!((shrink_slope(probe, max, sample(15, 17_800.0)) - 23_200.0 / 26.0).abs() < 1e-9);
        // No comparable pair (the sample is the max plate itself): the probe line.
        assert!((shrink_slope(probe, max, sample(41, 41_000.0)) - 1_000.0).abs() < 1e-9);
    }

    #[test]
    fn fit_window_gives_up_after_four_window_slices() {
        let mut sliced = 0;
        let error = fit_window(sample(1, 1_000.0), sample(41, 41_000.0), 15_500.0, |n| {
            sliced += 1;
            // Every measured prefix lands just over the window.
            Ok(sample(n, 15_501.0))
        })
        .unwrap_err();
        assert_eq!(sliced, MAX_WINDOW_SLICES);
        assert!(error.to_string().contains("time window"));
    }
}

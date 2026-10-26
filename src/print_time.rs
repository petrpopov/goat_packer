//! Corrected print-time model shared by `colored_3mf_packer` and `daily_scheduler`.
//!
//! The slicer's own estimate assumes a per-filament-change duration taken from the project
//! settings (e.g. 23 s for an A2L with AMS lite), while the AMS actually attached to the printer
//! takes much longer (gen-1 AMS measured at ~91-95 s on both P1S and A2L). On a multi-colour
//! plate with hundreds of changes that difference is hours, so every time budget must use the
//! corrected value. Details and measurements:
//! `docs/superpowers/specs/2026-10-26-time-model-and-window-design.md`.

use serde::{Deserialize, Serialize};

/// How to turn a slicer estimate into a realistic wall-clock time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeCorrection {
    /// Real duration of one filament change; `None` disables the change correction (tool
    /// changers, or the user did not ask for it).
    pub real_change_s: Option<f64>,
    /// Fixed preparation time (bed swap, loading) added once per plate.
    pub prep_s: f64,
    /// Relative safety margin, e.g. 0.1 for 10 %.
    pub margin: f64,
}

/// Lenient read of a project-settings value: settings are JSON strings (`"23"`) or arrays of
/// strings (per-extruder values; element 0 is used). Plain numbers are accepted too.
fn setting_number(settings: &serde_json::Value, key: &str) -> Option<f64> {
    let value = settings.get(key)?;
    let value = match value {
        serde_json::Value::Array(items) => items.first()?,
        other => other,
    };
    match value {
        serde_json::Value::String(text) => text.trim().parse().ok(),
        serde_json::Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

/// Seconds the project's slicer settings assume for one filament change (spec "Facts").
///
/// BambuStudio uses the per-AMS-type load/unload pair selected by `default_ams_type` when it is
/// non-zero; P1S projects have no `default_ams_type` and zeroed AMS pairs, and the slicer then
/// falls back to the machine load/unload times. No keys at all means the slicer adds nothing.
pub fn slicer_change_seconds(settings: &serde_json::Value) -> f64 {
    let ams_suffix = match setting_number(settings, "default_ams_type").map(|v| v.round() as i64) {
        Some(1) => Some("ams"),
        Some(2) => Some("ams_lite"),
        Some(3) => Some("n3f_s"),
        _ => None,
    };
    if let Some(suffix) = ams_suffix {
        let load = setting_number(settings, &format!("ams_filament_load_time_{suffix}"))
            .unwrap_or(0.0);
        let unload = setting_number(settings, &format!("ams_filament_unload_time_{suffix}"))
            .unwrap_or(0.0);
        if load + unload > 0.0 {
            return load + unload;
        }
    }
    setting_number(settings, "machine_load_filament_time").unwrap_or(0.0)
        + setting_number(settings, "machine_unload_filament_time").unwrap_or(0.0)
}

/// `(estimate + changes * (real - slicer) + prep) * (1 + margin)`; `real == None` drops the
/// change term. A negative difference (project already assumes a slower AMS) is applied as is.
pub fn corrected_seconds(
    estimate_s: f64,
    changes: u64,
    slicer_change_s: f64,
    correction: &TimeCorrection,
) -> f64 {
    let change_term = correction
        .real_change_s
        .map_or(0.0, |real| changes as f64 * (real - slicer_change_s));
    (estimate_s + change_term + correction.prep_s) * (1.0 + correction.margin)
}

/// One measured plate: object count, raw slicer estimate, change count, corrected time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeSample {
    pub count: usize,
    pub estimate_s: f64,
    pub changes: u64,
    pub corrected_s: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowFit {
    /// The larger sample already fits; the window does not bind.
    All,
    /// Largest object count predicted to fit.
    Count(usize),
    /// Even one object is predicted to exceed the window.
    NotEvenOne,
}

/// Floating-point slack so a count landing exactly on the window boundary is not lost to
/// rounding in the affine solve.
const SOLVE_EPSILON: f64 = 1e-9;

/// Affine line through two samples (`a.count < b.count`); largest N in `1..=b.count` with
/// `line(N) <= window_s`. `b` fits => `All`; `line(1) > window` => `NotEvenOne`.
pub fn count_for_window(a: TimeSample, b: TimeSample, window_s: f64) -> WindowFit {
    if b.corrected_s <= window_s {
        return WindowFit::All;
    }
    if b.count <= a.count {
        // Degenerate (single-object plate): b is the only real data point and it does not fit.
        return WindowFit::NotEvenOne;
    }
    let slope = (b.corrected_s - a.corrected_s) / (b.count - a.count) as f64;
    let line_at_one = a.corrected_s + slope * (1.0 - a.count as f64);
    if line_at_one > window_s || slope <= 0.0 {
        // A non-increasing line whose larger end is over the window is over it everywhere.
        return WindowFit::NotEvenOne;
    }
    let solved = (window_s - a.corrected_s) / slope + a.count as f64 + SOLVE_EPSILON;
    // b itself is known not to fit, so never hand it back as the answer.
    let upper = b.count.saturating_sub(1).max(1);
    let n = if solved < 1.0 { 1 } else { solved.floor() as usize };
    WindowFit::Count(n.clamp(1, upper))
}

/// Next smaller N after a measured overshoot: `n - max(1, ceil(excess / slope))`, min 1.
/// A non-positive slope (noisy samples) carries no step information, so step by one.
pub fn shrink_count(n: usize, corrected_s: f64, window_s: f64, slope_s_per_object: f64) -> usize {
    let excess = (corrected_s - window_s).max(0.0);
    let step = if slope_s_per_object > 0.0 {
        ((excess / slope_s_per_object).ceil() as usize).max(1)
    } else {
        1
    };
    n.saturating_sub(step).max(1)
}

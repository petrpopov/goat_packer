//! Stage events for the webapp's progress bar (spec 2026-10-31-support-landing-and-progress-design,
//! "Progress protocol"). The human line always goes to stderr; with `GOAT_LOG_FORMAT=json` a
//! machine-readable `{"event":"stage",...}` follows it, so the job view can name the stage and
//! interpolate a long slice by its expected duration instead of parsing English log lines.
use serde_json::json;

/// Stage ids the webapp maps to its own labels; keep in sync with `ProgressTracker.kt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    ProbeSlice,
    Packing,
    Assembling,
    FinalSlice,
    GcodeCheck,
    Repacking,
    WindowSlice,
    Writing,
}

impl Stage {
    pub fn id(self) -> &'static str {
        match self {
            Stage::ProbeSlice => "probe_slice",
            Stage::Packing => "packing",
            Stage::Assembling => "assembling",
            Stage::FinalSlice => "final_slice",
            Stage::GcodeCheck => "gcode_check",
            Stage::Repacking => "repacking",
            Stage::WindowSlice => "window_slice",
            Stage::Writing => "writing",
        }
    }
}

/// Why a repack runs; the webapp names it in the status line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepackReason {
    /// The final tower outgrew the probe keepout and reaches a placement.
    TowerGrown,
    /// Supports touched a foreign body: the support patch was widened to the printed one.
    SupportContact,
    /// Still touching after learning: the strict silhouette rule.
    SupportStrict,
    /// The slicer rejected the plate with the tower in a bed corner: back to the source spot.
    TowerCornerFailed,
}

impl RepackReason {
    pub fn id(self) -> &'static str {
        match self {
            RepackReason::TowerGrown => "tower_grown",
            RepackReason::SupportContact => "support_contact",
            RepackReason::SupportStrict => "support_strict",
            RepackReason::TowerCornerFailed => "tower_corner_failed",
        }
    }
}

/// The `repacking` stage event with its `reason`.
pub fn repack_event(reason: RepackReason, message: &str) -> serde_json::Value {
    let mut event = stage_event(Stage::Repacking, message, None, None);
    event["reason"] = reason.id().into();
    event
}

/// Announces a repack (stage `repacking` with its reason).
pub fn repack(reason: RepackReason, message: &str) {
    eprintln!("{message}");
    if structured() {
        eprintln!("{}", repack_event(reason, message));
    }
}

pub fn structured() -> bool {
    std::env::var("GOAT_LOG_FORMAT").as_deref() == Ok("json")
}

/// The JSON stage event (without printing it).
pub fn stage_event(
    stage: Stage,
    message: &str,
    expected_s: Option<f64>,
    objects: Option<usize>,
) -> serde_json::Value {
    json!({
        "event": "stage",
        "stage": stage.id(),
        "message": message,
        "expected_s": expected_s.map(|s| (s * 10.0).round() / 10.0),
        "objects": objects,
    })
}

/// Announces a stage: the human line, then (structured mode) the JSON event.
pub fn stage(stage: Stage, message: &str, expected_s: Option<f64>, objects: Option<usize>) {
    eprintln!("{message}");
    if structured() {
        eprintln!("{}", stage_event(stage, message, expected_s, objects));
    }
}

/// Rough slicing time of an `objects`-object plate from the one-object probe. The P1S goat
/// measured 22 s → 317 s for 37 objects on the server (0.37 probe per extra object) but
/// 4 s → 136 s for 43 on an M-series Mac (0.78): the probe is mostly start-up, so the ratio is
/// a compromise. Once a full plate was sliced, `expected_reslice_seconds` scales the measured
/// time instead. Only drives the progress bar, never a decision.
pub fn expected_slice_seconds(probe_seconds: f64, objects: usize) -> f64 {
    probe_seconds * (1.0 + 0.5 * objects.saturating_sub(1) as f64)
}

/// Expected time of another plate slice from a measured one (`seconds` for `measured_objects`).
pub fn expected_reslice_seconds(seconds: f64, measured_objects: usize, objects: usize) -> f64 {
    seconds * objects as f64 / measured_objects.max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_event_carries_id_message_and_rounded_expectation() {
        let event = stage_event(Stage::FinalSlice, "slicing final plate (37 objects)...", Some(317.04), Some(37));
        assert_eq!(event["event"], "stage");
        assert_eq!(event["stage"], "final_slice");
        assert_eq!(event["expected_s"], 317.0);
        assert_eq!(event["objects"], 37);
        let event = stage_event(Stage::ProbeSlice, "probing", None, None);
        assert!(event["expected_s"].is_null() && event["objects"].is_null());
    }

    #[test]
    fn repack_event_names_its_reason() {
        let event = repack_event(RepackReason::SupportContact, "re-packing");
        assert_eq!(event["stage"], "repacking");
        assert_eq!(event["reason"], "support_contact");
    }

    #[test]
    fn expected_slice_time_scales_with_objects() {
        assert_eq!(expected_slice_seconds(22.0, 37), 22.0 * 19.0);
        assert_eq!(expected_slice_seconds(22.0, 1), 22.0);
        assert_eq!(expected_slice_seconds(22.0, 0), 22.0);
        assert_eq!(expected_reslice_seconds(136.0, 43, 42), 136.0 * 42.0 / 43.0);
        assert_eq!(expected_reslice_seconds(10.0, 0, 3), 30.0);
    }
}

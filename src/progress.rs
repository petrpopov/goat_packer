//! Stage events for the webapp's progress bar (spec 2026-10-31-support-landing-and-progress-design,
//! "Progress protocol"). The human line always goes to stderr; with `GOAT_LOG_FORMAT=json` a
//! machine-readable `{"event":"stage",...}` follows it, so the job view can name the stage and
//! interpolate a long slice by its expected duration instead of parsing English log lines.
use geo::{Coord, LineString, Simplify};
use serde_json::json;

/// Stage ids the webapp maps to its own labels; keep in sync with `ProgressTracker.kt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    ProbeSlice,
    Packing,
    SampleSlice,
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
            Stage::SampleSlice => "sample_slice",
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
    /// Supports touched a foreign body: only the touching objects moved apart, no repack.
    SupportNudge,
    /// Supports printed past the plate edge: the support patch was widened to the printed one.
    SupportOffPlate,
}

impl RepackReason {
    pub fn id(self) -> &'static str {
        match self {
            RepackReason::TowerGrown => "tower_grown",
            RepackReason::SupportContact => "support_contact",
            RepackReason::SupportStrict => "support_strict",
            RepackReason::TowerCornerFailed => "tower_corner_failed",
            RepackReason::SupportNudge => "support_nudge",
            RepackReason::SupportOffPlate => "support_off_plate",
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


/// Size the packer aims for with one `layout` line. The webapp polls job status every second and
/// the backend reads packer output with a 64 KiB line cap, so the footprint is simplified until
/// it fits; with hundreds of objects the placements alone can still exceed it. A line over the
/// cap is truncated by the backend and then ignored (unparseable), which is harmless.
pub const LAYOUT_EVENT_MAX_BYTES: usize = 16_000;

/// Which pass produced the layout the webapp should draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutSource {
    /// First winning layout.
    Pack,
    /// Layout after a repack (grown tower, support contact/strict, corner tower failed).
    Repack,
    /// Prefix kept for the print-time window.
    Window,
}

impl LayoutSource {
    pub fn id(self) -> &'static str {
        match self {
            LayoutSource::Pack => "pack",
            LayoutSource::Repack => "repack",
            LayoutSource::Window => "window",
        }
    }
}

/// Rounds to `decimals` places through an integer scale: `(v / step).round() * step` leaves float
/// noise in the JSON (`143.67000000000002`).
fn round_to(value: f64, decimals: i32) -> f64 {
    let scale = 10f64.powi(decimals);
    (value * scale).round() / scale
}

fn rect_json(rect: &crate::Rect) -> serde_json::Value {
    json!([round_to(rect.min_x, 1), round_to(rect.min_y, 1), round_to(rect.max_x, 1), round_to(rect.max_y, 1)])
}

/// Footprint rings simplified with Ramer–Douglas–Peucker at `epsilon_mm`, rounded to 0.01 mm.
/// Rings that collapse below a triangle are dropped.
fn simplified_loops(loops: &[Vec<crate::Point>], epsilon_mm: f64) -> Vec<Vec<[f64; 2]>> {
    loops
        .iter()
        .filter_map(|ring| {
            let line: LineString<f64> = ring.iter().map(|p| Coord { x: p.x, y: p.y }).collect();
            let simple = line.simplify(epsilon_mm);
            let points: Vec<[f64; 2]> = simple.0.iter().map(|c| [round_to(c.x, 2), round_to(c.y, 2)]).collect();
            (points.len() >= 4).then_some(points)
        })
        .collect()
}

/// The `layout` event: footprint loops once (centred on the mesh bbox centre, mm) plus
/// `[x_mm, y_mm, angle_deg]` per placement — the webapp rotates the loops by the angle and
/// translates them, the same math as `placement_world_loops`.
pub fn layout_event(
    source: LayoutSource,
    loops: &[Vec<crate::Point>],
    placements: &[crate::PackedPlacement],
    bed: crate::Rect,
    keepouts: &[crate::Rect],
    tower: Option<crate::Rect>,
) -> serde_json::Value {
    let placements_json: Vec<serde_json::Value> = placements
        .iter()
        .map(|p| json!([round_to(p.x_mm, 2), round_to(p.y_mm, 2), round_to(p.angle_deg, 1)]))
        .collect();
    let mut epsilon_mm = 0.1;
    loop {
        let event = json!({
            "event": "layout",
            "source": source.id(),
            "objects": placements.len(),
            "bed": rect_json(&bed),
            "keepouts": keepouts.iter().map(rect_json).collect::<Vec<_>>(),
            "tower": tower.as_ref().map(rect_json),
            "footprint": simplified_loops(loops, epsilon_mm),
            "placements": placements_json,
            "epsilon_mm": epsilon_mm,
        });
        // A coarse contour is still a faithful picture of where things stand; give up shrinking
        // past 5 mm (hundreds of tiny objects) and let the backend's 64 KiB line cap decide.
        if event.to_string().len() <= LAYOUT_EVENT_MAX_BYTES || epsilon_mm >= 5.0 {
            return event;
        }
        epsilon_mm *= 2.0;
    }
}

/// Prints the layout event for the webapp (machine-readable mode only; the human log already
/// says how many objects were packed).
pub fn layout(
    source: LayoutSource,
    loops: &[Vec<crate::Point>],
    placements: &[crate::PackedPlacement],
    bed: crate::Rect,
    keepouts: &[crate::Rect],
    tower: Option<crate::Rect>,
) {
    if structured() {
        eprintln!("{}", layout_event(source, loops, placements, bed, keepouts, tower));
    }
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

    #[test]
    fn layout_event_fits_the_line_budget_and_keeps_every_placement() {
        #[derive(serde::Deserialize)]
        struct Loops {
            loops: Vec<Vec<crate::Point>>,
        }
        let fixture: Loops = serde_json::from_str(include_str!("../tests/fixtures/goat4.json")).unwrap();
        let placements: Vec<crate::PackedPlacement> = (0..60)
            .map(|i| crate::PackedPlacement {
                x_mm: 20.0 + (i % 6) as f64 * 37.5,
                y_mm: 20.0 + (i / 6) as f64 * 21.25,
                angle_deg: if i % 2 == 0 { 0.0 } else { 180.0 },
            })
            .collect();
        let bed = crate::Rect { min_x: 0.0, min_y: 0.0, max_x: 256.0, max_y: 256.0 };
        let corner = crate::Rect { min_x: 0.0, min_y: 0.0, max_x: 22.0, max_y: 32.0 };
        let event = layout_event(LayoutSource::Pack, &fixture.loops, &placements, bed, &[corner], None);

        assert!(event.to_string().len() <= LAYOUT_EVENT_MAX_BYTES, "{} bytes", event.to_string().len());
        assert_eq!(event["event"], "layout");
        assert_eq!(event["source"], "pack");
        assert_eq!(event["objects"], 60);
        assert_eq!(event["placements"].as_array().unwrap().len(), 60);
        assert_eq!(event["placements"][1], serde_json::json!([57.5, 20.0, 180.0]));
        assert_eq!(event["bed"], serde_json::json!([0.0, 0.0, 256.0, 256.0]));
        assert!(event["tower"].is_null());
        let ring = event["footprint"][0].as_array().unwrap();
        assert!(ring.len() >= 4 && ring.len() < fixture.loops[0].len(), "simplified: {} points", ring.len());
        assert_eq!(ring.first(), ring.last(), "rings stay closed");
    }

    #[test]
    fn layout_event_numbers_serialise_without_float_noise() {
        let placements = [crate::PackedPlacement { x_mm: 143.67, y_mm: 20.0 + 0.1 + 0.2, angle_deg: 90.0 }];
        let bed = crate::Rect { min_x: 0.0, min_y: 0.0, max_x: 256.0, max_y: 256.0 };
        let event = layout_event(LayoutSource::Pack, &[], &placements, bed, &[], None);
        let x = event["placements"][0][0].to_string();
        assert!(x.contains("143.67") && !x.contains("143.670000"), "{x}");
        assert_eq!(event["placements"][0][1].to_string(), "20.3");
    }

    #[test]
    fn layout_source_ids() {
        assert_eq!(LayoutSource::Pack.id(), "pack");
        assert_eq!(LayoutSource::Repack.id(), "repack");
        assert_eq!(LayoutSource::Window.id(), "window");
    }
}

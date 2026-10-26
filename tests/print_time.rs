use goat_packer::print_time::{
    TimeCorrection, TimeSample, WindowFit, corrected_seconds, count_for_window, shrink_count,
    slicer_change_seconds,
};
use serde_json::json;

/// A2L project settings as BambuStudio writes them (all values are strings).
fn a2l_settings(ams_type: &str) -> serde_json::Value {
    json!({
        "default_ams_type": ams_type,
        "ams_filament_load_time_ams": "23",
        "ams_filament_unload_time_ams": "21",
        "ams_filament_load_time_ams_lite": "11",
        "ams_filament_unload_time_ams_lite": "12",
        "ams_filament_load_time_n3f_s": "18",
        "ams_filament_unload_time_n3f_s": "19",
        "machine_load_filament_time": "11",
        "machine_unload_filament_time": "12"
    })
}

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

#[test]
fn slicer_change_seconds_uses_the_per_ams_type_pair() {
    assert_close(slicer_change_seconds(&a2l_settings("1")), 44.0);
    assert_close(slicer_change_seconds(&a2l_settings("2")), 23.0);
    assert_close(slicer_change_seconds(&a2l_settings("3")), 37.0);
}

#[test]
fn slicer_change_seconds_p1s_falls_back_to_machine_times() {
    let settings = json!({
        "ams_filament_load_time_ams": "0",
        "ams_filament_unload_time_ams": "0",
        "machine_load_filament_time": "29",
        "machine_unload_filament_time": "28"
    });
    assert_close(slicer_change_seconds(&settings), 57.0);
}

#[test]
fn slicer_change_seconds_zero_ams_pair_falls_back_to_machine_times() {
    let settings = json!({
        "default_ams_type": "1",
        "ams_filament_load_time_ams": "0",
        "ams_filament_unload_time_ams": "0",
        "machine_load_filament_time": "29",
        "machine_unload_filament_time": "28"
    });
    assert_close(slicer_change_seconds(&settings), 57.0);
}

#[test]
fn slicer_change_seconds_without_any_key_is_zero() {
    assert_close(slicer_change_seconds(&json!({})), 0.0);
}

#[test]
fn slicer_change_seconds_reads_array_values() {
    let settings = json!({
        "default_ams_type": ["2"],
        "ams_filament_load_time_ams_lite": ["11", "11"],
        "ams_filament_unload_time_ams_lite": ["12"],
    });
    assert_close(slicer_change_seconds(&settings), 23.0);
    let machine = json!({
        "machine_load_filament_time": ["29"],
        "machine_unload_filament_time": ["28"]
    });
    assert_close(slicer_change_seconds(&machine), 57.0);
}

#[test]
fn corrected_seconds_without_real_change_only_adds_prep_and_margin() {
    let correction = TimeCorrection {
        real_change_s: None,
        prep_s: 600.0,
        margin: 0.1,
    };
    assert_close(corrected_seconds(10_000.0, 227, 44.0, &correction), 11_660.0);
}

#[test]
fn corrected_seconds_applies_change_difference() {
    let correction = TimeCorrection {
        real_change_s: Some(95.0),
        prep_s: 0.0,
        margin: 0.0,
    };
    // A2L ams_lite assumption 23 s, real 95 s: 100 changes add 7200 s.
    assert_close(corrected_seconds(10_000.0, 100, 23.0, &correction), 17_200.0);
}

#[test]
fn corrected_seconds_applies_negative_difference_as_is() {
    let correction = TimeCorrection {
        real_change_s: Some(30.0),
        prep_s: 0.0,
        margin: 0.0,
    };
    assert_close(corrected_seconds(10_000.0, 10, 57.0, &correction), 9_730.0);
}

#[test]
fn corrected_seconds_margin_scales_everything() {
    let correction = TimeCorrection {
        real_change_s: Some(95.0),
        prep_s: 300.0,
        margin: 0.1,
    };
    // (1000 + 10 * (95 - 45) + 300) * 1.1 = 1980
    assert_close(corrected_seconds(1_000.0, 10, 45.0, &correction), 1_980.0);
}

fn sample(count: usize, corrected_s: f64) -> TimeSample {
    TimeSample {
        count,
        estimate_s: corrected_s,
        changes: 0,
        corrected_s,
    }
}

#[test]
fn count_for_window_all_when_max_plate_fits() {
    assert_eq!(
        count_for_window(sample(1, 1_000.0), sample(40, 40_000.0), 40_000.0),
        WindowFit::All
    );
}

#[test]
fn count_for_window_solves_affine_line() {
    // line(N) = 10000 + 1000 * (N - 1); window 25500 -> N = 16 (25000), 17 would be 26000.
    assert_eq!(
        count_for_window(sample(1, 10_000.0), sample(41, 50_000.0), 25_500.0),
        WindowFit::Count(16)
    );
}

#[test]
fn count_for_window_exact_boundary_is_included() {
    assert_eq!(
        count_for_window(sample(1, 10_000.0), sample(41, 50_000.0), 25_000.0),
        WindowFit::Count(16)
    );
}

#[test]
fn count_for_window_not_even_one() {
    assert_eq!(
        count_for_window(sample(1, 10_000.0), sample(41, 50_000.0), 9_000.0),
        WindowFit::NotEvenOne
    );
}

#[test]
fn count_for_window_clamps_into_range() {
    // Flat or decreasing line through samples where b does not fit: never above b.count - 1
    // makes no sense, but the result must stay in 1..=b.count.
    match count_for_window(sample(1, 1_000.0), sample(10, 1_000.0 + 1e-12), 1_000.0) {
        WindowFit::Count(n) => assert!((1..=10).contains(&n)),
        other => panic!("unexpected {other:?}"),
    }
    // Barely-fitting line(1) with a steep slope still yields 1.
    assert_eq!(
        count_for_window(sample(1, 1_000.0), sample(10, 100_000.0), 1_000.0),
        WindowFit::Count(1)
    );
}

#[test]
fn shrink_count_steps_by_excess_over_slope() {
    // excess 2500, slope 1000 -> ceil 3.
    assert_eq!(shrink_count(16, 28_000.0, 25_500.0, 1_000.0), 13);
    // tiny excess still removes one object.
    assert_eq!(shrink_count(16, 25_501.0, 25_500.0, 1_000.0), 15);
}

#[test]
fn shrink_count_guards_zero_slope_and_minimum() {
    assert_eq!(shrink_count(16, 28_000.0, 25_500.0, 0.0), 15);
    assert_eq!(shrink_count(16, 28_000.0, 25_500.0, -5.0), 15);
    assert_eq!(shrink_count(3, 100_000.0, 25_500.0, 1_000.0), 1);
    assert_eq!(shrink_count(1, 100_000.0, 25_500.0, 1_000.0), 1);
}

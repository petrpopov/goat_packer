use goat_packer::scheduler::{BatchTiming, ScheduleConfig, optimize_schedule};

#[test]
fn shorter_day_batches_plus_overnight_beat_full_plate_repetition() {
    let batches = vec![
        BatchTiming::new(5, 300 * 60, 0, 50.0),
        BatchTiming::new(4, 190 * 60, 0, 40.0),
    ];
    let config = ScheduleConfig {
        day_start_seconds: 10 * 3600,
        night_start_seconds: 10 * 3600 + 400 * 60,
        horizon_end_seconds: 10 * 3600 + 600 * 60,
        turnaround_seconds: 10 * 60,
    };

    let result = optimize_schedule("Test", &batches, &config).unwrap();

    assert_eq!(result.total_objects, 12);
    assert_eq!(result.jobs.len(), 3);
    assert_eq!(
        result
            .jobs
            .iter()
            .map(|job| job.batch_size)
            .collect::<Vec<_>>(),
        [4, 4, 4]
    );
    assert_eq!(result.jobs[0].start_seconds, 10 * 3600);
    assert_eq!(
        result.jobs.last().unwrap().start_seconds,
        config.night_start_seconds
    );
    assert_eq!(result.jobs[2].end_seconds, 10 * 3600 + 590 * 60);
    assert_eq!(result.jobs.iter().filter(|job| job.overnight).count(), 1);
}

#[test]
fn ties_prefer_fewer_plate_changes_then_earlier_completion() {
    let batches = vec![
        BatchTiming::new(6, 400 * 60, 0, 60.0),
        BatchTiming::new(3, 195 * 60, 0, 30.0),
    ];
    let config = ScheduleConfig {
        day_start_seconds: 0,
        night_start_seconds: 0,
        horizon_end_seconds: 400 * 60,
        turnaround_seconds: 10 * 60,
    };

    let result = optimize_schedule("Test", &batches, &config).unwrap();

    assert_eq!(result.total_objects, 6);
    assert_eq!(result.jobs.len(), 1);
    assert_eq!(result.jobs[0].batch_size, 6);
}

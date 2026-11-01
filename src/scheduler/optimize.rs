//! The day/night schedule: which batches run when, to print the most objects by the horizon.

use super::{SchedulerResult, batch_curve::BatchTiming};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ScheduleConfig {
    pub day_start_seconds: u64,
    pub night_start_seconds: u64,
    pub horizon_end_seconds: u64,
    pub turnaround_seconds: u64,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ScheduleJob {
    pub sequence: usize,
    pub batch_size: usize,
    pub start_seconds: u64,
    pub end_seconds: u64,
    pub slicer_seconds: u64,
    pub preparation_seconds: u64,
    pub filament_grams: f64,
    pub cumulative_objects: usize,
    pub overnight: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct OptimizedSchedule {
    pub printer: String,
    pub total_objects: usize,
    pub total_filament_grams: f64,
    pub completion_seconds: u64,
    pub jobs: Vec<ScheduleJob>,
}

#[derive(Clone)]
struct ScheduleState {
    total_objects: usize,
    total_filament_grams: f64,
    jobs: Vec<ScheduleJob>,
}

pub fn optimize_schedule(
    printer: &str,
    batches: &[BatchTiming],
    config: &ScheduleConfig,
) -> SchedulerResult<OptimizedSchedule> {
    if batches.is_empty() {
        return Err("at least one batch timing is required".into());
    }
    if config.day_start_seconds > config.night_start_seconds
        || config.night_start_seconds > config.horizon_end_seconds
    {
        return Err("invalid schedule time window".into());
    }
    if batches
        .iter()
        .any(|batch| batch.batch_size == 0 || batch.job_seconds() == 0)
    {
        return Err("batch size and duration must be positive".into());
    }

    let latest_day_end = config
        .night_start_seconds
        .saturating_sub(config.turnaround_seconds);
    let mut day_states: Vec<Option<ScheduleState>> =
        vec![None; usize::try_from(config.night_start_seconds)? + 1];
    if config.day_start_seconds < config.night_start_seconds {
        for batch in batches {
            insert_day_job_state(
                &mut day_states,
                None,
                batch,
                config.day_start_seconds,
                latest_day_end,
            )?;
        }
        for end_index in
            usize::try_from(config.day_start_seconds)?..=usize::try_from(latest_day_end)?
        {
            let Some(state) = day_states[end_index].clone() else {
                continue;
            };
            let next_start = u64::try_from(end_index)?.saturating_add(config.turnaround_seconds);
            for batch in batches {
                insert_day_job_state(
                    &mut day_states,
                    Some(&state),
                    batch,
                    next_start,
                    latest_day_end,
                )?;
            }
        }
    }

    let mut day_candidates: Vec<Option<ScheduleState>> = vec![None];
    day_candidates.extend(day_states.into_iter().flatten().map(Some));
    let mut best: Option<(u64, ScheduleState)> = None;
    for day in day_candidates {
        for batch in batches {
            let end_seconds = config
                .night_start_seconds
                .saturating_add(batch.job_seconds());
            if end_seconds > config.horizon_end_seconds {
                continue;
            }
            let mut state = day.clone().unwrap_or(ScheduleState {
                total_objects: 0,
                total_filament_grams: 0.0,
                jobs: Vec::new(),
            });
            state.total_objects += batch.batch_size;
            state.total_filament_grams += batch.filament_grams;
            state.jobs.push(ScheduleJob {
                sequence: state.jobs.len() + 1,
                batch_size: batch.batch_size,
                start_seconds: config.night_start_seconds,
                end_seconds,
                slicer_seconds: batch.slicer_seconds,
                preparation_seconds: batch.preparation_seconds,
                filament_grams: batch.filament_grams,
                cumulative_objects: state.total_objects,
                overnight: true,
            });
            let replace = best.as_ref().is_none_or(|(best_end, existing)| {
                compare_schedule_states(&state, end_seconds, existing, *best_end).is_gt()
            });
            if replace {
                best = Some((end_seconds, state));
            }
        }
    }
    let (completion_seconds, best) = best.ok_or("no overnight batch fits the horizon")?;
    Ok(OptimizedSchedule {
        printer: printer.to_owned(),
        total_objects: best.total_objects,
        total_filament_grams: best.total_filament_grams,
        completion_seconds,
        jobs: best.jobs,
    })
}

fn insert_day_job_state(
    states: &mut [Option<ScheduleState>],
    previous: Option<&ScheduleState>,
    batch: &BatchTiming,
    start_seconds: u64,
    latest_end_seconds: u64,
) -> SchedulerResult<()> {
    let end_seconds = start_seconds.saturating_add(batch.job_seconds());
    if end_seconds > latest_end_seconds {
        return Ok(());
    }
    let mut state = previous.cloned().unwrap_or(ScheduleState {
        total_objects: 0,
        total_filament_grams: 0.0,
        jobs: Vec::new(),
    });
    state.total_objects += batch.batch_size;
    state.total_filament_grams += batch.filament_grams;
    state.jobs.push(ScheduleJob {
        sequence: state.jobs.len() + 1,
        batch_size: batch.batch_size,
        start_seconds,
        end_seconds,
        slicer_seconds: batch.slicer_seconds,
        preparation_seconds: batch.preparation_seconds,
        filament_grams: batch.filament_grams,
        cumulative_objects: state.total_objects,
        overnight: false,
    });
    let index = usize::try_from(end_seconds)?;
    let replace = states[index].as_ref().is_none_or(|existing| {
        compare_schedule_states(&state, end_seconds, existing, end_seconds).is_gt()
    });
    if replace {
        states[index] = Some(state);
    }
    Ok(())
}

fn compare_schedule_states(
    a: &ScheduleState,
    end_a: u64,
    b: &ScheduleState,
    end_b: u64,
) -> std::cmp::Ordering {
    a.total_objects
        .cmp(&b.total_objects)
        .then_with(|| b.jobs.len().cmp(&a.jobs.len()))
        .then_with(|| end_b.cmp(&end_a))
}

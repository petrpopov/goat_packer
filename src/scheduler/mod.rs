//! Scheduling a day of prints: what a slicer reports for a plate (`estimate`), how print time
//! grows with the batch size (`batch_curve`), the day/night optimizer (`optimize`), and the 3MF
//! surgery that cuts a packed plate into batches (`subset`) or several plates (`plates`).

use std::error::Error;

mod archive;
mod batch_curve;
mod estimate;
mod optimize;
mod plates;
mod subset;
mod xml_util;

pub type SchedulerResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub use batch_curve::{BatchCurve, BatchTiming, least_squares_line, next_refinement_count};
pub use estimate::{SliceEstimate, parse_bambu_result, parse_duration_text, parse_orca_gcode};
pub use optimize::{OptimizedSchedule, ScheduleConfig, ScheduleJob, optimize_schedule};
pub use plates::{PlateSpec, build_schedule_plates_3mf};
pub use subset::{object_count_3mf, plate_count_3mf, subset_3mf, subset_3mf_items};

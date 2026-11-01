//! What a slicer says about a plate: print time, filament, filament changes.

use super::SchedulerResult;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct SliceEstimate {
    pub object_count: usize,
    pub model_seconds: u64,
    pub slicer_total_seconds: u64,
    pub filament_grams: f64,
    /// Filament changes in the slice (Bambu `filament_change_times`, Orca `; toolchange #`
    /// lines). Feeds the AMS change correction in `print_time`; default 0 keeps reports that
    /// predate the field loadable.
    #[serde(default)]
    pub filament_changes: u64,
}

pub fn parse_bambu_result(data: &[u8]) -> SchedulerResult<SliceEstimate> {
    let root: serde_json::Value = serde_json::from_slice(data)?;
    let plate = root
        .get("sliced_plates")
        .and_then(serde_json::Value::as_array)
        .and_then(|plates| plates.first())
        .ok_or("Bambu result contains no sliced plate")?;
    let object_count = plate
        .get("objects")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let model_seconds = numeric_value(plate, "main_predication")?.round() as u64;
    let slicer_total_seconds = numeric_value(plate, "total_predication")?.round() as u64;
    let filament_grams = plate
        .get("filaments")
        .and_then(serde_json::Value::as_array)
        .map(|filaments| {
            filaments
                .iter()
                .filter_map(|filament| filament.get("total_used_g"))
                .filter_map(serde_json::Value::as_f64)
                .sum()
        })
        .unwrap_or(0.0);
    // Absent on older BambuStudio builds and on single-filament plates: no changes.
    let filament_changes = plate
        .get("filament_change_times")
        .and_then(serde_json::Value::as_f64)
        .map_or(0, |changes| changes.max(0.0).round() as u64);
    Ok(SliceEstimate {
        object_count,
        model_seconds,
        slicer_total_seconds,
        filament_grams,
        filament_changes,
    })
}

pub fn parse_orca_gcode(data: &[u8], object_count: usize) -> SchedulerResult<SliceEstimate> {
    let text = String::from_utf8_lossy(data);
    let duration_line = text
        .lines()
        .find(|line| line.contains("estimated printing time (normal mode)"))
        .ok_or("Orca G-code has no normal-mode time estimate")?;
    let duration_text = duration_line
        .split_once('=')
        .map(|(_, value)| value.trim())
        .ok_or("Orca time estimate has no value")?;
    let seconds = parse_duration_text(duration_text)?;
    let filament_grams = text
        .lines()
        .find(|line| line.contains("total filament used [g]"))
        .or_else(|| text.lines().find(|line| line.contains("filament used [g]")))
        .and_then(|line| line.split_once('=').map(|(_, value)| value.trim()))
        .map(parse_number_list_sum)
        .transpose()?
        .unwrap_or(0.0);
    let filament_changes = crate::print_time::orca_toolchange_count(&text);
    Ok(SliceEstimate {
        object_count,
        model_seconds: seconds,
        slicer_total_seconds: seconds,
        filament_grams,
        filament_changes,
    })
}

pub fn parse_duration_text(text: &str) -> SchedulerResult<u64> {
    let mut total = 0u64;
    let mut found = false;
    for token in text.split_whitespace() {
        let Some(unit) = token.chars().last() else {
            continue;
        };
        let multiplier = match unit {
            'h' => 3600,
            'm' => 60,
            's' => 1,
            _ => continue,
        };
        let value: u64 = token[..token.len() - unit.len_utf8()].parse()?;
        total = total.saturating_add(value.saturating_mul(multiplier));
        found = true;
    }
    if !found {
        return Err(format!("duration has no h/m/s tokens: {text}").into());
    }
    Ok(total)
}

fn numeric_value(object: &serde_json::Value, key: &str) -> SchedulerResult<f64> {
    object
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| format!("numeric key missing: {key}").into())
}

fn parse_number_list_sum(text: &str) -> SchedulerResult<f64> {
    text.split(',')
        .map(|value| value.trim().parse::<f64>().map_err(Into::into))
        .try_fold(0.0, |sum, value: SchedulerResult<f64>| {
            value.map(|number| sum + number)
        })
}

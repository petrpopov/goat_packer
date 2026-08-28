use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs::File,
    io::{Cursor, Read, Write},
    path::Path,
};

use xmltree::{Element, EmitterConfig, XMLNode};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

pub type SchedulerResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct SliceEstimate {
    pub object_count: usize,
    pub model_seconds: u64,
    pub slicer_total_seconds: u64,
    pub filament_grams: f64,
}

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
    Ok(SliceEstimate {
        object_count,
        model_seconds,
        slicer_total_seconds,
        filament_grams,
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
    Ok(SliceEstimate {
        object_count,
        model_seconds: seconds,
        slicer_total_seconds: seconds,
        filament_grams,
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

pub fn subset_3mf(
    source: &Path,
    output: &Path,
    keep_count: usize,
    blank_start_gcode: bool,
) -> SchedulerResult<Vec<String>> {
    if keep_count == 0 {
        return Err("keep_count must be positive".into());
    }
    let mut archive = ZipArchive::new(File::open(source)?)?;
    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        entries.push((entry.name().to_owned(), entry.is_dir(), data));
    }

    let core_index = entry_index(&entries, "3D/3dmodel.model")?;
    let mut core = Element::parse(entries[core_index].2.as_slice())?;
    let build = child_mut(&mut core, "build")?;
    let all_ids: Vec<String> = element_children(build, "item")
        .map(|item| required_attr(item, "objectid"))
        .collect::<Result<_, _>>()?;
    if keep_count > all_ids.len() {
        return Err(format!(
            "requested {keep_count} objects but archive contains {}",
            all_ids.len()
        )
        .into());
    }
    let kept_ids: Vec<String> = all_ids.into_iter().take(keep_count).collect();
    let kept: HashSet<&str> = kept_ids.iter().map(String::as_str).collect();
    retain_elements(build, "item", |item| {
        item.attributes
            .get("objectid")
            .is_some_and(|id| kept.contains(id.as_str()))
    });
    let resources = child_mut(&mut core, "resources")?;
    retain_elements(resources, "object", |object| {
        object
            .attributes
            .get("id")
            .is_some_and(|id| kept.contains(id.as_str()))
    });
    entries[core_index].2 = emit_core_xml(&core)?;

    let model_index = entry_index(&entries, "Metadata/model_settings.config")?;
    let mut model = Element::parse(entries[model_index].2.as_slice())?;
    retain_elements(&mut model, "object", |object| {
        object
            .attributes
            .get("id")
            .is_some_and(|id| kept.contains(id.as_str()))
    });
    for plate in element_children_mut(&mut model, "plate") {
        retain_elements(plate, "model_instance", |instance| {
            metadata_value(instance, "object_id").is_some_and(|id| kept.contains(id))
        });
    }
    for assemble in element_children_mut(&mut model, "assemble") {
        retain_elements(assemble, "assemble_item", |item| {
            item.attributes
                .get("object_id")
                .is_some_and(|id| kept.contains(id.as_str()))
        });
    }
    entries[model_index].2 = emit_xml(&model)?;

    if let Ok(cut_index) = entry_index(&entries, "Metadata/cut_information.xml") {
        let mut cut = Element::parse(entries[cut_index].2.as_slice())?;
        let mut retained = 0usize;
        cut.children.retain(|node| {
            if node
                .as_element()
                .is_some_and(|element| element.name == "object")
            {
                retained += 1;
                retained <= keep_count
            } else {
                true
            }
        });
        entries[cut_index].2 = emit_xml(&cut)?;
    }

    if let Ok(plate_index) = entry_index(&entries, "Metadata/plate_1.json")
        && let Ok(mut plate) = serde_json::from_slice::<serde_json::Value>(&entries[plate_index].2)
    {
        if let Some(objects) = plate
            .get_mut("bbox_objects")
            .and_then(serde_json::Value::as_array_mut)
        {
            objects.truncate(keep_count);
            let boxes: Vec<[f64; 4]> = objects
                .iter()
                .filter_map(|object| object.get("bbox")?.as_array())
                .filter_map(|bbox| {
                    Some([
                        bbox.first()?.as_f64()?,
                        bbox.get(1)?.as_f64()?,
                        bbox.get(2)?.as_f64()?,
                        bbox.get(3)?.as_f64()?,
                    ])
                })
                .collect();
            if !boxes.is_empty() {
                plate["bbox_all"] = serde_json::json!([
                    boxes.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min),
                    boxes.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min),
                    boxes.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max),
                    boxes.iter().map(|b| b[3]).fold(f64::NEG_INFINITY, f64::max)
                ]);
            }
        }
        entries[plate_index].2 = serde_json::to_vec(&plate)?;
    }

    if blank_start_gcode {
        let settings_index = entry_index(&entries, "Metadata/project_settings.config")?;
        let mut settings: serde_json::Value = serde_json::from_slice(&entries[settings_index].2)?;
        settings["machine_start_gcode"] = serde_json::Value::String(String::new());
        entries[settings_index].2 = serde_json::to_vec(&settings)?;
    }

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = ZipWriter::new(File::create(output)?);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, is_dir, data) in entries {
        if is_dir {
            writer.add_directory(name, options)?;
        } else {
            writer.start_file(name, options)?;
            writer.write_all(&data)?;
        }
    }
    writer.finish()?;
    Ok(kept_ids)
}

pub fn object_count_3mf(source: &Path) -> SchedulerResult<usize> {
    let mut archive = ZipArchive::new(File::open(source)?)?;
    let mut data = Vec::new();
    archive
        .by_name("3D/3dmodel.model")?
        .read_to_end(&mut data)?;
    let core = Element::parse(data.as_slice())?;
    let build = core
        .get_child("build")
        .ok_or("3MF core model has no build element")?;
    Ok(element_children(build, "item").count())
}

/// Counts `<plate>` blocks in `Metadata/model_settings.config`. `subset_3mf` and
/// `object_count_3mf` both treat every `<build><item>` in document order as belonging to one
/// densely-packed plate; a source with more than one `<plate>` breaks that assumption silently —
/// `subset_3mf` happily includes build items whose underlying object was actually assigned to a
/// later plate, and slicing plate 1 alone then drops them without any error, undercounting the
/// batch. Callers should reject anything but a single-plate source before benchmarking it.
pub fn plate_count_3mf(source: &Path) -> SchedulerResult<usize> {
    let mut archive = ZipArchive::new(File::open(source)?)?;
    let mut data = Vec::new();
    archive
        .by_name("Metadata/model_settings.config")?
        .read_to_end(&mut data)?;
    let model = Element::parse(data.as_slice())?;
    Ok(element_children(&model, "plate").count())
}

fn entry_index(entries: &[(String, bool, Vec<u8>)], name: &str) -> SchedulerResult<usize> {
    entries
        .iter()
        .position(|entry| entry.0 == name)
        .ok_or_else(|| format!("archive entry not found: {name}").into())
}

fn child_mut<'a>(element: &'a mut Element, name: &str) -> SchedulerResult<&'a mut Element> {
    element
        .children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .find(|child| child.name == name)
        .ok_or_else(|| format!("XML child not found: {name}").into())
}

fn element_children<'a>(element: &'a Element, name: &'a str) -> impl Iterator<Item = &'a Element> {
    element
        .children
        .iter()
        .filter_map(XMLNode::as_element)
        .filter(move |child| child.name == name)
}

fn element_children_mut<'a>(
    element: &'a mut Element,
    name: &'a str,
) -> impl Iterator<Item = &'a mut Element> {
    element
        .children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .filter(move |child| child.name == name)
}

fn retain_elements<F>(element: &mut Element, name: &str, mut keep: F)
where
    F: FnMut(&Element) -> bool,
{
    element.children.retain(|node| {
        node.as_element()
            .filter(|child| child.name == name)
            .is_none_or(&mut keep)
    });
}

fn required_attr(element: &Element, name: &str) -> SchedulerResult<String> {
    element
        .attributes
        .get(name)
        .cloned()
        .ok_or_else(|| format!("attribute {name} missing on {}", element.name).into())
}

fn metadata_value<'a>(element: &'a Element, key: &str) -> Option<&'a str> {
    element_children(element, "metadata")
        .find(|metadata| {
            metadata
                .attributes
                .get("key")
                .is_some_and(|value| value == key)
        })
        .and_then(|metadata| metadata.attributes.get("value"))
        .map(String::as_str)
}

fn emit_xml(element: &Element) -> SchedulerResult<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    element.write_with_config(
        &mut output,
        EmitterConfig::new()
            .perform_indent(true)
            .write_document_declaration(true),
    )?;
    Ok(output.into_inner())
}

fn emit_core_xml(element: &Element) -> SchedulerResult<Vec<u8>> {
    let xml = String::from_utf8(emit_xml(element)?)?
        .replace(" UUID=", " p:UUID=")
        .replace(" path=", " p:path=")
        .replace(" lang=", " xml:lang=");
    Ok(xml.into_bytes())
}

#[allow(dead_code)]
fn _entries_map(entries: &[(String, bool, Vec<u8>)]) -> HashMap<&str, usize> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.0.as_str(), index))
        .collect()
}

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs::File,
    io::{Cursor, Read, Write},
    path::Path,
};

use sha2::{Digest, Sha256};
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
    let mut entries = read_zip_entries(source)?;

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
        patch_plate_json(&mut plate, keep_count);
        entries[plate_index].2 = serde_json::to_vec(&plate)?;
    }

    if blank_start_gcode {
        let settings_index = entry_index(&entries, "Metadata/project_settings.config")?;
        let mut settings: serde_json::Value = serde_json::from_slice(&entries[settings_index].2)?;
        settings["machine_start_gcode"] = serde_json::Value::String(String::new());
        entries[settings_index].2 = serde_json::to_vec(&settings)?;
    }

    write_zip_entries(output, entries)?;
    Ok(kept_ids)
}

/// One physical print job to materialize as a `<plate>` in [`build_schedule_plates_3mf`]: a
/// human-readable label (shown as the plate name in BambuStudio's plate list) and how many of the
/// source's densely-packed objects that plate should contain.
#[derive(Clone, Debug)]
pub struct PlateSpec {
    pub label: String,
    pub object_count: usize,
}

/// Builds one multi-plate 3MF out of a single densely-packed source (e.g. the output of
/// `colored_3mf_packer`) by trimming its build items down to each requested count and grouping
/// each trim into its own `<plate>`. This mirrors `subset_3mf`'s "first N build items" trimming
/// (same convention: drop from the tail of the packer's placement order, not randomly, so a
/// smaller plate loses objects from one edge of the layout instead of leaving holes) but keeps
/// every requested count as a separate plate in one file instead of one file per count.
///
/// Every plate gets its own copies of the trimmed objects — fresh resource/build-item ids and
/// UUIDs, but the same shared external mesh component and the same packed transform (rotation +
/// position) as the source — rather than sharing build items across plates. BambuStudio's own
/// multi-plate projects never let one object instance belong to two plates at once, so this
/// mirrors that rather than risk an ownership conflict.
///
/// Known cosmetic limitation: every plate reuses the source's plate-1 preview PNGs (no per-plate
/// thumbnail is rendered), and `Metadata/plate_N.json` / `cut_information.xml` /
/// `filament_sequence.json` are extended by the same position-based truncation `subset_3mf`
/// already relies on. Verified end-to-end against the real BambuStudio CLI (2026-09-02, Goat 3
/// P1S 20-object source, plates 20/15/8): every `--slice N` reports Success with exactly N
/// objects in the plate's G-code header, and the plate grid offsets land where BambuStudio's own
/// PartPlate lookup expects them (see the grid comment below).
pub fn build_schedule_plates_3mf(
    source: &Path,
    output: &Path,
    plates: &[PlateSpec],
    bed_width_mm: f64,
    bed_depth_mm: f64,
    blank_start_gcode: bool,
) -> SchedulerResult<()> {
    if plates.is_empty() {
        return Err("at least one plate is required".into());
    }
    if plates.iter().any(|plate| plate.object_count == 0) {
        return Err("plate object_count must be positive".into());
    }

    let mut entries = read_zip_entries(source)?;

    let core_index = entry_index(&entries, "3D/3dmodel.model")?;
    let mut core = Element::parse(entries[core_index].2.as_slice())?;
    let model_index = entry_index(&entries, "Metadata/model_settings.config")?;
    let mut model = Element::parse(entries[model_index].2.as_slice())?;

    // Canonical packing order: the build item order in 3D/3dmodel.model, same convention
    // subset_3mf uses for "first N".
    let build = child_mut(&mut core, "build")?;
    let original_item_ids: Vec<String> = element_children(build, "item")
        .map(|item| required_attr(item, "objectid"))
        .collect::<Result<_, _>>()?;
    let max_count = original_item_ids.len();
    for plate in plates {
        if plate.object_count > max_count {
            return Err(format!(
                "plate '{}' requests {} objects but source has {max_count}",
                plate.label, plate.object_count
            )
            .into());
        }
    }
    let original_items: HashMap<String, Element> = element_children(build, "item")
        .map(|item| Ok((required_attr(item, "objectid")?, item.clone())))
        .collect::<SchedulerResult<_>>()?;
    let resources = child_mut(&mut core, "resources")?;
    let original_objects: HashMap<String, Element> = element_children(resources, "object")
        .map(|object| Ok((required_attr(object, "id")?, object.clone())))
        .collect::<SchedulerResult<_>>()?;

    let config_objects: HashMap<String, Element> = element_children(&model, "object")
        .map(|object| Ok((required_attr(object, "id")?, object.clone())))
        .collect::<SchedulerResult<_>>()?;
    let plate_template = element_children(&model, "plate")
        .next()
        .cloned()
        .ok_or("model_settings.config has no <plate> to use as a template")?;
    let assemble_template = element_children(&model, "assemble")
        .next()
        .cloned()
        .ok_or("model_settings.config has no <assemble> block")?;
    let assemble_items: HashMap<String, Element> =
        element_children(&assemble_template, "assemble_item")
            .map(|item| Ok((required_attr(item, "object_id")?, item.clone())))
            .collect::<SchedulerResult<_>>()?;

    let mut next_id = original_item_ids
        .iter()
        .chain(config_objects.keys())
        .filter_map(|id| id.parse::<u64>().ok())
        .max()
        .map(|max| max + 1)
        .ok_or("source object ids are not numeric")?;

    // BambuStudio assigns an object to a plate purely by testing its bounding box against each
    // internal PartPlate's bed-sized region in one shared coordinate space (PartPlate::
    // intersect_instance) — it never consults <plate>/<model_instance> for this at slice time. A
    // plate whose objects sit at the source's original (plate-1) coordinates therefore slices as
    // "nothing to be sliced" for every plate but the first, however correct the XML bookkeeping
    // is. BambuStudio lays plates out in a grid with `cols = round(sqrt(plate_count))` and each
    // slot offset by `bed_size * 1.2` (`LOGICAL_PART_PLATE_GAP = 1/5` in PartPlate.cpp); replicate
    // that exactly so every plate's objects land in the slot BambuStudio itself will look for them
    // in.
    let cols = (plates.len() as f64).sqrt().round().max(1.0) as usize;
    let stride_x = bed_width_mm * 1.2;
    let stride_y = bed_depth_mm * 1.2;

    let mut new_items = Vec::new();
    let mut new_objects = Vec::new();
    let mut new_config_objects = Vec::new();
    let mut new_plates = Vec::new();
    let mut new_assemble_items = Vec::new();
    let mut identify_id = 1000u64;

    for (plate_index, plate) in plates.iter().enumerate() {
        let mut plate_element = plate_template.clone();
        set_metadata_value(&mut plate_element, "plater_id", &(plate_index + 1).to_string());
        set_metadata_value(&mut plate_element, "plater_name", &plate.label);
        plate_element.children.retain(|node| {
            node.as_element()
                .is_none_or(|element| element.name != "model_instance")
        });

        for original_id in &original_item_ids[..plate.object_count] {
            let new_id = next_id.to_string();
            next_id += 1;

            let mut item = original_items
                .get(original_id)
                .cloned()
                .ok_or_else(|| format!("build item {original_id} missing"))?;
            item.attributes
                .insert("objectid".to_owned(), new_id.clone());
            item.attributes.insert(
                "UUID".to_owned(),
                stable_uuid(&format!("build-item-{new_id}")),
            );
            let row = plate_index / cols;
            let col = plate_index % cols;
            if row > 0 || col > 0 {
                let transform = item
                    .attributes
                    .get("transform")
                    .ok_or("build item missing transform")?;
                let offset = offset_transform_xy(
                    transform,
                    col as f64 * stride_x,
                    -(row as f64) * stride_y,
                )?;
                item.attributes.insert("transform".to_owned(), offset);
            }
            new_items.push(item);

            let mut object = original_objects
                .get(original_id)
                .cloned()
                .ok_or_else(|| format!("resource object {original_id} missing"))?;
            object.attributes.insert("id".to_owned(), new_id.clone());
            object.attributes.insert(
                "UUID".to_owned(),
                stable_uuid(&format!("resource-{new_id}")),
            );
            for components in element_children_mut(&mut object, "components") {
                for component in element_children_mut(components, "component") {
                    component.attributes.insert(
                        "UUID".to_owned(),
                        stable_uuid(&format!("component-{new_id}")),
                    );
                }
            }
            new_objects.push(object);

            let mut config_object = config_objects
                .get(original_id)
                .cloned()
                .ok_or_else(|| format!("model_settings object {original_id} missing"))?;
            config_object
                .attributes
                .insert("id".to_owned(), new_id.clone());
            for part in element_children_mut(&mut config_object, "part") {
                part.attributes
                    .insert("uuid".to_owned(), stable_uuid(&format!("part-{new_id}")));
            }
            new_config_objects.push(config_object);

            let mut model_instance = Element::new("model_instance");
            push_metadata(&mut model_instance, "object_id", &new_id);
            push_metadata(&mut model_instance, "instance_id", "0");
            push_metadata(&mut model_instance, "identify_id", &identify_id.to_string());
            identify_id += 10;
            plate_element
                .children
                .push(XMLNode::Element(model_instance));

            let mut assemble_item = assemble_items
                .get(original_id)
                .cloned()
                .ok_or_else(|| format!("assemble_item {original_id} missing"))?;
            assemble_item
                .attributes
                .insert("object_id".to_owned(), new_id.clone());
            new_assemble_items.push(assemble_item);
        }

        new_plates.push(plate_element);
    }

    {
        let build = child_mut(&mut core, "build")?;
        build.children = new_items.into_iter().map(XMLNode::Element).collect();
        let resources = child_mut(&mut core, "resources")?;
        resources.children = new_objects.into_iter().map(XMLNode::Element).collect();
    }
    entries[core_index].2 = emit_core_xml(&core)?;

    {
        let mut assemble_element = assemble_template;
        assemble_element.children = new_assemble_items
            .into_iter()
            .map(XMLNode::Element)
            .collect();
        let mut children: Vec<XMLNode> = new_config_objects
            .into_iter()
            .map(XMLNode::Element)
            .collect();
        children.extend(new_plates.into_iter().map(XMLNode::Element));
        children.push(XMLNode::Element(assemble_element));
        model.children = children;
    }
    entries[model_index].2 = emit_xml(&model)?;

    if let Ok(cut_index) = entry_index(&entries, "Metadata/cut_information.xml") {
        let cut = Element::parse(entries[cut_index].2.as_slice())?;
        let original_cut_objects: Vec<Element> =
            element_children(&cut, "object").cloned().collect();
        let mut new_cut = Element::new("objects");
        // Every <object id="..."> here must be unique across the whole file — BambuStudio
        // rejects a project with "duplicated cut_object_id" if two plates reuse the same id, so
        // this can't just replay the source's per-plate positional ids like the rest of this
        // function does.
        let mut next_cut_id = 1u64;
        for plate in plates {
            for object in original_cut_objects.iter().take(plate.object_count) {
                let mut object = object.clone();
                object
                    .attributes
                    .insert("id".to_owned(), next_cut_id.to_string());
                next_cut_id += 1;
                new_cut.children.push(XMLNode::Element(object));
            }
        }
        entries[cut_index].2 = emit_xml(&new_cut)?;
    }

    if let Ok(plate_json_index) = entry_index(&entries, "Metadata/plate_1.json") {
        let template: serde_json::Value = serde_json::from_slice(&entries[plate_json_index].2)?;
        let mut renamed = Vec::with_capacity(plates.len());
        for (plate_index, plate) in plates.iter().enumerate() {
            let mut value = template.clone();
            patch_plate_json(&mut value, plate.object_count);
            renamed.push((
                format!("Metadata/plate_{}.json", plate_index + 1),
                false,
                serde_json::to_vec(&value)?,
            ));
        }
        entries.remove(plate_json_index);
        entries.extend(renamed);
    }

    if let Ok(sequence_index) = entry_index(&entries, "Metadata/filament_sequence.json") {
        let existing: serde_json::Value = serde_json::from_slice(&entries[sequence_index].2)?;
        let default_entry = existing.get("plate_1").cloned().unwrap_or_else(|| {
            serde_json::json!({"nozzle_sequence": [], "optimal_assignment": [], "sequence": []})
        });
        let mut map = serde_json::Map::new();
        for plate_index in 0..plates.len() {
            map.insert(format!("plate_{}", plate_index + 1), default_entry.clone());
        }
        entries[sequence_index].2 = serde_json::to_vec(&serde_json::Value::Object(map))?;
    }

    if blank_start_gcode {
        let settings_index = entry_index(&entries, "Metadata/project_settings.config")?;
        let mut settings: serde_json::Value = serde_json::from_slice(&entries[settings_index].2)?;
        settings["machine_start_gcode"] = serde_json::Value::String(String::new());
        entries[settings_index].2 = serde_json::to_vec(&settings)?;
    }

    write_zip_entries(output, entries)
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

fn read_zip_entries(path: &Path) -> SchedulerResult<Vec<(String, bool, Vec<u8>)>> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        entries.push((entry.name().to_owned(), entry.is_dir(), data));
    }
    Ok(entries)
}

fn write_zip_entries(path: &Path, entries: Vec<(String, bool, Vec<u8>)>) -> SchedulerResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = ZipWriter::new(File::create(path)?);
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
    Ok(())
}

/// Truncates a `Metadata/plate_N.json`-shaped value's `bbox_objects` to `keep_count` entries (the
/// same position-based truncation `subset_3mf` applies to the 3MF XML) and recomputes `bbox_all`
/// from what remains.
fn patch_plate_json(value: &mut serde_json::Value, keep_count: usize) {
    let Some(objects) = value
        .get_mut("bbox_objects")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
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
        value["bbox_all"] = serde_json::json!([
            boxes.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min),
            boxes.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min),
            boxes.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max),
            boxes.iter().map(|b| b[3]).fold(f64::NEG_INFINITY, f64::max)
        ]);
    }
}

/// Deterministic pseudo-UUID (not RFC 4122 version/variant compliant, but the same opaque
/// 8-4-4-4-12 hex shape BambuStudio's own `p:UUID`/`uuid` attributes use) derived from `seed`, so
/// re-running the same schedule produces byte-identical plate XML.
fn stable_uuid(seed: &str) -> String {
    let hash = Sha256::digest(seed.as_bytes());
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        hash[0],
        hash[1],
        hash[2],
        hash[3],
        hash[4],
        hash[5],
        hash[6],
        hash[7],
        hash[8],
        hash[9],
        hash[10],
        hash[11],
        hash[12],
        hash[13],
        hash[14],
        hash[15],
    )
}

/// Adds `dx`/`dy` to a 3MF build item's `transform` attribute (12 space-separated floats: a 3x3
/// rotation/scale matrix followed by the x/y/z translation) in place, preserving rotation.
fn offset_transform_xy(transform: &str, dx: f64, dy: f64) -> SchedulerResult<String> {
    let mut values: Vec<f64> = transform
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| "malformed transform attribute")?;
    if values.len() != 12 {
        return Err("transform attribute must have 12 values".into());
    }
    values[9] += dx;
    values[10] += dy;
    Ok(values
        .iter()
        .map(f64::to_string)
        .collect::<Vec<_>>()
        .join(" "))
}

fn set_metadata_value(element: &mut Element, key: &str, value: &str) {
    for metadata in element_children_mut(element, "metadata") {
        if metadata.attributes.get("key").map(String::as_str) == Some(key) {
            metadata.attributes.insert("value".to_owned(), value.to_owned());
        }
    }
}

fn push_metadata(element: &mut Element, key: &str, value: &str) {
    let mut metadata = Element::new("metadata");
    metadata.attributes.insert("key".to_owned(), key.to_owned());
    metadata
        .attributes
        .insert("value".to_owned(), value.to_owned());
    element.children.push(XMLNode::Element(metadata));
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

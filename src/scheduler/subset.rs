//! Cutting a packed 3MF down to some of its objects, and counting what it holds.

use std::{collections::HashSet, path::Path};

use xmltree::Element;

use super::SchedulerResult;
use super::archive::{Entries, entry_index, read_entry, read_zip_entries, write_zip_entries};
use super::xml_util::{
    attr_is_kept, child_mut, element_children, element_children_mut, emit_core_xml, emit_xml,
    metadata_value, required_attr, retain_elements,
};

pub fn subset_3mf(
    source: &Path,
    output: &Path,
    keep_count: usize,
    blank_start_gcode: bool,
) -> SchedulerResult<Vec<String>> {
    if keep_count == 0 {
        return Err("keep_count must be positive".into());
    }
    let keep: Vec<usize> = (0..keep_count).collect();
    subset_3mf_items(source, output, &keep, blank_start_gcode)
}

/// `subset_3mf` keeping the build items at the given positions (any order, no duplicates) —
/// the packer's sample slice cuts a few neighbouring objects out of the middle of a plate.
pub fn subset_3mf_items(
    source: &Path,
    output: &Path,
    keep_items: &[usize],
    blank_start_gcode: bool,
) -> SchedulerResult<Vec<String>> {
    if keep_items.is_empty() {
        return Err("keep_count must be positive".into());
    }
    let mut entries = read_zip_entries(source)?;

    let core_index = entry_index(&entries, "3D/3dmodel.model")?;
    let mut core = Element::parse(entries[core_index].2.as_slice())?;
    let kept_ids = keep_build_items(&mut core, keep_items)?;
    entries[core_index].2 = emit_core_xml(&core)?;
    let kept: HashSet<&str> = kept_ids.iter().map(String::as_str).collect();

    trim_model_settings(&mut entries, &kept)?;
    trim_cut_information(&mut entries, keep_items)?;
    trim_plate_json(&mut entries, keep_items.len())?;
    if blank_start_gcode {
        let settings_index = entry_index(&entries, "Metadata/project_settings.config")?;
        let mut settings: serde_json::Value = serde_json::from_slice(&entries[settings_index].2)?;
        blank_machine_start_gcode(&mut settings);
        entries[settings_index].2 = serde_json::to_vec(&settings)?;
    }

    write_zip_entries(output, entries)?;
    Ok(kept_ids)
}

/// The CLI workaround of `colored_packer::apply_cli_settings_workarounds`: no machine start
/// G-code, so the slice estimate covers the print itself.
pub(super) fn blank_machine_start_gcode(settings: &mut serde_json::Value) {
    settings["machine_start_gcode"] = serde_json::Value::String(String::new());
}

/// Drops every build item (and its object resource) but the ones at `keep_items`; returns the
/// object ids that stay, in build order.
fn keep_build_items(core: &mut Element, keep_items: &[usize]) -> SchedulerResult<Vec<String>> {
    let keep_count = keep_items.len();
    let build = child_mut(core, "build")?;
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
    if let Some(&missing) = keep_items.iter().find(|&&index| index >= all_ids.len()) {
        return Err(format!(
            "build item {missing} does not exist ({} items)",
            all_ids.len()
        )
        .into());
    }
    let kept_ids: Vec<String> = all_ids
        .into_iter()
        .enumerate()
        .filter(|(index, _)| keep_items.contains(index))
        .map(|(_, id)| id)
        .collect();
    let kept: HashSet<&str> = kept_ids.iter().map(String::as_str).collect();
    retain_elements(build, "item", |item| attr_is_kept(item, "objectid", &kept));
    let resources = child_mut(core, "resources")?;
    retain_elements(resources, "object", |object| {
        attr_is_kept(object, "id", &kept)
    });
    Ok(kept_ids)
}

/// Keeps the objects, plate instances and assemble items of the kept ids.
fn trim_model_settings(entries: &mut Entries, kept: &HashSet<&str>) -> SchedulerResult<()> {
    let model_index = entry_index(entries, "Metadata/model_settings.config")?;
    let mut model = Element::parse(entries[model_index].2.as_slice())?;
    retain_elements(&mut model, "object", |object| {
        attr_is_kept(object, "id", kept)
    });
    for plate in element_children_mut(&mut model, "plate") {
        retain_elements(plate, "model_instance", |instance| {
            metadata_value(instance, "object_id").is_some_and(|id| kept.contains(id))
        });
    }
    for assemble in element_children_mut(&mut model, "assemble") {
        retain_elements(assemble, "assemble_item", |item| {
            attr_is_kept(item, "object_id", kept)
        });
    }
    entries[model_index].2 = emit_xml(&model)?;
    Ok(())
}

/// Cut information is positional: the n-th `<object>` belongs to the n-th build item.
fn trim_cut_information(entries: &mut Entries, keep_items: &[usize]) -> SchedulerResult<()> {
    let Ok(cut_index) = entry_index(entries, "Metadata/cut_information.xml") else {
        return Ok(());
    };
    let mut cut = Element::parse(entries[cut_index].2.as_slice())?;
    let mut position = 0usize;
    cut.children.retain(|node| {
        if node
            .as_element()
            .is_some_and(|element| element.name == "object")
        {
            position += 1;
            keep_items.contains(&(position - 1))
        } else {
            true
        }
    });
    entries[cut_index].2 = emit_xml(&cut)?;
    Ok(())
}

fn trim_plate_json(entries: &mut Entries, keep_count: usize) -> SchedulerResult<()> {
    if let Ok(plate_index) = entry_index(entries, "Metadata/plate_1.json")
        && let Ok(mut plate) = serde_json::from_slice::<serde_json::Value>(&entries[plate_index].2)
    {
        patch_plate_json(&mut plate, keep_count);
        entries[plate_index].2 = serde_json::to_vec(&plate)?;
    }
    Ok(())
}

/// Truncates a `Metadata/plate_N.json`-shaped value's `bbox_objects` to `keep_count` entries (the
/// same position-based truncation `subset_3mf` applies to the 3MF XML) and recomputes `bbox_all`
/// from what remains.
pub(super) fn patch_plate_json(value: &mut serde_json::Value, keep_count: usize) {
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

pub fn object_count_3mf(source: &Path) -> SchedulerResult<usize> {
    let data = read_entry(source, "3D/3dmodel.model")?;
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
    let data = read_entry(source, "Metadata/model_settings.config")?;
    let model = Element::parse(data.as_slice())?;
    Ok(element_children(&model, "plate").count())
}

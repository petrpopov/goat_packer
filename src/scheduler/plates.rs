//! One multi-plate 3MF out of a single densely-packed source.

use std::{collections::HashMap, path::Path};

use xmltree::{Element, XMLNode};

use super::SchedulerResult;
use super::archive::{Entries, entry_index, read_zip_entries, write_zip_entries};
use super::subset::{blank_machine_start_gcode, patch_plate_json};
use super::xml_util::{
    child_mut, element_children, element_children_mut, emit_core_xml, emit_xml, push_metadata,
    required_attr, set_metadata_value, stable_uuid,
};

/// BambuStudio lays plates out in a grid whose slots are the bed size plus a fifth
/// (`LOGICAL_PART_PLATE_GAP = 1/5` in PartPlate.cpp); measured stride on a 256 mm bed: 307.2.
const PLATE_STRIDE_FACTOR: f64 = 1.2;

/// `identify_id` of the first plate instance; later instances step by `IDENTIFY_ID_STEP`
/// (the numbering BambuStudio itself writes).
const FIRST_IDENTIFY_ID: u64 = 1000;
const IDENTIFY_ID_STEP: u64 = 10;

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
/// PartPlate lookup expects them (see `PlateGrid`).
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

    let source_project = SourceProject::read(&mut core, &model, plates)?;
    let mut next_id = source_project.first_free_id()?;
    let grid = PlateGrid::new(plates.len(), bed_width_mm, bed_depth_mm);

    let mut built = BuiltPlates::default();
    let mut identify_id = FIRST_IDENTIFY_ID;
    for (plate_index, plate) in plates.iter().enumerate() {
        let mut plate_element = source_project.plate_template.clone();
        set_metadata_value(
            &mut plate_element,
            "plater_id",
            &(plate_index + 1).to_string(),
        );
        set_metadata_value(&mut plate_element, "plater_name", &plate.label);
        plate_element.children.retain(|node| {
            node.as_element()
                .is_none_or(|element| element.name != "model_instance")
        });
        for original_id in &source_project.item_ids[..plate.object_count] {
            let new_id = next_id.to_string();
            next_id += 1;
            let cloned =
                source_project.clone_object(original_id, &new_id, grid.offset(plate_index))?;
            built.items.push(cloned.item);
            built.objects.push(cloned.object);
            built.config_objects.push(cloned.config_object);
            built.assemble_items.push(cloned.assemble_item);

            let mut model_instance = Element::new("model_instance");
            push_metadata(&mut model_instance, "object_id", &new_id);
            push_metadata(&mut model_instance, "instance_id", "0");
            push_metadata(&mut model_instance, "identify_id", &identify_id.to_string());
            identify_id += IDENTIFY_ID_STEP;
            plate_element
                .children
                .push(XMLNode::Element(model_instance));
        }
        built.plates.push(plate_element);
    }

    write_core_model(&mut core, &mut entries[core_index].2, &mut built)?;
    write_model_settings(
        &mut model,
        source_project.assemble_template,
        &mut entries[model_index].2,
        built,
    )?;
    rewrite_cut_information(&mut entries, plates)?;
    rewrite_plate_jsons(&mut entries, plates)?;
    rewrite_filament_sequence(&mut entries, plates.len())?;
    // One pass over project_settings.config: the optional start-gcode blanking (CLI workaround,
    // same as subset_3mf) and the mandatory per-plate wipe-tower entries.
    let settings_index = entry_index(&entries, "Metadata/project_settings.config")?;
    let mut settings: serde_json::Value = serde_json::from_slice(&entries[settings_index].2)?;
    patch_wipe_tower_per_plate(&mut settings, plates.len());
    if blank_start_gcode {
        blank_machine_start_gcode(&mut settings);
    }
    entries[settings_index].2 = serde_json::to_vec(&settings)?;

    write_zip_entries(output, entries)
}

/// The elements of the source project that every plate's clones are made from.
struct SourceProject {
    /// Canonical packing order: the build item order in 3D/3dmodel.model, same convention
    /// subset_3mf uses for "first N".
    item_ids: Vec<String>,
    items: HashMap<String, Element>,
    objects: HashMap<String, Element>,
    config_objects: HashMap<String, Element>,
    plate_template: Element,
    assemble_template: Element,
    assemble_items: HashMap<String, Element>,
}

/// An object cloned for one plate.
struct ClonedObject {
    item: Element,
    object: Element,
    config_object: Element,
    assemble_item: Element,
}

/// Everything the plates add, in plate order.
#[derive(Default)]
struct BuiltPlates {
    items: Vec<Element>,
    objects: Vec<Element>,
    config_objects: Vec<Element>,
    plates: Vec<Element>,
    assemble_items: Vec<Element>,
}

fn index_by_attr(
    parent: &Element,
    child: &str,
    attr: &str,
) -> SchedulerResult<HashMap<String, Element>> {
    element_children(parent, child)
        .map(|element| Ok((required_attr(element, attr)?, element.clone())))
        .collect()
}

/// No plate asks for more objects than the source has.
fn check_capacity(item_ids: &[String], plates: &[PlateSpec]) -> SchedulerResult<()> {
    let max_count = item_ids.len();
    for plate in plates {
        if plate.object_count > max_count {
            return Err(format!(
                "plate '{}' requests {} objects but source has {max_count}",
                plate.label, plate.object_count
            )
            .into());
        }
    }
    Ok(())
}

impl SourceProject {
    fn read(core: &mut Element, model: &Element, plates: &[PlateSpec]) -> SchedulerResult<Self> {
        let build = child_mut(core, "build")?;
        let item_ids: Vec<String> = element_children(build, "item")
            .map(|item| required_attr(item, "objectid"))
            .collect::<Result<_, _>>()?;
        check_capacity(&item_ids, plates)?;
        let items = index_by_attr(build, "item", "objectid")?;
        let resources = child_mut(core, "resources")?;
        let objects = index_by_attr(resources, "object", "id")?;
        let config_objects = index_by_attr(model, "object", "id")?;
        let plate_template = element_children(model, "plate")
            .next()
            .cloned()
            .ok_or("model_settings.config has no <plate> to use as a template")?;
        let assemble_template = element_children(model, "assemble")
            .next()
            .cloned()
            .ok_or("model_settings.config has no <assemble> block")?;
        let assemble_items = index_by_attr(&assemble_template, "assemble_item", "object_id")?;
        Ok(Self {
            item_ids,
            items,
            objects,
            config_objects,
            plate_template,
            assemble_template,
            assemble_items,
        })
    }

    /// The first object id no resource or settings object uses yet.
    fn first_free_id(&self) -> SchedulerResult<u64> {
        self.item_ids
            .iter()
            .chain(self.config_objects.keys())
            .filter_map(|id| id.parse::<u64>().ok())
            .max()
            .map(|max| max + 1)
            .ok_or_else(|| "source object ids are not numeric".into())
    }

    /// Copies of one source object under `new_id`, placed on the plate whose grid slot is
    /// shifted by `shift` from the first plate's (`None` for the first plate itself).
    fn clone_object(
        &self,
        original_id: &str,
        new_id: &str,
        shift: Option<(f64, f64)>,
    ) -> SchedulerResult<ClonedObject> {
        let mut item = self
            .items
            .get(original_id)
            .cloned()
            .ok_or_else(|| format!("build item {original_id} missing"))?;
        item.attributes
            .insert("objectid".to_owned(), new_id.to_owned());
        item.attributes.insert(
            "UUID".to_owned(),
            stable_uuid(&format!("build-item-{new_id}")),
        );
        if let Some((dx, dy)) = shift {
            let transform = item
                .attributes
                .get("transform")
                .ok_or("build item missing transform")?;
            let offset = offset_transform_xy(transform, dx, dy)?;
            item.attributes.insert("transform".to_owned(), offset);
        }

        let mut object = self
            .objects
            .get(original_id)
            .cloned()
            .ok_or_else(|| format!("resource object {original_id} missing"))?;
        object.attributes.insert("id".to_owned(), new_id.to_owned());
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

        let mut config_object = self
            .config_objects
            .get(original_id)
            .cloned()
            .ok_or_else(|| format!("model_settings object {original_id} missing"))?;
        config_object
            .attributes
            .insert("id".to_owned(), new_id.to_owned());
        for part in element_children_mut(&mut config_object, "part") {
            part.attributes
                .insert("uuid".to_owned(), stable_uuid(&format!("part-{new_id}")));
        }

        let mut assemble_item = self
            .assemble_items
            .get(original_id)
            .cloned()
            .ok_or_else(|| format!("assemble_item {original_id} missing"))?;
        assemble_item
            .attributes
            .insert("object_id".to_owned(), new_id.to_owned());
        Ok(ClonedObject {
            item,
            object,
            config_object,
            assemble_item,
        })
    }
}

/// Where each plate sits in BambuStudio's plate grid.
///
/// BambuStudio assigns an object to a plate purely by testing its bounding box against each
/// internal PartPlate's bed-sized region in one shared coordinate space (PartPlate::
/// intersect_instance) — it never consults <plate>/<model_instance> for this at slice time. A
/// plate whose objects sit at the source's original (plate-1) coordinates therefore slices as
/// "nothing to be sliced" for every plate but the first, however correct the XML bookkeeping
/// is. BambuStudio lays plates out in a row-major grid, each slot offset by `bed_size * 1.2`
/// (`PLATE_STRIDE_FACTOR`), rows extending downward (negative Y).
///
/// The column count is `ceil(sqrt(plate_count))` — NOT round(). Measured 2026-09-06 from real
/// BambuStudio-saved multi-plate projects (256x256 bed, stride exactly 307.2): 2 plates sit
/// side by side (cols=2), 3 plates wrap to a 2nd row (cols=2), 11 and 12 plates use cols=4.
/// round(sqrt(2))=1 stacks plate 2 *below* plate 1 while BambuStudio looks for it to the
/// *right* — every plate-2 object then lands outside any PartPlate slot and the GUI shows them
/// "flown off the plate" (a real 2-plate schedule file hit exactly this). The earlier
/// end-to-end check (2026-09-02) passed only because round and ceil agree at 3 plates.
struct PlateGrid {
    cols: usize,
    stride_x: f64,
    stride_y: f64,
}

impl PlateGrid {
    fn new(plate_count: usize, bed_width_mm: f64, bed_depth_mm: f64) -> Self {
        Self {
            cols: ((plate_count as f64).sqrt().ceil() as usize).max(1),
            stride_x: bed_width_mm * PLATE_STRIDE_FACTOR,
            stride_y: bed_depth_mm * PLATE_STRIDE_FACTOR,
        }
    }

    /// The `(dx, dy)` shift of a plate's slot from the first plate's; `None` for the first.
    fn offset(&self, plate_index: usize) -> Option<(f64, f64)> {
        let row = plate_index / self.cols;
        let col = plate_index % self.cols;
        (row > 0 || col > 0).then(|| (col as f64 * self.stride_x, -(row as f64) * self.stride_y))
    }
}

/// Replaces the core model's build items and resources with the plates' clones.
fn write_core_model(
    core: &mut Element,
    entry: &mut Vec<u8>,
    built: &mut BuiltPlates,
) -> SchedulerResult<()> {
    let build = child_mut(core, "build")?;
    build.children = std::mem::take(&mut built.items)
        .into_iter()
        .map(XMLNode::Element)
        .collect();
    let resources = child_mut(core, "resources")?;
    resources.children = std::mem::take(&mut built.objects)
        .into_iter()
        .map(XMLNode::Element)
        .collect();
    *entry = emit_core_xml(core)?;
    Ok(())
}

/// Objects, then plates, then the assemble block with the clones' items.
fn write_model_settings(
    model: &mut Element,
    mut assemble_element: Element,
    entry: &mut Vec<u8>,
    built: BuiltPlates,
) -> SchedulerResult<()> {
    assemble_element.children = built
        .assemble_items
        .into_iter()
        .map(XMLNode::Element)
        .collect();
    let mut children: Vec<XMLNode> = built
        .config_objects
        .into_iter()
        .map(XMLNode::Element)
        .collect();
    children.extend(built.plates.into_iter().map(XMLNode::Element));
    children.push(XMLNode::Element(assemble_element));
    model.children = children;
    *entry = emit_xml(model)?;
    Ok(())
}

fn rewrite_cut_information(entries: &mut Entries, plates: &[PlateSpec]) -> SchedulerResult<()> {
    let Ok(cut_index) = entry_index(entries, "Metadata/cut_information.xml") else {
        return Ok(());
    };
    let cut = Element::parse(entries[cut_index].2.as_slice())?;
    let original_cut_objects: Vec<Element> = element_children(&cut, "object").cloned().collect();
    let mut new_cut = Element::new("objects");
    // Every <object id="..."> here must be unique across the whole file — BambuStudio rejects a
    // project with "duplicated cut_object_id" if two plates reuse the same id, so this can't
    // just replay the source's per-plate positional ids like the rest of this function does.
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
    Ok(())
}

/// `plate_1.json` becomes `plate_N.json` per plate, each truncated to its object count.
fn rewrite_plate_jsons(entries: &mut Entries, plates: &[PlateSpec]) -> SchedulerResult<()> {
    let Ok(plate_json_index) = entry_index(entries, "Metadata/plate_1.json") else {
        return Ok(());
    };
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
    Ok(())
}

fn rewrite_filament_sequence(entries: &mut Entries, plate_count: usize) -> SchedulerResult<()> {
    let Ok(sequence_index) = entry_index(entries, "Metadata/filament_sequence.json") else {
        return Ok(());
    };
    let existing: serde_json::Value = serde_json::from_slice(&entries[sequence_index].2)?;
    let default_entry = existing.get("plate_1").cloned().unwrap_or_else(
        || serde_json::json!({"nozzle_sequence": [], "optimal_assignment": [], "sequence": []}),
    );
    let mut map = serde_json::Map::new();
    for plate_index in 0..plate_count {
        map.insert(format!("plate_{}", plate_index + 1), default_entry.clone());
    }
    entries[sequence_index].2 = serde_json::to_vec(&serde_json::Value::Object(map))?;
    Ok(())
}

/// `wipe_tower_x`/`wipe_tower_y` in project_settings.config are parallel arrays holding ONE
/// tower position per plate (a real BambuStudio 3-plate AMS project carries three entries, and
/// its plate-2 G-code places the tower exactly at the second entry's coordinates). Two
/// measured semantics matter here:
///
/// 1. A plate without its own entry gets an auto-placed tower — for a packed plate that means
///    a tower dropped on top of objects, and BambuStudio refuses the slice with a false "gcode
///    path conflicts found between WipeTower and <mesh>" (reproduced 2026-09-06: A2L 30-object
///    2-plate schedule, plate 1 clean, plate 2 exit 155).
/// 2. Entries are bed coordinates used **verbatim** in that plate's G-code: the CLI shifts a
///    plate's objects back to plate-local coordinates when slicing it, but the tower entry is
///    not shifted with the plate's grid slot. Shifting the entry (like the build items) put the
///    tower at x=627 on a 330 mm bed and the slice died with "gcode outside of the printable
///    area" (same repro).
///
/// So every plate gets the SAME entry as the source: each plate's G-code then reproduces the
/// exact object/tower geometry the packer validated on plate 1. Absent keys (single-material
/// project, no tower) are left alone.
fn patch_wipe_tower_per_plate(settings: &mut serde_json::Value, plate_count: usize) {
    let first_entry = |key: &str| {
        settings
            .get(key)
            .and_then(serde_json::Value::as_array)
            .and_then(|values| values.first())
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let (Some(base_x), Some(base_y)) = (first_entry("wipe_tower_x"), first_entry("wipe_tower_y"))
    else {
        return;
    };
    let repeated = |base: String| {
        serde_json::Value::Array(
            std::iter::repeat_n(serde_json::Value::String(base), plate_count).collect(),
        )
    };
    settings["wipe_tower_x"] = repeated(base_x);
    settings["wipe_tower_y"] = repeated(base_y);
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

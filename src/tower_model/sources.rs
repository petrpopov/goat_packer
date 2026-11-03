//! Where the filaments of the source object come from, besides paint (spec 04 §2, last
//! paragraph): the object's and each part's `extruder` in `Metadata/model_settings.config`,
//! and the extruder (and layer height) overrides of `Metadata/layer_config_ranges.xml`.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::Path,
};

use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use zip::ZipArchive;

use crate::embedded_mesh::{ObjectMesh, Part, PartKind};

const CORE_MODEL_ENTRY: &str = "3D/3dmodel.model";
const MODEL_SETTINGS_ENTRY: &str = "Metadata/model_settings.config";
const LAYER_RANGES_ENTRY: &str = "Metadata/layer_config_ranges.xml";
/// Present when the user painted a variable layer height.
const LAYER_HEIGHT_PROFILE_ENTRY: &str = "Metadata/layer_heights_profile.txt";

/// A z band of the object with its own settings.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerRange {
    pub min_z: f64,
    pub max_z: f64,
    /// 1-based; `None` when the range does not set one.
    pub filament: Option<u32>,
    pub layer_height: Option<f64>,
}

/// The filament settings of the first build item's object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilamentSources {
    /// The object's `extruder` (1 when unset).
    pub object_filament: u32,
    /// Explicit `extruder` of a part, by the part's 3MF object id (`Part::source_id`).
    pub part_filaments: BTreeMap<u32, u32>,
    pub layer_ranges: Vec<LayerRange>,
    /// The project carries a painted layer-height profile.
    pub variable_layer_height: bool,
}

impl FilamentSources {
    /// The filament a part prints with where neither paint nor a layer range says otherwise.
    /// Modifiers without their own extruder change nothing (`None`).
    pub fn part_filament(&self, part: &Part) -> Option<u32> {
        let explicit = self.part_filaments.get(&part.source_id).copied();
        match part.kind {
            PartKind::Normal => Some(explicit.unwrap_or(self.object_filament)),
            PartKind::Modifier => explicit,
            _ => None,
        }
    }

    /// `part_filament` for every part in `mesh.parts` order, 0 where a part prints nothing.
    pub fn part_filaments_of(&self, mesh: &ObjectMesh) -> Vec<u32> {
        mesh.parts
            .iter()
            .map(|part| self.part_filament(part).unwrap_or(0))
            .collect()
    }

    /// The layer-range filament at height `z`, which replaces every model part's own one.
    /// A range owns `[min_z, max_z)`; the ±1-layer edge disagreement is the schedule's
    /// dilation's job (04 §4.1 step 4).
    pub fn range_filament_at(&self, z: f64) -> Option<u32> {
        self.layer_ranges
            .iter()
            .filter(|range| range.min_z <= z && z < range.max_z)
            .find_map(|range| range.filament)
    }
}

/// Reads the sources of the first build item's object of `source_3mf`. Missing config files
/// mean defaults (every part on filament 1, no ranges).
pub fn read_filament_sources(source_3mf: &Path) -> io::Result<FilamentSources> {
    let mut archive = ZipArchive::new(File::open(source_3mf)?)
        .map_err(|error| invalid(format!("{source_3mf:?} is not a 3MF archive: {error}")))?;
    let core = entry_text(&mut archive, CORE_MODEL_ENTRY)?
        .ok_or_else(|| invalid("3MF has no 3D/3dmodel.model"))?;
    let object_id = first_item_object(&core)?;
    let settings = entry_text(&mut archive, MODEL_SETTINGS_ENTRY)?;
    let (object_filament, part_filaments, object_index) = match &settings {
        Some(text) => object_extruders(text, object_id)?,
        None => (1, BTreeMap::new(), 1),
    };
    let layer_ranges = match entry_text(&mut archive, LAYER_RANGES_ENTRY)? {
        Some(text) => layer_ranges(&text, object_index)?,
        None => Vec::new(),
    };
    let variable_layer_height = archive.by_name(LAYER_HEIGHT_PROFILE_ENTRY).is_ok();
    Ok(FilamentSources {
        object_filament,
        part_filaments,
        layer_ranges,
        variable_layer_height,
    })
}

fn entry_text(archive: &mut ZipArchive<File>, name: &str) -> io::Result<Option<String>> {
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(invalid(format!("cannot read {name}: {error}"))),
    };
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// The `objectid` of the first `<item>` of the build.
fn first_item_object(core: &str) -> io::Result<u32> {
    let mut reader = Reader::from_reader(core.as_bytes());
    loop {
        match reader
            .read_event()
            .map_err(|e| invalid(format!("3dmodel.model: {e}")))?
        {
            Event::Start(start) | Event::Empty(start)
                if start.local_name().into_inner() == "item" =>
            {
                return number(&attribute(&start, "objectid")?.unwrap_or_default());
            }
            Event::Eof => return Err(invalid("3MF build has no item")),
            _ => {}
        }
    }
}

/// The object's extruder, its parts' explicit extruders and its 1-based position among the
/// config's objects (the index `layer_config_ranges.xml` names objects by).
fn object_extruders(config: &str, object_id: u32) -> io::Result<(u32, BTreeMap<u32, u32>, u32)> {
    let mut reader = Reader::from_reader(config.as_bytes());
    let (mut object_filament, mut parts) = (1, BTreeMap::new());
    let (mut index, mut found, mut in_object, mut part) = (0, 0, false, None);
    loop {
        match reader
            .read_event()
            .map_err(|e| invalid(format!("model_settings: {e}")))?
        {
            Event::Start(start) | Event::Empty(start) => match start.local_name().into_inner() {
                "object" => {
                    index += 1;
                    in_object = attribute(&start, "id")? == Some(object_id.to_string());
                    if in_object {
                        found = index;
                    }
                }
                "part" if in_object => {
                    part = Some(number(&attribute(&start, "id")?.unwrap_or_default())?)
                }
                "metadata"
                    if in_object && attribute(&start, "key")?.as_deref() == Some("extruder") =>
                {
                    let value = number(&attribute(&start, "value")?.unwrap_or_default())?;
                    match part {
                        // 0 means "the object's", the same as no setting.
                        Some(id) if value > 0 => {
                            parts.insert(id, value);
                        }
                        None if value > 0 => object_filament = value,
                        _ => {}
                    }
                }
                _ => {}
            },
            Event::End(end) => match end.local_name().into_inner() {
                "part" => part = None,
                "object" => in_object = false,
                _ => {}
            },
            Event::Eof => return Ok((object_filament, parts, found.max(1))),
            _ => {}
        }
    }
}

/// The `<range>` entries of object `index` (1-based).
fn layer_ranges(xml: &str, index: u32) -> io::Result<Vec<LayerRange>> {
    let mut reader = Reader::from_reader(xml.as_bytes());
    let (mut ranges, mut in_object, mut key) = (Vec::new(), false, None::<String>);
    loop {
        match reader
            .read_event()
            .map_err(|e| invalid(format!("layer_config_ranges: {e}")))?
        {
            Event::Start(start) | Event::Empty(start) => match start.local_name().into_inner() {
                "object" => in_object = attribute(&start, "id")? == Some(index.to_string()),
                "range" if in_object => ranges.push(LayerRange {
                    min_z: float(&attribute(&start, "min_z")?.unwrap_or_default())?,
                    max_z: float(&attribute(&start, "max_z")?.unwrap_or_default())?,
                    filament: None,
                    layer_height: None,
                }),
                "option" if in_object => key = attribute(&start, "opt_key")?,
                _ => {}
            },
            Event::Text(text) if in_object => {
                let value = text.as_ref().trim().to_owned();
                if let (Some(range), Some(name)) = (ranges.last_mut(), key.take()) {
                    match name.as_str() {
                        "extruder" => range.filament = Some(number(&value)?).filter(|f| *f > 0),
                        "layer_height" => range.layer_height = Some(float(&value)?),
                        _ => {}
                    }
                }
            }
            Event::End(end) if end.local_name().into_inner() == "object" => in_object = false,
            Event::Eof => return Ok(ranges),
            _ => {}
        }
    }
}

fn attribute(start: &BytesStart<'_>, key: &str) -> io::Result<Option<String>> {
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|e| invalid(format!("XML attribute: {e}")))?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.value.to_string()));
        }
    }
    Ok(None)
}

fn number(text: &str) -> io::Result<u32> {
    text.trim()
        .parse()
        .map_err(|_| invalid(format!("not a whole number: {text:?}")))
}

fn float(text: &str) -> io::Result<f64> {
    text.trim()
        .parse()
        .map_err(|_| invalid(format!("not a number: {text:?}")))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

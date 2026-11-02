//! The slicer configuration of the source project as the support model reads it:
//! `Metadata/project_settings.config` (JSON, string or string-array values), the first
//! object's overrides in `Metadata/model_settings.config`, its height ranges in
//! `Metadata/layer_config_ranges.xml`, a stored variable-layer profile in
//! `Metadata/layer_heights_profile.txt`, and the `Application` metadata of the core model.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use quick_xml::{Reader, XmlVersion, events::Event};
use serde_json::Value;
use zip::ZipArchive;

const PROJECT_SETTINGS: &str = "Metadata/project_settings.config";
const MODEL_SETTINGS: &str = "Metadata/model_settings.config";
const LAYER_RANGES: &str = "Metadata/layer_config_ranges.xml";
const LAYER_PROFILE: &str = "Metadata/layer_heights_profile.txt";
const CORE_MODEL: &str = "3D/3dmodel.model";

/// Object metadata in `model_settings.config` that names things rather than setting them.
const NOT_SETTINGS: [&str; 1] = ["name"];

/// One `<range>` of `layer_config_ranges.xml`: a height interval with its own options.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerRange {
    pub min_z: f64,
    pub max_z: f64,
    /// The range's `layer_height` option; `None` keeps the object's.
    pub layer_height: Option<f64>,
}

/// Every setting of one project, flattened to the first object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectConfig {
    values: BTreeMap<String, Value>,
    pub layer_ranges: Vec<LayerRange>,
    /// The stored variable layer-height profile `[z0, h0, z1, h1, ...]`, if any.
    pub layer_profile: Option<Vec<f64>>,
    /// `Application` metadata of the core model (flavour resolution, design §5.3).
    pub application: Option<String>,
}

impl ProjectConfig {
    /// A config from `project_settings.config` text alone.
    pub fn from_json(text: &str) -> Result<ProjectConfig, String> {
        let values: BTreeMap<String, Value> = serde_json::from_str(text)
            .map_err(|error| format!("{PROJECT_SETTINGS} is not a JSON object: {error}"))?;
        Ok(ProjectConfig {
            values,
            ..ProjectConfig::default()
        })
    }

    /// Reads every source of settings from a Bambu/Orca 3MF project.
    pub fn load(source_3mf: &Path) -> Result<ProjectConfig, String> {
        let file = File::open(source_3mf).map_err(|error| format!("{source_3mf:?}: {error}"))?;
        let mut archive = ZipArchive::new(file)
            .map_err(|error| format!("{source_3mf:?} is not a 3MF: {error}"))?;
        let text = entry(&mut archive, PROJECT_SETTINGS)?
            .ok_or_else(|| format!("{source_3mf:?} has no {PROJECT_SETTINGS}"))?;
        let mut config = ProjectConfig::from_json(&text)?;
        let core = entry(&mut archive, CORE_MODEL)?.unwrap_or_default();
        config.application = application(&core)?;
        let object_id = first_item_object(&core)?;
        if let (Some(model), Some(id)) = (entry(&mut archive, MODEL_SETTINGS)?, object_id) {
            for (key, value) in object_overrides(&model, &id)? {
                config.set(&key, &value);
            }
        }
        if let Some(ranges) = entry(&mut archive, LAYER_RANGES)? {
            config.layer_ranges = layer_ranges(&ranges)?;
        }
        if let Some(profile) = entry(&mut archive, LAYER_PROFILE)? {
            config.layer_profile = layer_profile(&profile);
        }
        Ok(config)
    }

    /// Overrides one setting with a plain string value.
    pub fn set(&mut self, key: &str, value: &str) {
        self.values
            .insert(key.to_owned(), Value::String(value.to_owned()));
    }

    pub fn has(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// The text of a setting; for per-extruder arrays, the first entry.
    pub fn text(&self, key: &str) -> Option<String> {
        match self.values.get(key)? {
            Value::String(text) => Some(text.clone()),
            Value::Array(items) => items.first().and_then(Value::as_str).map(str::to_owned),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(if *flag { "1" } else { "0" }.to_owned()),
            _ => None,
        }
    }

    /// Every entry of an array setting (a scalar gives one entry).
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.values.get(key) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            Some(_) => self.text(key).into_iter().collect(),
            None => Vec::new(),
        }
    }

    /// A number; a trailing `%` is stripped (the caller decides what the percentage means).
    pub fn number(&self, key: &str) -> Result<Option<f64>, String> {
        self.text(key)
            .map(|text| parse_number(key, &text))
            .transpose()
    }

    pub fn number_or(&self, key: &str, default: f64) -> Result<f64, String> {
        Ok(self.number(key)?.unwrap_or(default))
    }

    /// Whether a value is written as a percentage.
    pub fn is_percent(&self, key: &str) -> bool {
        self.text(key)
            .is_some_and(|text| text.trim_end().ends_with('%'))
    }

    /// A boolean written as `0`/`1` or `false`/`true`.
    pub fn flag_or(&self, key: &str, default: bool) -> Result<bool, String> {
        match self.text(key).as_deref().map(str::trim) {
            None => Ok(default),
            Some("1" | "true") => Ok(true),
            Some("0" | "false") => Ok(false),
            Some(other) => Err(format!("setting {key} is not a boolean: {other:?}")),
        }
    }

    pub fn text_or(&self, key: &str, default: &str) -> String {
        self.text(key).unwrap_or_else(|| default.to_owned())
    }

    /// `printable_area`-style point lists (`"0x0", "256x0", ...`).
    pub fn points(&self, key: &str) -> Result<Vec<[f64; 2]>, String> {
        self.list(key)
            .iter()
            .map(|entry| {
                let (x, y) = entry
                    .split_once('x')
                    .ok_or_else(|| format!("setting {key} has a bad point {entry:?}"))?;
                Ok([parse_number(key, x)?, parse_number(key, y)?])
            })
            .collect()
    }
}

fn parse_number(key: &str, text: &str) -> Result<f64, String> {
    let trimmed = text.trim().trim_end_matches('%');
    trimmed
        .parse::<f64>()
        .map_err(|_| format!("setting {key} is not a number: {text:?}"))
}

fn entry(archive: &mut ZipArchive<File>, name: &str) -> Result<Option<String>, String> {
    let Ok(mut file) = archive.by_name(name) else {
        return Ok(None);
    };
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|error| format!("{name}: {error}"))?;
    Ok(Some(text))
}

/// Visits the start (and empty) tags of an XML text with their attributes.
fn each_tag(
    text: &str,
    mut visit: impl FnMut(&str, &BTreeMap<String, String>, bool),
) -> Result<(), String> {
    let mut reader = Reader::from_str(text);
    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("bad XML: {error}"))?;
        let (tag, empty) = match &event {
            Event::Start(tag) => (tag, false),
            Event::Empty(tag) => (tag, true),
            Event::End(tag) => {
                visit(
                    &format!("/{}", tag.local_name().into_inner()),
                    &BTreeMap::new(),
                    false,
                );
                continue;
            }
            Event::Eof => return Ok(()),
            _ => continue,
        };
        let name = tag.local_name().into_inner().to_owned();
        let mut attributes = BTreeMap::new();
        for attribute in tag.attributes().flatten() {
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|error| format!("bad XML attribute: {error}"))?
                .into_owned();
            attributes.insert(attribute.key.local_name().into_inner().to_owned(), value);
        }
        visit(&name, &attributes, empty);
    }
}

/// The `Application` metadata text of the core model.
fn application(core: &str) -> Result<Option<String>, String> {
    let mut reader = Reader::from_str(core);
    let mut inside = false;
    loop {
        match reader
            .read_event()
            .map_err(|error| format!("{CORE_MODEL}: {error}"))?
        {
            Event::Start(tag) if tag.local_name().into_inner() == "metadata" => {
                inside = tag.attributes().flatten().any(|attribute| {
                    attribute.key.local_name().into_inner() == "name"
                        && attribute.value.as_ref() == "Application"
                });
            }
            Event::Text(text) if inside => {
                return Ok(Some(text.xml10_content().trim().to_owned()));
            }
            Event::End(_) => inside = false,
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// The object id of the first build item.
fn first_item_object(core: &str) -> Result<Option<String>, String> {
    let mut found = None;
    each_tag(core, |name, attributes, _| {
        if name == "item" && found.is_none() {
            found = attributes.get("objectid").cloned();
        }
    })?;
    Ok(found)
}

/// Object-level `<metadata key value>` of object `id` (parts and plates are skipped).
fn object_overrides(model: &str, id: &str) -> Result<Vec<(String, String)>, String> {
    let mut overrides = Vec::new();
    let mut depth_in_object: Option<usize> = None;
    let mut depth = 0usize;
    each_tag(model, |name, attributes, empty| {
        if let Some(closing) = name.strip_prefix('/') {
            depth = depth.saturating_sub(1);
            if closing == "object" && depth_in_object == Some(depth) {
                depth_in_object = None;
            }
            return;
        }
        if name == "object" && attributes.get("id").map(String::as_str) == Some(id) {
            depth_in_object = Some(depth);
        }
        let directly_in_object = depth_in_object.is_some_and(|object| depth == object + 1);
        if name == "metadata"
            && directly_in_object
            && let (Some(key), Some(value)) = (attributes.get("key"), attributes.get("value"))
            && !NOT_SETTINGS.contains(&key.as_str())
        {
            overrides.push((key.clone(), value.clone()));
        }
        if !empty {
            depth += 1;
        }
    })?;
    Ok(overrides)
}

/// The ranges of the first object in `layer_config_ranges.xml`.
fn layer_ranges(text: &str) -> Result<Vec<LayerRange>, String> {
    let mut ranges: Vec<LayerRange> = Vec::new();
    let mut objects = 0usize;
    let mut current_option: Option<String> = None;
    let mut reader = Reader::from_str(text);
    loop {
        match reader
            .read_event()
            .map_err(|error| format!("{LAYER_RANGES}: {error}"))?
        {
            Event::Start(tag) if tag.local_name().into_inner() == "object" => objects += 1,
            Event::Start(tag) if tag.local_name().into_inner() == "range" && objects == 1 => {
                let attribute = |name: &str| {
                    tag.attributes()
                        .flatten()
                        .find(|attribute| attribute.key.local_name().into_inner() == name)
                        .and_then(|attribute| attribute.value.trim().parse::<f64>().ok())
                };
                ranges.push(LayerRange {
                    min_z: attribute("min_z").ok_or("layer range without min_z")?,
                    max_z: attribute("max_z").ok_or("layer range without max_z")?,
                    layer_height: None,
                });
            }
            Event::Start(tag) if tag.local_name().into_inner() == "option" && objects == 1 => {
                current_option = tag
                    .attributes()
                    .flatten()
                    .find(|attribute| attribute.key.local_name().into_inner() == "opt_key")
                    .map(|attribute| attribute.value.to_string());
            }
            Event::Text(text) if current_option.as_deref() == Some("layer_height") => {
                let value = text.xml10_content().trim().parse::<f64>().ok();
                if let Some(range) = ranges.last_mut() {
                    range.layer_height = value;
                }
            }
            Event::End(tag) if tag.local_name().into_inner() == "option" => current_option = None,
            Event::Eof => return Ok(ranges),
            _ => {}
        }
    }
}

/// The profile of the first object (`object_id=1|z;h;z;h...`).
fn layer_profile(text: &str) -> Option<Vec<f64>> {
    let line = text.lines().find(|line| line.starts_with("object_id=1|"))?;
    let (_, values) = line.split_once('|')?;
    values
        .split(';')
        .map(|value| value.trim().parse::<f64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_strings_arrays_and_percentages() {
        let config = ProjectConfig::from_json(
            r#"{"layer_height": "0.28", "nozzle_diameter": ["0.4", "0.6"],
                "tree_support_top_rate": "30%", "enable_support": "1",
                "printable_area": ["0x0", "256x0", "256x256", "0x256"]}"#,
        )
        .unwrap();
        assert_eq!(config.number("layer_height").unwrap(), Some(0.28));
        assert_eq!(config.number("nozzle_diameter").unwrap(), Some(0.4));
        assert_eq!(config.number("tree_support_top_rate").unwrap(), Some(30.0));
        assert!(config.is_percent("tree_support_top_rate"));
        assert!(config.flag_or("enable_support", false).unwrap());
        assert_eq!(config.points("printable_area").unwrap()[2], [256.0, 256.0]);
        assert_eq!(config.number("missing").unwrap(), None);
    }

    #[test]
    fn layer_ranges_and_object_overrides() {
        let ranges = layer_ranges(
            r#"<objects><object id="1">
                <range min_z="14.9" max_z="18.24"><option opt_key="extruder">3</option>
                <option opt_key="layer_height">0.28</option></range>
              </object></objects>"#,
        )
        .unwrap();
        assert_eq!(
            ranges,
            vec![LayerRange {
                min_z: 14.9,
                max_z: 18.24,
                layer_height: Some(0.28)
            }]
        );
        let overrides = object_overrides(
            r#"<config><object id="3"><metadata key="name" value="goat"/>
                <metadata key="support_threshold_angle" value="45"/>
                <part id="1"><metadata key="extruder" value="2"/></part></object>
              <object id="4"><metadata key="support_threshold_angle" value="10"/></object></config>"#,
            "3",
        )
        .unwrap();
        assert_eq!(
            overrides,
            vec![("support_threshold_angle".to_owned(), "45".to_owned())]
        );
        assert_eq!(
            layer_profile("object_id=1|0;0.2;0.2;0.2;0.2;0.28;44.7;0.28\n"),
            Some(vec![0.0, 0.2, 0.2, 0.2, 0.2, 0.28, 44.7, 0.28])
        );
    }
}

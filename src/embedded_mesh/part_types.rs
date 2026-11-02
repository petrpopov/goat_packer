//! Volume types of an object's parts, read from Bambu/Orca `Metadata/model_settings.config`.
//!
//! A `<part>` names its volume either by `id` (the component's `objectid`, the layout every
//! current Bambu/Orca project uses) or by a triangle range `firstid..=lastid` of one inline
//! mesh (older single-mesh projects). Parts the config does not mention are normal parts, as
//! in the slicer's loader.

use std::collections::BTreeMap;

use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use serde::{Deserialize, Serialize};

use super::invalid_data;

pub(super) const MODEL_SETTINGS_ENTRY: &str = "Metadata/model_settings.config";

/// What a volume does to the print (3MF `subtype`, spec 01 §1.4).
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PartKind {
    /// Printed material: the only kind that makes the body, the frame centre and the paint
    /// that supports read.
    #[default]
    Normal,
    /// Removes material of the normal parts listed before it.
    Negative,
    /// Changes settings only, never geometry.
    Modifier,
    SupportEnforcer,
    SupportBlocker,
}

impl PartKind {
    fn from_subtype(subtype: &str) -> Result<Self, String> {
        match subtype {
            "" | "normal_part" => Ok(PartKind::Normal),
            "negative_part" => Ok(PartKind::Negative),
            "modifier_part" => Ok(PartKind::Modifier),
            "support_enforcer" => Ok(PartKind::SupportEnforcer),
            "support_blocker" => Ok(PartKind::SupportBlocker),
            other => Err(format!("unknown part subtype {other:?}")),
        }
    }
}

/// The part kinds of one object: by component object id, and by triangle range for the
/// single-mesh layout.
#[derive(Clone, Debug, Default)]
pub(super) struct PartKinds {
    pub by_id: BTreeMap<u32, PartKind>,
    /// `(first_triangle, last_triangle, kind)`, inclusive, in config order.
    pub by_range: Vec<(u32, u32, PartKind)>,
}

impl PartKinds {
    pub fn kind_of(&self, object_id: u32) -> PartKind {
        self.by_id.get(&object_id).copied().unwrap_or_default()
    }
}

/// Reads the `<part>` entries of `<object id = object_id>`; an absent config gives no entries.
pub(super) fn part_kinds(config: Option<&str>, object_id: u32) -> std::io::Result<PartKinds> {
    let mut kinds = PartKinds::default();
    let Some(config) = config else {
        return Ok(kinds);
    };
    let mut reader = Reader::from_reader(config.as_bytes());
    let mut in_object = false;
    loop {
        let event = reader.read_event().map_err(|error| {
            invalid_data(format!(
                "{MODEL_SETTINGS_ENTRY} is not well-formed XML: {error}"
            ))
        })?;
        match event {
            Event::Start(start) | Event::Empty(start) => match start.local_name().into_inner() {
                "object" => in_object = attribute(&start, "id")? == Some(object_id.to_string()),
                "part" if in_object => add_part(&mut kinds, &start)?,
                _ => {}
            },
            Event::End(end) if end.local_name().into_inner() == "object" => in_object = false,
            Event::Eof => return Ok(kinds),
            _ => {}
        }
    }
}

fn add_part(kinds: &mut PartKinds, start: &BytesStart<'_>) -> std::io::Result<()> {
    let subtype = attribute(start, "subtype")?.unwrap_or_default();
    let kind = PartKind::from_subtype(&subtype).map_err(invalid_data)?;
    let first = number(attribute(start, "firstid")?)?;
    let last = number(attribute(start, "lastid")?)?;
    // The loader prefers the triangle range when `lastid` is set (single-mesh layout).
    match (first, last) {
        (first, Some(last)) if last > 0 => kinds.by_range.push((first.unwrap_or(0), last, kind)),
        _ => {
            if let Some(id) = number(attribute(start, "id")?)? {
                kinds.by_id.insert(id, kind);
            }
        }
    }
    Ok(())
}

fn number(value: Option<String>) -> std::io::Result<Option<u32>> {
    value
        .map(|text| {
            text.trim()
                .parse::<u32>()
                .map_err(|_| invalid_data(format!("part index is not a number: {text:?}")))
        })
        .transpose()
}

fn attribute(start: &BytesStart<'_>, key: &str) -> std::io::Result<Option<String>> {
    for attribute in start.attributes() {
        let attribute = attribute
            .map_err(|error| invalid_data(format!("{MODEL_SETTINGS_ENTRY} attribute: {error}")))?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.value.to_string()));
        }
    }
    Ok(None)
}

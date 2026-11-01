//! Small helpers over xmltree elements, and the XML writer of the 3MF parts.
//!
//! xmltree drops namespace prefixes, so `emit_core_xml` restores `p:UUID` / `p:path` /
//! `xml:lang` by string replacement. `assemble` avoids this with a quick-xml round-trip; new
//! 3MF edits belong there.

use std::io::Cursor;

use sha2::{Digest, Sha256};
use xmltree::{Element, EmitterConfig, XMLNode};

use super::SchedulerResult;

pub(super) fn child_mut<'a>(
    element: &'a mut Element,
    name: &str,
) -> SchedulerResult<&'a mut Element> {
    element
        .children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .find(|child| child.name == name)
        .ok_or_else(|| format!("XML child not found: {name}").into())
}

pub(super) fn element_children<'a>(
    element: &'a Element,
    name: &'a str,
) -> impl Iterator<Item = &'a Element> {
    element
        .children
        .iter()
        .filter_map(XMLNode::as_element)
        .filter(move |child| child.name == name)
}

pub(super) fn element_children_mut<'a>(
    element: &'a mut Element,
    name: &'a str,
) -> impl Iterator<Item = &'a mut Element> {
    element
        .children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .filter(move |child| child.name == name)
}

/// Drops the children called `name` that `keep` rejects; other children stay.
pub(super) fn retain_elements<F>(element: &mut Element, name: &str, mut keep: F)
where
    F: FnMut(&Element) -> bool,
{
    element.children.retain(|node| {
        node.as_element()
            .filter(|child| child.name == name)
            .is_none_or(&mut keep)
    });
}

pub(super) fn required_attr(element: &Element, name: &str) -> SchedulerResult<String> {
    element
        .attributes
        .get(name)
        .cloned()
        .ok_or_else(|| format!("attribute {name} missing on {}", element.name).into())
}

/// Whether the attribute `name` of `element` is one of `kept`.
pub(super) fn attr_is_kept(
    element: &Element,
    name: &str,
    kept: &std::collections::HashSet<&str>,
) -> bool {
    element
        .attributes
        .get(name)
        .is_some_and(|id| kept.contains(id.as_str()))
}

pub(super) fn metadata_value<'a>(element: &'a Element, key: &str) -> Option<&'a str> {
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

pub(super) fn set_metadata_value(element: &mut Element, key: &str, value: &str) {
    for metadata in element_children_mut(element, "metadata") {
        if metadata.attributes.get("key").map(String::as_str) == Some(key) {
            metadata
                .attributes
                .insert("value".to_owned(), value.to_owned());
        }
    }
}

pub(super) fn push_metadata(element: &mut Element, key: &str, value: &str) {
    let mut metadata = Element::new("metadata");
    metadata.attributes.insert("key".to_owned(), key.to_owned());
    metadata
        .attributes
        .insert("value".to_owned(), value.to_owned());
    element.children.push(XMLNode::Element(metadata));
}

pub(super) fn emit_xml(element: &Element) -> SchedulerResult<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    element.write_with_config(
        &mut output,
        EmitterConfig::new()
            .perform_indent(true)
            .write_document_declaration(true),
    )?;
    Ok(output.into_inner())
}

pub(super) fn emit_core_xml(element: &Element) -> SchedulerResult<Vec<u8>> {
    let xml = String::from_utf8(emit_xml(element)?)?
        .replace(" UUID=", " p:UUID=")
        .replace(" path=", " p:path=")
        .replace(" lang=", " xml:lang=");
    Ok(xml.into_bytes())
}

/// Deterministic pseudo-UUID (not RFC 4122 version/variant compliant, but the same opaque
/// 8-4-4-4-12 hex shape BambuStudio's own `p:UUID`/`uuid` attributes use) derived from `seed`, so
/// re-running the same schedule produces byte-identical plate XML.
pub(super) fn stable_uuid(seed: &str) -> String {
    let hash = Sha256::digest(seed.as_bytes());
    let hex = |range: std::ops::Range<usize>| -> String {
        hash[range]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    };
    format!(
        "{}-{}-{}-{}-{}",
        hex(0..4),
        hex(4..6),
        hex(6..8),
        hex(8..10),
        hex(10..16)
    )
}

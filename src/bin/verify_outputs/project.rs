//! Reading the parts of a packed 3MF: project settings, core model, meshes and model settings.

use std::{collections::HashSet, fs, io::Read as _};

use quick_xml::{
    events::{BytesStart, Event},
    name::{NamespaceResolver, ResolveResult},
    reader::NsReader,
};
use serde_json::Value;
use zip::ZipArchive;

use crate::Failure;

pub const CORE: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const PRODUCTION: &str = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06";

/// `parse_xy`: "WxH" strings → (x, y) pairs.
pub fn parse_xy(values: &[Value]) -> Result<Vec<(f64, f64)>, Failure> {
    values
        .iter()
        .map(|value| {
            let text = value.as_str().ok_or_else(|| {
                Failure::Error(format!("printable area entry {value} is not a string"))
            })?;
            let mut parts = text.split('x');
            let mut coordinate = |name: &str| {
                parts.next().ok_or_else(|| {
                    Failure::Error(format!("printable area entry {text:?} has no {name} part"))
                })
            };
            let x = coordinate("x")?.parse::<f64>().map_err(|error| {
                Failure::Error(format!("printable area entry {text:?}: {error}"))
            })?;
            let y = coordinate("y")?.parse::<f64>().map_err(|error| {
                Failure::Error(format!("printable area entry {text:?}: {error}"))
            })?;
            Ok((x, y))
        })
        .collect()
}

pub fn read_entry(archive: &mut ZipArchive<fs::File>, name: &str) -> Result<String, Failure> {
    let mut entry = archive
        .by_name(name)
        .map_err(|error| Failure::Error(format!("archive is missing {name}: {error}")))?;
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|error| Failure::Error(format!("failed to read {name}: {error}")))?;
    Ok(text)
}

#[derive(Debug, Default)]
pub struct ParsedItem {
    /// Core `objectid`; a missing attribute becomes `None` (and never matches a resource id).
    pub object_id: Option<String>,
    pub transform: Option<String>,
}

#[derive(Debug, Default)]
pub struct ParsedProjectModel {
    /// `<object>` element count under `<resources>` (duplicates counted, like ElementTree).
    pub resource_objects: usize,
    /// Core `id` of every resource object; a missing id maps to "" which never matches.
    pub resource_ids: HashSet<String>,
    pub items: Vec<ParsedItem>,
    /// Production `p:path` per `<component>` in document order (None when the attr is absent).
    pub component_paths: Vec<Option<String>>,
    /// Transform of the first component nested under resources → object → components.
    pub first_resource_component_transform: Option<String>,
}

pub fn parse_project_model(xml: &str) -> Result<ParsedProjectModel, Failure> {
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut parsed = ParsedProjectModel::default();
    // (namespace URI, local name) ancestry stack: the ElementTree `.//a/b` paths are matched
    // against it by namespace URI + parent instead of by prefix spelling.
    let mut stack: Vec<(String, String)> = Vec::new();
    let mut seen_resource_component = false;
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|error| {
            Failure::Error(format!("failed to parse 3D/3dmodel.model: {error}"))
        })?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let empty = matches!(event, Event::Empty(_));
                let ns_uri = namespace_str(&namespace)?;
                let local = start.name().local_name().into_inner().to_owned();
                let parent = stack.last().map(|(ns, name)| (ns.as_str(), name.as_str()));
                let grandparent = stack
                    .len()
                    .checked_sub(2)
                    .and_then(|index| stack.get(index))
                    .map(|(ns, name)| (ns.as_str(), name.as_str()));
                if ns_uri == CORE && local == "object" && parent == Some((CORE, "resources")) {
                    parsed.resource_objects += 1;
                    parsed
                        .resource_ids
                        .insert(attr(start, reader.resolver(), None, "id").unwrap_or_default());
                } else if ns_uri == CORE && local == "item" && parent == Some((CORE, "build")) {
                    parsed.items.push(ParsedItem {
                        object_id: attr(start, reader.resolver(), None, "objectid"),
                        transform: attr(start, reader.resolver(), None, "transform"),
                    });
                } else if ns_uri == CORE && local == "component" {
                    parsed.component_paths.push(attr(
                        start,
                        reader.resolver(),
                        Some(PRODUCTION),
                        "path",
                    ));
                    // root.find(".//c:resources/c:object/c:components/c:component"): the first
                    // component nested under resources → object → components, document order.
                    if !seen_resource_component
                        && parent == Some((CORE, "components"))
                        && grandparent == Some((CORE, "object"))
                        && stack
                            .iter()
                            .any(|(ns, name)| ns == CORE && name == "resources")
                    {
                        seen_resource_component = true;
                        parsed.first_resource_component_transform =
                            attr(start, reader.resolver(), None, "transform");
                    }
                }
                stack.push((ns_uri, local));
                if empty {
                    stack.pop();
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(parsed)
}

pub fn parse_mesh_vertices(xml: &str) -> Result<Vec<(f64, f64, f64)>, Failure> {
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut vertices = Vec::new();
    loop {
        let (namespace, event) = reader
            .read_resolved_event()
            .map_err(|error| Failure::Error(format!("failed to parse component mesh: {error}")))?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let ns_uri = namespace_str(&namespace)?;
                let local = start.name().local_name().into_inner();
                if ns_uri == CORE && local == "vertex" {
                    let number = |name: &str| -> Result<f64, Failure> {
                        let raw = attr(start, reader.resolver(), None, name).ok_or_else(|| {
                            Failure::Error(format!("mesh vertex is missing the {name} attribute"))
                        })?;
                        raw.parse::<f64>().map_err(|error| {
                            Failure::Error(format!(
                                "mesh vertex {name}=\"{raw}\" is not a number: {error}"
                            ))
                        })
                    };
                    vertices.push((number("x")?, number("y")?, number("z")?));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(vertices)
}

/// Returns (object count, plate/model_instance count) — ElementTree's `findall("object")` and
/// `findall("plate/model_instance")` on the namespace-free model_settings.config.
pub fn parse_model_settings(xml: &str) -> Result<(usize, usize), Failure> {
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut objects = 0;
    let mut instances = 0;
    let mut stack: Vec<(String, String)> = Vec::new();
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|error| {
            Failure::Error(format!(
                "failed to parse Metadata/model_settings.config: {error}"
            ))
        })?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let empty = matches!(event, Event::Empty(_));
                let ns_uri = namespace_str(&namespace)?;
                let local = start.name().local_name().into_inner().to_owned();
                let parent = stack.last().map(|(ns, name)| (ns.as_str(), name.as_str()));
                // Root-level <object> (depth 1) and <model_instance> under a root-level
                // <plate> (depth 2): stack still holds only ancestors while we process.
                if ns_uri.is_empty() && local == "object" && stack.len() == 1 {
                    objects += 1;
                }
                if ns_uri.is_empty()
                    && local == "model_instance"
                    && stack.len() == 2
                    && parent == Some(("", "plate"))
                {
                    instances += 1;
                }
                stack.push((ns_uri, local));
                if empty {
                    stack.pop();
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((objects, instances))
}

/// Attribute lookup by (namespace URI, local name) — the ElementTree `{uri}local` spelling.
/// `None` means an unprefixed (no-namespace) attribute.
fn attr(
    start: &BytesStart,
    resolver: &NamespaceResolver,
    uri: Option<&str>,
    local: &str,
) -> Option<String> {
    start
        .attributes()
        .with_checks(false)
        .filter_map(Result::ok)
        .find(|attribute| {
            let (attribute_ns, attribute_local) = resolver.resolve_attribute(attribute.key);
            let namespace_ok = match (uri, &attribute_ns) {
                (Some(uri), ResolveResult::Bound(bound)) => bound.0 == uri,
                (None, ResolveResult::Unbound) => true,
                _ => false,
            };
            namespace_ok && attribute_local.into_inner() == local
        })
        .map(|attribute| attribute.value.into_owned())
}

fn namespace_str(namespace: &ResolveResult) -> Result<String, Failure> {
    Ok(match namespace {
        ResolveResult::Bound(bound) => bound.0.to_owned(),
        ResolveResult::Unbound => String::new(),
        ResolveResult::Unknown(prefix) => {
            return Err(Failure::Error(format!(
                "undeclared namespace prefix: {prefix}"
            )));
        }
    })
}

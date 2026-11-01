//! The mesh strategies of `AssembleOptions`: where the clones' geometry comes from.

use super::archive::{CORE_MODEL_ENTRY, Entries, RELATIONSHIPS_ENTRY, entry, set_entry};
use super::format::number;
use super::xml::{
    attribute_value, child_spans, element_is, parse_events, root_span, set_attribute,
    stack_ends_with,
};
use crate::embedded_mesh;
use quick_xml::events::{BytesStart, Event};

/// The first `resources/object/components/component` element's production path and optional
/// transform — the same strict `/`-separated ElementTree path the Python resolves for both
/// duplicate and inline modes.
pub(super) struct SourceComponent {
    pub(super) path: String,
    pub(super) transform: Option<String>,
}

pub(super) fn first_source_component(core: &[u8]) -> Result<SourceComponent, String> {
    let events = parse_events(core, CORE_MODEL_ENTRY)?;
    let mut stack: Vec<String> = Vec::new();
    for event in &events {
        match event {
            Event::Start(start) | Event::Empty(start) => {
                let name = start.local_name().into_inner().to_owned();
                if name == "component"
                    && stack_ends_with(&stack, &["resources", "object", "components"])
                {
                    let path = attribute_value(start, "p:path")?.unwrap_or_default();
                    if path.is_empty() {
                        return Err("source component has no production path".to_owned());
                    }
                    return Ok(SourceComponent {
                        path,
                        transform: attribute_value(start, "transform")?,
                    });
                }
                if matches!(event, Event::Start(_)) {
                    stack.push(name);
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            _ => {}
        }
    }
    Err("3MF core model has no source component".to_owned())
}

/// Contract 2: duplicated mesh parts are byte copies of the source entry — `paint_color`
/// attributes live there and must never be touched. Returns the per-index component paths.
pub(super) fn duplicate_colored_mesh_parts(
    entries: &mut Entries,
    core: &[u8],
    count: usize,
) -> Result<Vec<String>, String> {
    let component = first_source_component(core)?;
    let source_path = component.path;
    let source_entry = source_path.trim_start_matches('/').to_owned();
    if entry(entries, &source_entry).is_none() {
        return Err(format!(
            "source mesh part missing from archive: {source_entry}"
        ));
    }
    let mut paths = vec![source_path];
    if count == 1 {
        return Ok(paths);
    }
    let base = source_entry
        .strip_suffix(".model")
        .unwrap_or(&source_entry)
        .to_owned();
    // Relationships are inserted by plain string surgery, mirroring the Python: only when the
    // rels entry exists and is non-empty. `str.replace` would rewrite every closing tag, but
    // well-formed XML has exactly one, so inserting before the first is equivalent.
    let mut relationship_text: Option<String> = entry(entries, RELATIONSHIPS_ENTRY)
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .filter(|text| !text.is_empty());
    for index in 1..count {
        let duplicate_entry = format!("{base}_instance_{}.model", index + 1);
        let source_bytes = entry(entries, &source_entry)
            .expect("source entry checked above")
            .to_vec();
        set_entry(entries, &duplicate_entry, source_bytes);
        let duplicate_path = format!("/{duplicate_entry}");
        paths.push(duplicate_path.clone());
        if let Some(text) = relationship_text.as_mut()
            && let Some(position) = text.find("</Relationships>")
        {
            let relationship = format!(
                " <Relationship Target=\"{duplicate_path}\" Id=\"paint-copy-{}\" \
                 Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\n",
                index + 1
            );
            text.insert_str(position, &relationship);
        }
    }
    if let Some(text) = relationship_text {
        set_entry(entries, RELATIONSHIPS_ENTRY, text.into_bytes());
    }
    Ok(paths)
}

/// Builds the inline mesh object for `inline_colored_mesh_objects` mode: the mesh part's
/// `<object>` element with the component transform baked into every vertex.
pub(super) fn load_inline_mesh_template(
    entries: &Entries,
    core: &[u8],
) -> Result<Vec<Event<'static>>, String> {
    let component = first_source_component(core)?;
    let source_entry = component.path.trim_start_matches('/');
    let mesh_bytes = entry(entries, source_entry)
        .ok_or_else(|| format!("source mesh part missing from archive: {source_entry}"))?
        .to_vec();
    let events = parse_events(&mesh_bytes, source_entry)?;
    let missing = || "source component has no inline mesh object".to_owned();
    let (root_start, root_end) = root_span(&events)?;
    let resources = child_spans(&events, root_start, root_end)
        .into_iter()
        .find(|&span| element_is(&events[span.0], "resources"))
        .ok_or_else(missing)?;
    let object_span = child_spans(&events, resources.0, resources.1)
        .into_iter()
        .find(|&span| element_is(&events[span.0], "object"))
        .ok_or_else(missing)?;
    if !child_spans(&events, object_span.0, object_span.1)
        .iter()
        .any(|&span| element_is(&events[span.0], "mesh"))
    {
        return Err(missing());
    }
    let mut object = events[object_span.0..object_span.1].to_vec();
    // An inlined mesh replaces the component, so the component's transform — normally applied
    // between mesh and build item — must be baked into the vertex coordinates to keep the
    // object-local geometry identical to the component-based modes (see `item_transform`).
    let matrix = embedded_mesh::parse_transform(component.transform.as_deref().unwrap_or(""))?;
    for event in object.iter_mut().skip(1) {
        if element_is(event, "vertex")
            && let Event::Start(start) | Event::Empty(start) = event
        {
            bake_vertex(start, &matrix)?;
        }
    }
    Ok(object)
}

/// Rewrites one `<vertex>`'s x/y/z with the matrix applied, through the same `_number`
/// formatting the rest of the generated XML uses.
fn bake_vertex(start: &mut BytesStart<'static>, matrix: &[[f64; 3]; 4]) -> Result<(), String> {
    let mut axes = [None::<f64>; 3];
    for attribute in start.attributes() {
        let attribute =
            attribute.map_err(|error| format!("malformed attribute in XML part: {error}"))?;
        let slot = match attribute.key.as_ref() {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            _ => continue,
        };
        let value = attribute
            .value
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("vertex coordinate is not a number: {:?}", attribute.value))?;
        axes[slot] = Some(value);
    }
    let [Some(x), Some(y), Some(z)] = axes else {
        return Err("vertex is missing an x/y/z coordinate".to_owned());
    };
    let (x, y, z) = embedded_mesh::apply_matrix(matrix, (x, y, z));
    set_attribute(start, "x", &number(x))?;
    set_attribute(start, "y", &number(y))?;
    set_attribute(start, "z", &number(z))?;
    Ok(())
}

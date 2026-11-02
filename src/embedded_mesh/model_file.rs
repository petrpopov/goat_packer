//! One 3MF model part (`3D/3dmodel.model` or a production-extension `3D/Objects/*.model`) read
//! into its objects (inline meshes with paint, component lists) and build items.
//!
//! Everything is kept in the file's own coordinates: transforms are composed by the caller,
//! because the same object can be referenced by several components.

use std::collections::BTreeMap;

use quick_xml::{
    Reader, XmlVersion,
    events::{BytesStart, Event},
};

use super::{Matrix, invalid_data, parse_transform};

/// An inline mesh exactly as stored: vertex order and triangle order are kept, because paint
/// codes are addressed by triangle index and their child geometry by vertex order
/// (spec 01 §2.1, §2.3).
#[derive(Clone, Debug, Default)]
pub(super) struct RawMesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub paint_supports: BTreeMap<u32, String>,
    pub paint_color: BTreeMap<u32, String>,
}

/// A `<component>` reference: the object it instantiates (in `path`, or in this file when
/// `path` is `None`) and the component transform.
#[derive(Clone, Debug)]
pub(super) struct ComponentRef {
    pub object_id: u32,
    pub path: Option<String>,
    pub transform: Matrix,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ObjectDef {
    pub mesh: Option<RawMesh>,
    pub components: Vec<ComponentRef>,
}

#[derive(Clone, Debug)]
pub(super) struct BuildItem {
    pub object_id: u32,
    pub transform: Matrix,
}

#[derive(Debug, Default)]
pub(super) struct ModelFile {
    pub objects: BTreeMap<u32, ObjectDef>,
    pub items: Vec<BuildItem>,
}

/// Parses every `resources/object` and `build/item` of one model part. `part` names the zip
/// entry in error messages.
pub(super) fn parse_model_file(xml: &str, part: &str) -> std::io::Result<ModelFile> {
    let mut reader = Reader::from_reader(xml.as_bytes());
    let mut parser = ModelParser::default();
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_data(format!("{part} is not well-formed XML: {error}")))?;
        match event {
            Event::Start(start) => {
                parser.open(&start, part)?;
                parser
                    .stack
                    .push(start.local_name().into_inner().to_owned());
            }
            Event::Empty(start) => {
                parser.open(&start, part)?;
                parser.close_element(start.local_name().into_inner());
            }
            Event::End(end) => {
                parser.stack.pop();
                parser.close_element(end.local_name().into_inner());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(parser.file)
}

/// Streaming state: the element stack and the object being read.
#[derive(Default)]
struct ModelParser {
    stack: Vec<String>,
    current: Option<(u32, ObjectDef)>,
    file: ModelFile,
}

impl ModelParser {
    fn open(&mut self, start: &BytesStart<'_>, part: &str) -> std::io::Result<()> {
        let name = start.local_name().into_inner();
        let context = |what: &str| format!("{part}: {what}");
        match name {
            "object" if self.stack_ends_with(&["resources"]) => {
                let id = required_u32(start, "id").map_err(|e| invalid_data(context(&e)))?;
                self.current = Some((id, ObjectDef::default()));
            }
            "mesh" => {
                if let Some((_, object)) = self.current.as_mut() {
                    object.mesh.get_or_insert_with(RawMesh::default);
                }
            }
            "vertex" => {
                let vertex = vertex(start).map_err(|e| invalid_data(context(&e)))?;
                self.mesh()?.vertices.push(vertex);
            }
            "triangle" => {
                let mesh = self.mesh()?;
                let index = mesh.triangles.len() as u32;
                let (corners, supports, color) =
                    triangle(start).map_err(|e| invalid_data(context(&e)))?;
                mesh.triangles.push(corners);
                if let Some(code) = supports {
                    mesh.paint_supports.insert(index, code);
                }
                if let Some(code) = color {
                    mesh.paint_color.insert(index, code);
                }
            }
            "component" if self.stack_ends_with(&["object", "components"]) => {
                let component = component(start).map_err(|e| invalid_data(context(&e)))?;
                if let Some((_, object)) = self.current.as_mut() {
                    object.components.push(component);
                }
            }
            "item" if self.stack_ends_with(&["build"]) => {
                let item = build_item(start).map_err(|e| invalid_data(context(&e)))?;
                self.file.items.push(item);
            }
            _ => {}
        }
        Ok(())
    }

    /// Called when an element ends (or right after an empty element): finishing an object
    /// stores it.
    fn close_element(&mut self, name: &str) {
        if name == "object"
            && self.stack_ends_with(&["resources"])
            && let Some((id, object)) = self.current.take()
        {
            self.file.objects.insert(id, object);
        }
    }

    fn mesh(&mut self) -> std::io::Result<&mut RawMesh> {
        self.current
            .as_mut()
            .and_then(|(_, object)| object.mesh.as_mut())
            .ok_or_else(|| invalid_data("vertex or triangle outside an object mesh"))
    }

    fn stack_ends_with(&self, suffix: &[&str]) -> bool {
        self.stack.len() >= suffix.len()
            && self.stack[self.stack.len() - suffix.len()..]
                .iter()
                .zip(suffix)
                .all(|(entry, expected)| entry == expected)
    }
}

fn vertex(start: &BytesStart<'_>) -> Result<[f64; 3], String> {
    let mut axes = [None::<f64>; 3];
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| format!("vertex attribute: {error}"))?;
        let axis = match attribute.key.as_ref() {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            _ => continue,
        };
        axes[axis] = Some(number(&attribute.value, "vertex coordinate")?);
    }
    match axes {
        [Some(x), Some(y), Some(z)] => Ok([x, y, z]),
        _ => Err("vertex is missing an x/y/z coordinate".to_owned()),
    }
}

type TriangleRecord = ([u32; 3], Option<String>, Option<String>);

/// Vertex indices plus the support and colour paint codes. Bambu and Orca write
/// `paint_supports` / `paint_color`; PrusaSlicer-style projects write the `slic3rpe:` names
/// (spec 01 §2.1).
fn triangle(start: &BytesStart<'_>) -> Result<TriangleRecord, String> {
    let mut indices = [None::<u32>; 3];
    let mut supports = None;
    let mut color = None;
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| format!("triangle attribute: {error}"))?;
        let value: &str = &attribute.value;
        match attribute.key.as_ref() {
            "v1" => indices[0] = Some(index(value)?),
            "v2" => indices[1] = Some(index(value)?),
            "v3" => indices[2] = Some(index(value)?),
            "paint_supports" | "slic3rpe:custom_supports" => supports = paint_code(value),
            "paint_color" | "slic3rpe:mmu_segmentation" => color = paint_code(value),
            _ => {}
        }
    }
    match indices {
        [Some(a), Some(b), Some(c)] => Ok(([a, b, c], supports, color)),
        _ => Err("triangle is missing a v1/v2/v3 index".to_owned()),
    }
}

fn paint_code(value: &str) -> Option<String> {
    let code = value.trim();
    (!code.is_empty()).then(|| code.to_owned())
}

fn component(start: &BytesStart<'_>) -> Result<ComponentRef, String> {
    let mut object_id = None;
    let mut path = None;
    let mut transform = String::new();
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| format!("component attribute: {error}"))?;
        match attribute.key.as_ref() {
            "objectid" => object_id = Some(index(&attribute.value)?),
            "p:path" => {
                let value = attribute
                    .normalized_value(XmlVersion::Implicit1_0)
                    .map_err(|error| format!("component path: {error}"))?;
                let trimmed = value.trim_start_matches('/');
                path = (!trimmed.is_empty()).then(|| trimmed.to_owned());
            }
            "transform" => transform = attribute.value.to_string(),
            _ => {}
        }
    }
    Ok(ComponentRef {
        object_id: object_id.ok_or("component has no objectid")?,
        path,
        transform: parse_transform(transform.trim())?,
    })
}

fn build_item(start: &BytesStart<'_>) -> Result<BuildItem, String> {
    let object_id = required_u32(start, "objectid")?;
    let mut transform = String::new();
    for attribute in start.attributes().flatten() {
        if attribute.key.as_ref() == "transform" {
            transform = attribute.value.to_string();
        }
    }
    Ok(BuildItem {
        object_id,
        transform: parse_transform(transform.trim())?,
    })
}

fn required_u32(start: &BytesStart<'_>, key: &str) -> Result<u32, String> {
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| format!("{key} attribute: {error}"))?;
        if attribute.key.as_ref() == key {
            return index(&attribute.value);
        }
    }
    Err(format!("element has no {key}"))
}

fn index(value: &str) -> Result<u32, String> {
    value
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("index is not a number: {value:?}"))
}

fn number(value: &str, context: &str) -> Result<f64, String> {
    value
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{context} is not a number: {value:?}"))
}

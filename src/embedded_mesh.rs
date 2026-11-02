//! Export a source 3MF's single embedded object mesh to a binary STL.
//!
//! The packer must collide against the exact geometry that will actually be printed. A
//! separately-exported STL for the same model can drift from the 3MF's embedded mesh (different
//! export pass, different tool version, manual edits), silently invalidating every collision
//! check that assumes the two are identical. Deriving the STL directly from the 3MF's own
//! component eliminates that class of mismatch.
//!
//! The component's own transform is baked into the exported vertices, so the STL is the
//! object-local geometry a build-item transform actually acts on. A source component carrying a
//! non-zero translation (e.g. Goat 3's object_19.model: (-35.38, +40.12)) otherwise makes the
//! raw mesh a different geometry than the one placed on the bed.
//!
//! `load_object_mesh` is the slicer-free reader (design 2026-11-02 §4.1): every volume of the
//! build item's object, with the item transform, in the object frame (`object_mesh`). The
//! first-component functions below stay for the current pipeline until it moves over (I4).

mod model_file;
mod object_mesh;
mod part_types;

pub use object_mesh::{ObjectMesh, Part, compose, determinant, load_object_mesh, sin_cos_degrees};
pub use part_types::PartKind;

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::Path,
};

use quick_xml::{
    Reader,
    events::{BytesStart, Event},
    XmlVersion,
};
use zip::ZipArchive;

const CORE_MODEL_ENTRY: &str = "3D/3dmodel.model";

/// A 3MF transform: 4x3 row-major, row vectors, last row = translation (`parse_transform`).
pub type Matrix = [[f64; 3]; 4];

/// Parses a 3MF transform attribute into a 4×3 row-major matrix (last row = translation).
///
/// 3MF uses row vectors: p' = p·M, i.e. p'_j = sum_i p_i * M[i][j] + M[3][j]. An empty string is
/// the identity, so callers can feed a missing `transform` attribute straight in.
pub fn parse_transform(text: &str) -> Result<[[f64; 3]; 4], String> {
    if text.is_empty() {
        return Ok([
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0],
        ]);
    }
    let values: Vec<f64> = text
        .split_whitespace()
        .map(|token| {
            token
                .parse::<f64>()
                .map_err(|_| format!("3MF transform value is not a number: {token:?}"))
        })
        .collect::<Result<_, _>>()?;
    if values.len() != 12 {
        return Err(format!("3MF transform is not 12 numbers: {text:?}"));
    }
    Ok([
        values[0..3].try_into().expect("three values"),
        values[3..6].try_into().expect("three values"),
        values[6..9].try_into().expect("three values"),
        values[9..12].try_into().expect("three values"),
    ])
}

/// Applies a row-vector transform matrix (see `parse_transform`) to a point: result component j
/// is `x * M[0][j] + y * M[1][j] + z * M[2][j] + M[3][j]`.
pub fn apply_matrix(matrix: &[[f64; 3]; 4], point: (f64, f64, f64)) -> (f64, f64, f64) {
    let (x, y, z) = point;
    let mut transformed = [0.0f64; 3];
    for (j, component) in transformed.iter_mut().enumerate() {
        *component = x * matrix[0][j] + y * matrix[1][j] + z * matrix[2][j] + matrix[3][j];
    }
    (transformed[0], transformed[1], transformed[2])
}

/// Returns the source 3MF's single embedded object mesh as triangles with the component's own
/// transform baked into the vertices (all arithmetic in f64, as the Python exporter does).
///
/// The core `3D/3dmodel.model` is expected to reference one mesh part through a
/// `resources/object/components/component` element: the component's production `p:path` picks the
/// zip entry to load and its optional `transform` is applied to every vertex. Without a
/// component, the core model's own inline mesh is used under an identity component transform.
pub fn embedded_mesh_triangles(source_3mf: &Path) -> io::Result<Vec<[(f64, f64, f64); 3]>> {
    let mut archive = ZipArchive::new(File::open(source_3mf)?).map_err(|error| {
        invalid_data(format!(
            "{source_3mf:?} is not a readable 3MF archive: {error}"
        ))
    })?;
    let core = zip_entry_text(&mut archive, CORE_MODEL_ENTRY)?;
    let (mesh_xml, component_matrix) = match find_first_component(&core)? {
        Some(component) => {
            let matrix = parse_transform(component.transform.as_deref().unwrap_or(""))
                .map_err(invalid_data)?;
            let part = zip_entry_text(&mut archive, &component.path)?;
            (part, matrix)
        }
        None => (
            core,
            parse_transform("").expect("empty transform text is the identity"),
        ),
    };
    first_object_mesh(&mesh_xml, &component_matrix)
}

/// Writes a binary STL whose bytes are deterministic and f32-identical to the Python exporter's
/// `struct.pack` layout: 80 zero header bytes, LE u32 triangle count, then per triangle 12 zero
/// normal bytes, the 9 vertices as LE f32 (each converted from the f64 result of the baked
/// transform, in the same vertex order), and a zero u16 attribute byte count. The packed output's
/// byte-level determinism guarantee depends on this exact layout.
pub fn write_binary_stl(path: &Path, triangles: &[[(f64, f64, f64); 3]]) -> io::Result<()> {
    let count =
        u32::try_from(triangles.len()).map_err(|_| invalid_data("too many triangles for an STL"))?;
    let mut file = io::BufWriter::new(File::create(path)?);
    file.write_all(&[0u8; 80])?;
    file.write_all(&count.to_le_bytes())?;
    for triangle in triangles {
        file.write_all(&[0u8; 12])?;
        for (x, y, z) in triangle {
            for value in [x, y, z] {
                file.write_all(&(*value as f32).to_le_bytes())?;
            }
        }
        file.write_all(&0u16.to_le_bytes())?;
    }
    file.flush()?;
    Ok(())
}

/// Returns the XY bounding-box center of a binary STL's vertices. This is the exact point every
/// build-item translation must compensate against (`placement − R(angle)·c_stl`), and it must be
/// recovered from the same STL the packer collided against. The strict length validation mirrors
/// `extract_footprint` so a truncated or padded file fails loudly instead of shifting every
/// placement by a bogus center.
/// XYZ bounding-box center of a binary STL — the point to push through a build item's full
/// 3x4 transform when the bed position of the model center is needed, not just XY.
pub fn stl_center_3d(path: &Path) -> io::Result<(f64, f64, f64)> {
    let (x, y) = stl_xy_center(path)?;
    let data = fs::read(path)?;
    let triangle_count = u32::from_le_bytes(data[80..84].try_into().expect("four bytes")) as usize;
    let mut min_z = f64::INFINITY;
    let mut max_z = f64::NEG_INFINITY;
    for index in 0..triangle_count {
        let base = 84 + index * 50 + 12;
        for vertex_index in 0..3u16 {
            let offset = base + usize::from(vertex_index) * 12 + 8;
            let z = f64::from(f32::from_le_bytes(
                data[offset..offset + 4].try_into().expect("four bytes"),
            ));
            min_z = min_z.min(z);
            max_z = max_z.max(z);
        }
    }
    Ok((x, y, (min_z + max_z) * 0.5))
}

pub fn stl_xy_center(path: &Path) -> io::Result<(f64, f64)> {
    let data = fs::read(path)?;
    if data.len() < 84 {
        return Err(invalid_data("STL header is truncated"));
    }
    let triangle_count = u32::from_le_bytes(data[80..84].try_into().expect("four bytes")) as usize;
    let expected = 84usize
        .checked_add(
            triangle_count
                .checked_mul(50)
                .ok_or_else(|| invalid_data("STL triangle count overflows"))?,
        )
        .ok_or_else(|| invalid_data("STL size overflows"))?;
    if data.len() != expected {
        return Err(invalid_data(format!(
            "binary STL has {} bytes; expected {expected}",
            data.len()
        )));
    }
    if triangle_count == 0 {
        return Err(invalid_data("STL contains no triangles"));
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for index in 0..triangle_count {
        let base = 84 + index * 50 + 12;
        for vertex_index in 0..3u16 {
            let offset = base + usize::from(vertex_index) * 12;
            let x = f64::from(f32::from_le_bytes(
                data[offset..offset + 4].try_into().expect("four bytes"),
            ));
            let y = f64::from(f32::from_le_bytes(
                data[offset + 4..offset + 8].try_into().expect("four bytes"),
            ));
            min[0] = min[0].min(x);
            min[1] = min[1].min(y);
            max[0] = max[0].max(x);
            max[1] = max[1].max(y);
        }
    }
    Ok(((min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5))
}

/// The component reference found in the core model: the zip entry holding the mesh part and the
/// component's optional `transform` attribute.
struct ComponentSource {
    path: String,
    transform: Option<String>,
}

/// Finds the first `resources/object/components/component` element (at any depth), mirroring the
/// Python `core.find(".//c:resources/c:object/c:components/c:component", NS)`.
fn find_first_component(core: &str) -> io::Result<Option<ComponentSource>> {
    let mut reader = Reader::from_reader(core.as_bytes());
    let mut stack: Vec<String> = Vec::new();
    loop {
        let event = read_event(&mut reader, CORE_MODEL_ENTRY)?;
        match event {
            Event::Start(start) => {
                let name = start.local_name().into_inner();
                if name == "component" && stack_ends_with(&stack, &["resources", "object", "components"]) {
                    return component_source(&start).map(Some);
                }
                stack.push(name.to_owned());
            }
            Event::Empty(start) => {
                let name = start.local_name().into_inner();
                if name == "component" && stack_ends_with(&stack, &["resources", "object", "components"]) {
                    return component_source(&start).map(Some);
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// Extracts the first object's mesh from `xml` (the mesh part, or the core model itself when
/// there is no component), baking `matrix` into every vertex. Takes the first object that is a
/// direct child of a `resources` element and requires it to carry a `mesh` child, exactly like
/// the Python `mesh_root.find(".//c:resources/c:object")` + `mesh` presence check.
fn first_object_mesh(xml: &str, matrix: &[[f64; 3]; 4]) -> io::Result<Vec<[(f64, f64, f64); 3]>> {
    let mut reader = Reader::from_reader(xml.as_bytes());
    let mut stack: Vec<String> = Vec::new();
    // Stack length (before push) at which the tracked `<object>` element sits.
    let mut object_depth: Option<usize> = None;
    let mut has_mesh = false;
    let mut vertices: Vec<(f64, f64, f64)> = Vec::new();
    let mut triangles: Vec<[usize; 3]> = Vec::new();
    loop {
        match read_event(&mut reader, "the mesh model")? {
            Event::Start(start) => {
                let name = start.local_name().into_inner();
                let depth = stack.len();
                match object_depth {
                    None => {
                        if name == "object" && stack_ends_with(&stack, &["resources"]) {
                            object_depth = Some(depth);
                        }
                    }
                    Some(object_depth) => {
                        let direct_child_of_object = depth == object_depth + 1;
                        match name {
                            "mesh" if direct_child_of_object => has_mesh = true,
                            "vertex" => vertices.push(vertex_position(&start, matrix)?),
                            "triangle" => triangles.push(triangle_indices(&start)?),
                            _ => {}
                        }
                    }
                }
                stack.push(name.to_owned());
            }
            Event::Empty(start) => {
                let name = start.local_name().into_inner();
                match object_depth {
                    // An empty <object/> cannot carry a mesh child.
                    None => {
                        if name == "object" && stack_ends_with(&stack, &["resources"]) {
                            return Err(no_mesh_error());
                        }
                    }
                    Some(_) => match name {
                        "vertex" => vertices.push(vertex_position(&start, matrix)?),
                        "triangle" => triangles.push(triangle_indices(&start)?),
                        _ => {}
                    },
                }
            }
            Event::End(_) => {
                if object_depth.is_some_and(|depth| stack.len() == depth + 1) {
                    // This closes the tracked object: stop scanning, like the Python `find`
                    // taking the first match rather than merging every object's mesh.
                    break;
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if object_depth.is_none() || !has_mesh {
        return Err(no_mesh_error());
    }
    triangles
        .into_iter()
        .map(|[a, b, c]| {
            let lookup = |index: usize| {
                vertices.get(index).copied().ok_or_else(|| {
                    invalid_data(format!(
                        "triangle references vertex {index}, but only {} vertices exist",
                        vertices.len()
                    ))
                })
            };
            Ok([lookup(a)?, lookup(b)?, lookup(c)?])
        })
        .collect()
}

fn no_mesh_error() -> io::Error {
    invalid_data("3MF has no inline or component mesh to export")
}

fn read_event<'a>(
    reader: &mut Reader<&'a [u8]>,
    part: &str,
) -> io::Result<quick_xml::events::Event<'a>> {
    reader
        .read_event()
        .map_err(|error| invalid_data(format!("{part} is not well-formed XML: {error}")))
}

/// Reads a `component` element's production `p:path` (the mesh part zip entry) and its optional
/// `transform`. The production extension declares its attributes with a literal `p:` prefix;
/// quick-xml preserves those prefixes verbatim, which is exactly why it was chosen over xmltree.
fn component_source(start: &BytesStart<'_>) -> io::Result<ComponentSource> {
    let mut path = None;
    let mut transform = None;
    for attribute in start.attributes() {
        let attribute =
            attribute.map_err(|error| invalid_data(format!("component attribute: {error}")))?;
        match attribute.key.as_ref() {
            "p:path" => {
                let value = attribute
                    .normalized_value(XmlVersion::Implicit1_0)
                    .map_err(|error| invalid_data(format!("component path: {error}")))?;
                path = Some(value.into_owned());
            }
            "transform" => transform = Some(attribute.value.into_owned()),
            _ => {}
        }
    }
    let Some(path) = path else {
        return Err(invalid_data("source component has no production path"));
    };
    if path.is_empty() {
        return Err(invalid_data("source component has no production path"));
    }
    Ok(ComponentSource {
        path: path.trim_start_matches('/').to_owned(),
        transform,
    })
}

fn vertex_position(
    start: &BytesStart<'_>,
    matrix: &[[f64; 3]; 4],
) -> io::Result<(f64, f64, f64)> {
    let mut axes = [None::<f64>; 3];
    for attribute in start.attributes() {
        let attribute =
            attribute.map_err(|error| invalid_data(format!("vertex attribute: {error}")))?;
        let axis = match attribute.key.as_ref() {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            _ => continue,
        };
        axes[axis] = Some(parse_f64(
            &attribute.value,
            "vertex coordinate",
        )?);
    }
    let [Some(x), Some(y), Some(z)] = axes else {
        return Err(invalid_data("vertex is missing an x/y/z coordinate"));
    };
    Ok(apply_matrix(matrix, (x, y, z)))
}

fn triangle_indices(start: &BytesStart<'_>) -> io::Result<[usize; 3]> {
    let mut indices = [None::<usize>; 3];
    for attribute in start.attributes() {
        let attribute =
            attribute.map_err(|error| invalid_data(format!("triangle attribute: {error}")))?;
        let slot = match attribute.key.as_ref() {
            "v1" => 0,
            "v2" => 1,
            "v3" => 2,
            _ => continue,
        };
        indices[slot] = Some(
            attribute
                .value
                .trim()
                .parse::<usize>()
                .map_err(|_| invalid_data(format!("triangle index is not a number: {:?}", attribute.value)))?,
        );
    }
    let [Some(v1), Some(v2), Some(v3)] = indices else {
        return Err(invalid_data("triangle is missing a v1/v2/v3 index"));
    };
    Ok([v1, v2, v3])
}

fn parse_f64(value: &str, context: &str) -> io::Result<f64> {
    value
        .trim()
        .parse::<f64>()
        .map_err(|_| invalid_data(format!("{context} is not a number: {value:?}")))
}

fn zip_entry_text(archive: &mut ZipArchive<File>, name: &str) -> io::Result<String> {
    let mut entry = archive.by_name(name).map_err(|error| match error {
        zip::result::ZipError::FileNotFound => {
            invalid_data(format!("3MF does not contain the entry {name:?}"))
        }
        error => invalid_data(format!("cannot read {name:?} from the 3MF: {error}")),
    })?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn stack_ends_with(stack: &[String], suffix: &[&str]) -> bool {
    stack.len() >= suffix.len()
        && stack[stack.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(entry, expected)| entry == expected)
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests;

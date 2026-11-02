//! Every part of the source object in the object frame (design 2026-11-02 §4.1, spec 01 §1.1):
//! the source build item's full transform (rotation, scale, mirror) times each component's,
//! centred on the XY bounding-box centre of all normal parts and dropped so the lowest normal
//! part touches z = 0.
//!
//! The old first-component-only reader (`embedded_mesh_triangles`) lost the goat's pompom and
//! the item rotation (PM §0.5, §9.5): the pompom is a second normal part, so the silhouette and
//! the height (41.45 vs 44.74 mm) were wrong.

use std::{collections::BTreeMap, fs::File, io, path::Path};

use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use super::model_file::{ModelFile, RawMesh, parse_model_file};
use super::part_types::{MODEL_SETTINGS_ENTRY, PartKind, PartKinds, part_kinds};
use super::{CORE_MODEL_ENTRY, Matrix, apply_matrix, invalid_data, zip_entry_text};

/// Components nest at most this deep; deeper (or cyclic) references are rejected instead of
/// recursing without bound.
const MAX_COMPONENT_DEPTH: usize = 16;

/// One volume of the object.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Part {
    pub kind: PartKind,
    /// The 3MF object id of the volume's mesh (Bambu's `<part id>`); 0 for generated parts.
    pub source_id: u32,
    /// Object-frame vertices, mm.
    pub vertices: Vec<[f64; 3]>,
    /// Vertex indices in stored order: paint codes address triangles by index and their child
    /// geometry by vertex order (spec 01 §2.3), so the order is never changed.
    pub triangles: Vec<[u32; 3]>,
    /// The transform mirrors (negative determinant): the stored winding then faces inward, and
    /// every orientation test flips (spec 01 §2.6 step 2).
    pub mirrored: bool,
    /// Support paint codes (`paint_supports`) by triangle index.
    pub paint_supports: BTreeMap<u32, String>,
    /// Colour paint codes (`paint_color`) by triangle index.
    pub paint_color: BTreeMap<u32, String>,
}

impl Part {
    /// The three corners of triangle `index`, object frame.
    pub fn corners(&self, index: usize) -> [[f64; 3]; 3] {
        self.triangles[index].map(|vertex| self.vertices[vertex as usize])
    }

    /// `(min, max)` over all vertices; `None` for a part without vertices.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        bounds_of(self.vertices.iter())
    }
}

/// The source object: all parts in the object frame plus where that frame sits on the source
/// plate.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ObjectMesh {
    /// In volume order (component order), which is the order the slicer applies negative
    /// parts in (spec 01 §1.4).
    pub parts: Vec<Part>,
    /// Source-plate position of the object-frame origin: the normal parts' XY bbox centre and
    /// their lowest z. A world point is `frame + frame_origin` (the frame is not rotated).
    pub frame_origin: [f64; 3],
}

impl ObjectMesh {
    /// Builds the object frame from parts given in source-plate coordinates.
    pub fn from_plate_parts(mut parts: Vec<Part>) -> Result<ObjectMesh, String> {
        let normal_vertices = parts
            .iter()
            .filter(|part| part.kind == PartKind::Normal)
            .flat_map(|part| part.vertices.iter());
        let Some((min, max)) = bounds_of(normal_vertices) else {
            return Err("the object has no normal part with vertices".to_owned());
        };
        let origin = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, min[2]];
        for part in &mut parts {
            for vertex in &mut part.vertices {
                *vertex = [
                    vertex[0] - origin[0],
                    vertex[1] - origin[1],
                    vertex[2] - origin[2],
                ];
            }
        }
        Ok(ObjectMesh {
            parts,
            frame_origin: origin,
        })
    }

    pub fn normal_parts(&self) -> impl Iterator<Item = &Part> {
        self.parts
            .iter()
            .filter(|part| part.kind == PartKind::Normal)
    }

    /// `(min, max)` of the normal parts; the frame puts the XY centre at 0 and min z at 0.
    pub fn normal_bounds(&self) -> ([f64; 3], [f64; 3]) {
        bounds_of(self.normal_parts().flat_map(|part| part.vertices.iter()))
            .unwrap_or(([0.0; 3], [0.0; 3]))
    }

    /// Height of the normal parts above the bed.
    pub fn height(&self) -> f64 {
        let (min, max) = self.normal_bounds();
        max[2] - min[2]
    }

    /// The same object turned by `degrees` (counter-clockwise, seen from above) about the
    /// frame's Z axis, i.e. the plate-aligned frame of rotation class `degrees` (design §4.1).
    /// The origin stays the unrotated bbox centre, so placing a rotated print is a pure
    /// translation; the slicer's own frame for that rotation is centred on the rotated bbox
    /// (`normal_bounds` of the result), which callers that emulate the slicer must use.
    pub fn rotated(&self, degrees: f64) -> ObjectMesh {
        let (sin, cos) = sin_cos_degrees(degrees);
        let mut rotated = self.clone();
        for part in &mut rotated.parts {
            for vertex in &mut part.vertices {
                let [x, y, z] = *vertex;
                *vertex = [x * cos - y * sin, x * sin + y * cos, z];
            }
        }
        rotated
    }
}

/// `sin` and `cos` of an angle in degrees; exact at multiples of 90 so 0/90/180/270 frames do
/// not pick up 1e-16 shears (libm keeps the rest identical across platforms).
pub fn sin_cos_degrees(degrees: f64) -> (f64, f64) {
    let turns = degrees / 90.0;
    if turns == libm::round(turns) {
        return match (libm::round(turns) as i64).rem_euclid(4) {
            0 => (0.0, 1.0),
            1 => (1.0, 0.0),
            2 => (0.0, -1.0),
            _ => (-1.0, 0.0),
        };
    }
    let radians = degrees.to_radians();
    (libm::sin(radians), libm::cos(radians))
}

/// Reads the first build item's object with every volume (normal, negative, modifier,
/// enforcer, blocker), its full transform and its paint, in the object frame.
pub fn load_object_mesh(source_3mf: &Path) -> io::Result<ObjectMesh> {
    let mut archive = ZipArchive::new(File::open(source_3mf)?).map_err(|error| {
        invalid_data(format!(
            "{source_3mf:?} is not a readable 3MF archive: {error}"
        ))
    })?;
    let core_text = zip_entry_text(&mut archive, CORE_MODEL_ENTRY)?;
    let mut files = BTreeMap::new();
    files.insert(
        CORE_MODEL_ENTRY.to_owned(),
        parse_model_file(&core_text, CORE_MODEL_ENTRY)?,
    );
    let item = files[CORE_MODEL_ENTRY]
        .items
        .first()
        .cloned()
        .ok_or_else(|| invalid_data("3MF build has no item"))?;
    let config = zip_entry_text(&mut archive, MODEL_SETTINGS_ENTRY).ok();
    let kinds = part_kinds(config.as_deref(), item.object_id)?;
    let mut collector = PartCollector {
        archive: &mut archive,
        files,
        kinds,
        parts: Vec::new(),
    };
    collector.object(CORE_MODEL_ENTRY, item.object_id, &item.transform, None, 0)?;
    ObjectMesh::from_plate_parts(collector.parts).map_err(invalid_data)
}

/// Walks the component tree of the build item, loading model parts on first use.
struct PartCollector<'a> {
    archive: &'a mut ZipArchive<File>,
    files: BTreeMap<String, ModelFile>,
    kinds: PartKinds,
    parts: Vec<Part>,
}

impl PartCollector<'_> {
    /// Adds object `id` of `file` placed by `transform`. `top_id` is the object id the config
    /// names the part by: the first-level component's object, or the object itself.
    fn object(
        &mut self,
        file: &str,
        id: u32,
        transform: &Matrix,
        top_id: Option<u32>,
        depth: usize,
    ) -> io::Result<()> {
        if depth > MAX_COMPONENT_DEPTH {
            return Err(invalid_data(
                "3MF components nest too deep (or form a cycle)",
            ));
        }
        self.load(file)?;
        let object = self.files[file]
            .objects
            .get(&id)
            .ok_or_else(|| invalid_data(format!("{file} has no object {id}")))?;
        if let Some(mesh) = &object.mesh {
            push_mesh_parts(
                &mut self.parts,
                &self.kinds,
                mesh,
                transform,
                top_id.unwrap_or(id),
            );
        }
        let components = object.components.clone();
        for component in &components {
            let component_file = component.path.clone().unwrap_or_else(|| file.to_owned());
            let placed = compose(&component.transform, transform);
            let part_id = top_id.unwrap_or(component.object_id);
            self.object(
                &component_file,
                component.object_id,
                &placed,
                Some(part_id),
                depth + 1,
            )?;
        }
        Ok(())
    }

    fn load(&mut self, file: &str) -> io::Result<()> {
        if !self.files.contains_key(file) {
            let text = zip_entry_text(self.archive, file)?;
            self.files
                .insert(file.to_owned(), parse_model_file(&text, file)?);
        }
        Ok(())
    }
}

/// One mesh becomes one part, or several when the config splits it by triangle ranges.
fn push_mesh_parts(
    parts: &mut Vec<Part>,
    kinds: &PartKinds,
    mesh: &RawMesh,
    transform: &Matrix,
    part_id: u32,
) {
    let vertices: Vec<[f64; 3]> = mesh
        .vertices
        .iter()
        .map(|&[x, y, z]| {
            let (x, y, z) = apply_matrix(transform, (x, y, z));
            [x, y, z]
        })
        .collect();
    let mirrored = determinant(transform) < 0.0;
    if kinds.by_range.is_empty() {
        parts.push(Part {
            kind: kinds.kind_of(part_id),
            source_id: part_id,
            vertices,
            triangles: mesh.triangles.clone(),
            mirrored,
            paint_supports: mesh.paint_supports.clone(),
            paint_color: mesh.paint_color.clone(),
        });
        return;
    }
    for &(first, last, kind) in &kinds.by_range {
        parts.push(range_part(
            mesh, &vertices, first, last, kind, part_id, mirrored,
        ));
    }
}

/// The triangles `first..=last` of a single-mesh object as one part; paint is re-indexed to
/// the part's own triangle order.
fn range_part(
    mesh: &RawMesh,
    vertices: &[[f64; 3]],
    first: u32,
    last: u32,
    kind: PartKind,
    part_id: u32,
    mirrored: bool,
) -> Part {
    let end = (last as usize + 1).min(mesh.triangles.len());
    let start = (first as usize).min(end);
    let reindex = |codes: &BTreeMap<u32, String>| {
        codes
            .range(start as u32..end as u32)
            .map(|(index, code)| (index - start as u32, code.clone()))
            .collect()
    };
    Part {
        kind,
        source_id: part_id,
        vertices: vertices.to_vec(),
        triangles: mesh.triangles[start..end].to_vec(),
        mirrored,
        paint_supports: reindex(&mesh.paint_supports),
        paint_color: reindex(&mesh.paint_color),
    }
}

/// `first` applied, then `then` (3MF row vectors: p' = p * first * then).
pub fn compose(first: &Matrix, then: &Matrix) -> Matrix {
    let mut result = [[0.0; 3]; 4];
    for (row, out) in result.iter_mut().enumerate() {
        for (column, value) in out.iter_mut().enumerate() {
            let linear: f64 = (0..3).map(|k| first[row][k] * then[k][column]).sum();
            *value = linear + if row == 3 { then[3][column] } else { 0.0 };
        }
    }
    result
}

/// Determinant of the linear part; negative means the transform mirrors.
pub fn determinant(matrix: &Matrix) -> f64 {
    let [a, b, c] = [matrix[0], matrix[1], matrix[2]];
    a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
        + a[2] * (b[0] * c[1] - b[1] * c[0])
}

fn bounds_of<'a>(vertices: impl Iterator<Item = &'a [f64; 3]>) -> Option<([f64; 3], [f64; 3])> {
    let mut bounds: Option<([f64; 3], [f64; 3])> = None;
    for vertex in vertices {
        let (min, max) = bounds.get_or_insert((*vertex, *vertex));
        for axis in 0..3 {
            min[axis] = min[axis].min(vertex[axis]);
            max[axis] = max[axis].max(vertex[axis]);
        }
    }
    bounds
}

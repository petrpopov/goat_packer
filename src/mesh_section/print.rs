//! The body kinds of an `ObjectPrint` for one rotation class (design §4.2): body bands,
//! silhouette, part hulls and part boxes in the plate-aligned frame of that rotation.

use super::hull::{part_boxes, part_hulls};
use super::slab::{BodyBands, body_bands, silhouette};
use crate::embedded_mesh::ObjectMesh;
use crate::object_print::ObjectPrint;

/// The mesh turned by `rotation_deg` about the frame centre (the unrotated normal-part bbox
/// centre), so placing the print is a pure translation. Landing, brim and supports come from
/// their own models; they stay empty here.
pub fn body_print(mesh: &ObjectMesh, rotation_deg: f64, bands: &BodyBands) -> ObjectPrint {
    let rotated = mesh.rotated(rotation_deg);
    let body = body_bands(&rotated, bands);
    ObjectPrint {
        rotation_deg,
        silhouette: silhouette(&body),
        part_hulls: part_hulls(&rotated),
        part_boxes: part_boxes(&rotated),
        body,
        ..ObjectPrint::default()
    }
}

//! Triangle ∩ horizontal plane (spec 01 §1.3 step 1): one directed segment per crossing
//! triangle, with topological keys for its ends so neighbouring triangles chain exactly.

use crate::embedded_mesh::Part;

/// Where a segment end lies: on a mesh vertex exactly at the plane, or inside an edge
/// (vertex indices, smaller first). Two triangles sharing the edge produce the same key.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EndKey {
    Vertex(u32),
    Edge(u32, u32),
}

/// A directed piece of the section: the solid lies on its left, so outer loops run
/// counter-clockwise and holes clockwise.
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub from: (EndKey, [f64; 2]),
    pub to: (EndKey, [f64; 2]),
}

/// All segments of `part` at height `z`, in triangle order.
///
/// A vertex counts as "above" when its z is at or over the plane. Walking the triangle in
/// its winding order, the segment runs from the edge that goes down through the plane to the
/// edge that comes back up. With "at the plane" counted as above this reproduces the
/// slicer's rules: a vertex on the plane is used as is, an edge lying in the plane counts
/// only when the third vertex is below (the plane catches the top faces of a horizontal
/// surface, not its bottom faces), and horizontal triangles give nothing.
pub fn plane_segments(part: &Part, z: f64) -> Vec<Segment> {
    let mut segments = Vec::new();
    for triangle in &part.triangles {
        if let Some(segment) = triangle_segment(part, triangle, z) {
            segments.push(segment);
        }
    }
    segments
}

fn triangle_segment(part: &Part, triangle: &[u32; 3], z: f64) -> Option<Segment> {
    let corners = triangle.map(|index| part.vertices[index as usize]);
    let above = corners.map(|corner| corner[2] >= z);
    if above.iter().all(|&flag| flag) || above.iter().all(|&flag| !flag) {
        return None;
    }
    let mut down = None;
    let mut up = None;
    for side in 0..3 {
        let next = (side + 1) % 3;
        if above[side] && !above[next] {
            down = Some(edge_point(part, triangle[side], triangle[next], z));
        }
        if !above[side] && above[next] {
            up = Some(edge_point(part, triangle[side], triangle[next], z));
        }
    }
    let (from, to) = (down?, up?);
    if from.0 == to.0 {
        // The plane only touches one vertex.
        return None;
    }
    // A mirroring transform turns the stored winding inside out.
    Some(if part.mirrored {
        Segment { from: to, to: from }
    } else {
        Segment { from, to }
    })
}

/// The crossing of edge (a, b) with the plane. A vertex at the plane is returned as is;
/// otherwise the point is interpolated from the lower-indexed end so both triangles sharing
/// the edge get identical bits (spec 01 §1.3: "vertices ordered by index").
fn edge_point(part: &Part, a: u32, b: u32, z: f64) -> (EndKey, [f64; 2]) {
    for vertex in [a, b] {
        let corner = part.vertices[vertex as usize];
        if corner[2] == z {
            return (EndKey::Vertex(vertex), [corner[0], corner[1]]);
        }
    }
    let (low, high) = if a < b { (a, b) } else { (b, a) };
    let p = part.vertices[low as usize];
    let q = part.vertices[high as usize];
    let t = (z - p[2]) / (q[2] - p[2]);
    (
        EndKey::Edge(low, high),
        [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t],
    )
}

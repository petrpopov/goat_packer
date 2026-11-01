//! The mesh-projection footprint: the top-down silhouette of a binary STL.

use crate::footprint::Footprint;
use crate::geometry::{Point, closed_loop, drop_specks, sort_largest_first};
use geo::{Coord, Polygon, algorithm::bool_ops::unary_union};
use std::{fs, io, path::Path};

type Triangle = [[f32; 3]; 3];

/// Size of the binary STL header plus the triangle count field.
const STL_HEADER_BYTES: usize = 84;
/// One binary STL triangle: normal (12) + three vertices (36) + attribute bytes (2).
const STL_TRIANGLE_BYTES: usize = 50;
const STL_NORMAL_BYTES: usize = 12;
const STL_VERTEX_BYTES: usize = 12;

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn read_f32(bytes: &[u8]) -> f32 {
    f32::from_le_bytes(bytes.try_into().expect("four bytes"))
}

/// Triangles of a binary STL with the per-axis `(mins, maxs)` of all their vertices.
fn read_binary_stl(data: &[u8]) -> io::Result<(Vec<Triangle>, [f64; 3], [f64; 3])> {
    if data.len() < STL_HEADER_BYTES {
        return Err(invalid_data("STL header is truncated"));
    }
    let triangle_count = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
    let expected = STL_HEADER_BYTES
        .checked_add(
            triangle_count
                .checked_mul(STL_TRIANGLE_BYTES)
                .ok_or_else(|| invalid_data("STL triangle count overflows"))?,
        )
        .ok_or_else(|| invalid_data("STL size overflows"))?;
    if data.len() != expected {
        return Err(invalid_data(format!(
            "binary STL has {} bytes; expected {expected}",
            data.len()
        )));
    }

    let mut triangles = Vec::with_capacity(triangle_count);
    let mut mins = [f64::INFINITY; 3];
    let mut maxs = [f64::NEG_INFINITY; 3];
    for index in 0..triangle_count {
        let base = STL_HEADER_BYTES + index * STL_TRIANGLE_BYTES + STL_NORMAL_BYTES;
        let mut triangle = [[0.0f32; 3]; 3];
        for (vertex_index, vertex) in triangle.iter_mut().enumerate() {
            let offset = base + vertex_index * STL_VERTEX_BYTES;
            for axis in 0..3 {
                vertex[axis] = read_f32(&data[offset + axis * 4..offset + axis * 4 + 4]);
                if !vertex[axis].is_finite() {
                    return Err(invalid_data("STL contains non-finite coordinates"));
                }
                mins[axis] = mins[axis].min(f64::from(vertex[axis]));
                maxs[axis] = maxs[axis].max(f64::from(vertex[axis]));
            }
        }
        triangles.push(triangle);
    }
    Ok((triangles, mins, maxs))
}

/// Every triangle's XY projection as a counter-clockwise polygon; degenerate triangles
/// (zero XY area, e.g. vertical walls) add nothing to a union and are skipped.
fn projected_polygons(triangles: &[Triangle]) -> Vec<Polygon> {
    let mut projections = Vec::with_capacity(triangles.len());
    for triangle in triangles {
        let [a, b, c] = triangle.map(|vertex| Coord {
            x: f64::from(vertex[0]),
            y: f64::from(vertex[1]),
        });
        // STL gives no orientation guarantee between meshes; normalize to counter-clockwise
        // so the union's winding-based fill rule adds every patch instead of subtracting.
        let ordered = if (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x) >= 0.0 {
            [a, b, c]
        } else {
            [a, c, b]
        };
        if ordered[0] == ordered[1] || ordered[1] == ordered[2] {
            continue;
        }
        projections.push(Polygon::new(
            geo::LineString::from(ordered.to_vec()),
            Vec::new(),
        ));
    }
    projections
}

pub fn extract_footprint(path: &Path) -> io::Result<Footprint> {
    let (triangles, mins, maxs) = read_binary_stl(&fs::read(path)?)?;
    let triangle_count = triangles.len();

    let z_tolerance = ((maxs[2] - mins[2]) * 1e-6).max(1e-5);
    let center = ((mins[0] + maxs[0]) * 0.5, (mins[1] + maxs[1]) * 0.5);

    // The footprint is the union of every triangle's XY projection — the silhouette seen from
    // above. Two earlier designs were both less honest: an edge-parity walk over the flat
    // minimum-Z triangles (which real meshes break with T-junctions and self-touching contact
    // strips, failing as "bottom boundary is non-manifold" on a rounded-belly model), and even
    // a correct flat-bottom union still ignored overhangs — a lying animal whose head and ears
    // reach past its resting strip collided with neighbours that the sole said were clear.
    // The full projection also covers support bases, which stand under overhangs.
    let projections = projected_polygons(&triangles);
    if projections.is_empty() {
        return Err(invalid_data(format!(
            "no footprint could be built: all {} triangles project to degenerate (zero-area) \
             XY shapes (z range {:.3}..{:.3}, z tolerance {:.1e})",
            triangles.len(),
            mins[2],
            maxs[2],
            z_tolerance
        )));
    }
    let union = unary_union(&projections);

    let mut loops: Vec<Vec<Point>> = Vec::new();
    for polygon in &union.0 {
        loops.extend(closed_loop(polygon.exterior().coords(), center));
        for interior in polygon.interiors() {
            loops.extend(closed_loop(interior.coords(), center));
        }
    }
    if loops.is_empty() {
        return Err(invalid_data(format!(
            "no footprint could be built: {} triangles projected, but their union is empty \
             (all are degenerate slivers)",
            projections.len()
        )));
    }
    drop_specks(&mut loops);
    if loops.is_empty() {
        return Err(invalid_data(
            "STL has no printable footprint after removing degenerate rings",
        ));
    }

    sort_largest_first(&mut loops);
    Ok(Footprint {
        // The mesh projection branch: everything is body — supports, if any, sit inside it.
        body_loops: loops.clone(),
        support_landing_loops: Vec::new(),
        bed_loops: None,
        layer_zones: Vec::new(),
        loops,
        width: maxs[0] - mins[0],
        height: maxs[1] - mins[1],
        min_z: mins[2],
        max_z: maxs[2],
        triangle_count,
    })
}

//! The collision STL the packer wrote next to the project.

use std::{fs, path::Path};

use crate::Failure;

/// XY bbox center over ALL STL vertices — the `stl_bbox_center` the Python feeds the item
/// translation expectation. Binary STL only: the collision STL is always produced by the
/// packer in binary form (80 zero bytes, LE u32 count, 50-byte records).
pub fn stl_xy_center(path: &Path) -> Result<(f64, f64), Failure> {
    let data = fs::read(path).map_err(|error| {
        Failure::Error(format!("failed to read STL {}: {error}", path.display()))
    })?;
    if data.len() < 84 {
        return Err(Failure::Error("STL header is truncated".to_owned()));
    }
    let triangle_count = u32::from_le_bytes(data[80..84].try_into().expect("four bytes")) as usize;
    let expected_len = 84usize
        + triangle_count
            .checked_mul(50)
            .ok_or_else(|| Failure::Error("STL triangle count overflows".to_owned()))?;
    if data.len() != expected_len {
        return Err(Failure::Error(format!(
            "binary STL has {} bytes; expected {expected_len}",
            data.len()
        )));
    }
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for index in 0..triangle_count {
        // 12-byte facet normal header precedes the three vertices in each record.
        let base = 84 + index * 50 + 12;
        for vertex in 0..3 {
            let offset = base + vertex * 12;
            let x = f64::from(f32::from_le_bytes(
                data[offset..offset + 4].try_into().expect("four bytes"),
            ));
            let y = f64::from(f32::from_le_bytes(
                data[offset + 4..offset + 8].try_into().expect("four bytes"),
            ));
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    Ok(((min_x + max_x) * 0.5, (min_y + max_y) * 0.5))
}

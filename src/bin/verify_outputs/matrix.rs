//! 3MF transforms: 4x3 row-vector matrices.

use crate::{Failure, assertion};

/// 4x3 row-major 3MF transform (last row = translation).
pub type Matrix = [[f64; 3]; 4];

pub const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, 0.0],
];

/// Parse a 3MF transform into a 4x3 row-major matrix (last row = translation).
///
/// 3MF uses row vectors: p' = p·M, i.e. p'_j = sum_i p_i * M[i][j] + M[3][j].
pub fn parse_transform(text: Option<&str>) -> Result<Matrix, Failure> {
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return Ok(IDENTITY);
    };
    let values = text
        .split_whitespace()
        .map(|value| {
            value.parse::<f64>().map_err(|error| {
                Failure::Error(format!("invalid number in 3MF transform: {error}"))
            })
        })
        .collect::<Result<Vec<f64>, Failure>>()?;
    if values.len() != 12 {
        return Err(assertion(format!(
            "3MF transform is not 12 numbers: {text:?}"
        )));
    }
    Ok([
        [values[0], values[1], values[2]],
        [values[3], values[4], values[5]],
        [values[6], values[7], values[8]],
        [values[9], values[10], values[11]],
    ])
}

pub fn apply_matrix(matrix: &Matrix, point: (f64, f64, f64)) -> (f64, f64, f64) {
    let (x, y, z) = point;
    (
        x * matrix[0][0] + y * matrix[1][0] + z * matrix[2][0] + matrix[3][0],
        x * matrix[0][1] + y * matrix[1][1] + z * matrix[2][1] + matrix[3][1],
        x * matrix[0][2] + y * matrix[1][2] + z * matrix[2][2] + matrix[3][2],
    )
}

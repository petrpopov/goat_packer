//! The build-item transform of a clone of the source object: the source item's own transform
//! (rotation, scale, mirror, z offset), then a Z rotation about the object's frame centre, then
//! the move that puts that centre at the placement. The legacy path built clones from the
//! rotation and a translation only, which drops a source rotation (design §4.1).

use crate::embedded_mesh::sin_cos_degrees;

/// The source item of a 3MF and where its object frame sits on the source plate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemFrame {
    /// The source build item's transform as 3MF writes it: the 3x3 part column by column
    /// (`x' = v0 x + v3 y + v6 z + v9`, `y' = v1 x + v4 y + v7 z + v10`, `z' = v2 x + v5 y +
    /// v8 z + v11`), then the translation.
    pub source: [f64; 12],
    /// XY of the object frame's origin on the source plate (`ObjectMesh::frame_origin`): the
    /// normal parts' bbox centre after the source transform.
    pub origin: (f64, f64),
}

impl ItemFrame {
    /// The transform of a clone turned by `rotation_deg` about the frame origin and placed so
    /// that the origin lands on `at`. The source z offset is kept: clones stay at the on-bed
    /// height the source sliced at.
    pub fn clone_transform(&self, rotation_deg: f64, at: (f64, f64)) -> [f64; 12] {
        let (sin, cos) = sin_cos_degrees(rotation_deg);
        let v = &self.source;
        // Rows of the source's 3x3 part, then the Z rotation applied on the left.
        let rows = [[v[0], v[3], v[6]], [v[1], v[4], v[7]], [v[2], v[5], v[8]]];
        let turned = [
            std::array::from_fn(|c| cos * rows[0][c] - sin * rows[1][c]),
            std::array::from_fn(|c| sin * rows[0][c] + cos * rows[1][c]),
            rows[2],
        ];
        let (dx, dy) = (v[9] - self.origin.0, v[10] - self.origin.1);
        let (x, y) = (at.0 + cos * dx - sin * dy, at.1 + sin * dx + cos * dy);
        [
            turned[0][0], turned[1][0], turned[2][0],
            turned[0][1], turned[1][1], turned[2][1],
            turned[0][2], turned[1][2], turned[2][2],
            x, y, v[11],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies a 3MF transform to a point.
    fn apply(m: &[f64; 12], p: [f64; 3]) -> [f64; 3] {
        [
            m[0] * p[0] + m[3] * p[1] + m[6] * p[2] + m[9],
            m[1] * p[0] + m[4] * p[1] + m[7] * p[2] + m[10],
            m[2] * p[0] + m[5] * p[1] + m[8] * p[2] + m[11],
        ]
    }

    #[test]
    fn the_frame_origin_lands_on_the_placement_for_any_source_and_rotation() {
        // A source turned 30 degrees, then moved: its frame origin is wherever it put the
        // local bbox centre (4, 1).
        let (s, c) = (0.5_f64, 3.0_f64.sqrt() / 2.0);
        let source = [c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0, 100.0, 50.0, 0.4];
        let centre = apply(&source, [4.0, 1.0, 0.0]);
        let frame = ItemFrame { source, origin: (centre[0], centre[1]) };
        for rotation in [0.0, 90.0, 180.0, 37.0] {
            let moved = apply(&frame.clone_transform(rotation, (10.0, 20.0)), [4.0, 1.0, 0.0]);
            assert!((moved[0] - 10.0).abs() < 1e-9 && (moved[1] - 20.0).abs() < 1e-9, "{moved:?}");
            assert!((moved[2] - 0.4).abs() < 1e-12);
        }
    }

    #[test]
    fn a_half_turn_mirrors_every_point_through_the_origin() {
        let source = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 5.0, 6.0, 0.0];
        let frame = ItemFrame { source, origin: (7.0, 8.0) };
        let moved = apply(&frame.clone_transform(180.0, (0.0, 0.0)), [3.0, 2.0, 1.0]);
        // The source puts local (3, 2) at (8, 8); half a turn about (7, 8) sends it to (6, 8).
        assert_eq!(moved, [-1.0, 0.0, 1.0]);
    }
}

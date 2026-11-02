//! The probe plate: a cube generated here, placed off the draft's exclusions.

use goat_packer::orca_project;
use serde_json::Value;

/// The probe is a cube of this edge, mm, and this tall.
pub const PROBE_CUBE_MM: f64 = 20.0;
pub const PROBE_CUBE_HEIGHT_MM: f64 = 10.0;

/// Lower-left corner for the probe cube: centred in the draft's printable area, off its
/// exclusion rectangles (`orca_project::free_spot`).
pub fn probe_origin(draft: &Value, size: f64) -> (f64, f64) {
    let area: [f64; 4] =
        serde_json::from_value(draft.get("printable_area").cloned().unwrap_or_default())
            .unwrap_or([0.0, 0.0, size, size]);
    let exclusions: Vec<[f64; 4]> = serde_json::from_value(
        draft
            .get("exclusion_rectangles")
            .cloned()
            .unwrap_or_default(),
    )
    .unwrap_or_default();
    orca_project::free_spot(area, &exclusions, (size, size))
}

/// Closed box `[0,w]×[0,d]×[0,h]`, 12 triangles wound counter-clockwise seen from outside.
pub fn cube_triangles(w: f64, d: f64, h: f64) -> Vec<[(f64, f64, f64); 3]> {
    let v = |i: usize| {
        let (x, y, z) = (i & 1, (i >> 1) & 1, (i >> 2) & 1);
        (x as f64 * w, y as f64 * d, z as f64 * h)
    };
    // Faces as corner quads (outward CCW): -z, +z, -y, +y, -x, +x.
    let quads = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    quads
        .iter()
        .flat_map(|q| [[v(q[0]), v(q[1]), v(q[2])], [v(q[0]), v(q[2]), v(q[3])]])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn probe_cube_sits_centred_and_off_exclusions() {
        let plain = json!({"printable_area": [0.0, 0.0, 256.0, 256.0], "exclusion_rectangles": []});
        assert_eq!(probe_origin(&plain, 20.0), (118.0, 118.0));
        let blocked = json!({"printable_area": [0.0, 0.0, 256.0, 256.0], "exclusion_rectangles": [[100.0, 100.0, 156.0, 156.0]]});
        let (x, y) = probe_origin(&blocked, 20.0);
        assert!(
            x + 20.0 <= 100.0 || x >= 156.0 || y + 20.0 <= 100.0 || y >= 156.0,
            "({x}, {y})"
        );
        let h2d = json!({"printable_area": [25.0, 0.0, 325.0, 320.0]});
        assert_eq!(probe_origin(&h2d, 20.0), (165.0, 150.0));
    }

    #[test]
    fn probe_cube_is_closed_and_wound_outwards() {
        let triangles = cube_triangles(20.0, 20.0, 10.0);
        assert_eq!(triangles.len(), 12);
        // Divergence theorem: the signed volume is +w·d·h only for outward (CCW) winding.
        let volume: f64 = triangles
            .iter()
            .map(|[a, b, c]| {
                (a.0 * (b.1 * c.2 - b.2 * c.1) - a.1 * (b.0 * c.2 - b.2 * c.0)
                    + a.2 * (b.0 * c.1 - b.1 * c.0))
                    / 6.0
            })
            .sum();
        assert!((volume - 4000.0).abs() < 1e-9, "{volume}");
        let max = triangles
            .iter()
            .flatten()
            .fold((0.0_f64, 0.0_f64, 0.0_f64), |m, p| {
                (m.0.max(p.0), m.1.max(p.1), m.2.max(p.2))
            });
        assert_eq!(max, (20.0, 20.0, 10.0));
    }
}

//! Tower formulas every build shares (spec 04 §0, Appendix A).

use crate::first_layer::bead_spacing;

/// Minimum tower depth by tower height: linear between `(height, depth)` points, clamped at
/// both ends (B `GCode/WipeTower.cpp:1586-1625`; the GUI estimates of every build use it).
const MIN_DEPTH_BY_HEIGHT: [(f64, f64); 4] =
    [(5.0, 5.0), (100.0, 20.0), (250.0, 40.0), (350.0, 60.0)];

/// Auto tower brim: 8 % of the tower height, at most 8 mm (B `:1627-1630`).
const AUTO_BRIM_PER_HEIGHT: f64 = 0.08;
const AUTO_BRIM_MAX_MM: f64 = 8.0;

/// Cross-section of a bead of width `width` and height `height`, mm²: what turns a purge
/// volume into line length (04 §0 symbol A(h)).
pub fn bead_area(width: f64, height: f64) -> f64 {
    height * bead_spacing(width, height)
}

pub fn min_depth_for_height(height: f64) -> f64 {
    let (first, last) = (MIN_DEPTH_BY_HEIGHT[0], MIN_DEPTH_BY_HEIGHT[3]);
    if height <= first.0 {
        return first.1;
    }
    for pair in MIN_DEPTH_BY_HEIGHT.windows(2) {
        let ((h0, d0), (h1, d1)) = (pair[0], pair[1]);
        if height < h1 {
            return d0 + (height - h0) / (h1 - h0) * (d1 - d0);
        }
    }
    last.1
}

pub fn auto_brim_width(tower_height: f64) -> f64 {
    (AUTO_BRIM_PER_HEIGHT * tower_height).min(AUTO_BRIM_MAX_MM)
}

/// Loops of pitch `spacing` that fit a brim of `width`: the outermost loop centre may pass the
/// width by half a pitch (04 §4.6).
pub fn brim_loops(width: f64, spacing: f64) -> usize {
    if width <= 0.0 || spacing <= 0.0 {
        return 0;
    }
    ((width + spacing / 2.0) / spacing).floor() as usize
}

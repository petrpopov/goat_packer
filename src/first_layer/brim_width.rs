//! The auto-brim width of one volume group (spec 03 §5.2): a tipping-risk term from the
//! group's height, first-layer second moments and the object's fastest speed, a thermal term
//! (zero in BambuStudio), clamped and dropped below 5 mm; then quantised to whole brim line
//! pairs and capped per island.

use super::project::{ProjectSettings, SlicerBuild};
use crate::geometry::Point;

/// Brims narrower than this are dropped (unless the 1.5·D cap is smaller).
const MIN_AUTO_BRIM_MM: f64 = 5.0;
/// Upper clamp of the auto width.
const MAX_AUTO_BRIM_MM: f64 = 18.0;
/// Extra brim for a thin tall island (O/R/S only), before quantisation.
const THIN_ISLAND_EXTRA_MM: f64 = 5.0;
/// An island with area / contour length below this counts as thin.
const THIN_ISLAND_RATIO_MM: f64 = 1.1;
/// The extra brim needs a group taller than this.
const THIN_ISLAND_MIN_HEIGHT_MM: f64 = 10.0;
/// Speed assumed when no speed setting is positive.
const FALLBACK_SPEED_MM_S: f64 = 250.0;

/// Area, centroid and second moments of an even-odd set of rings about the origin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AreaMoments {
    pub area: f64,
    /// ∫x dA, ∫y dA.
    pub first: [f64; 2],
    /// ∫y² dA (about the x axis), ∫x² dA (about the y axis).
    pub second: [f64; 2],
}

impl AreaMoments {
    /// Signed sums over closed rings: counter-clockwise rings add, clockwise rings (holes)
    /// subtract.
    pub fn of_rings(rings: &[Vec<Point>]) -> AreaMoments {
        let mut moments = AreaMoments::default();
        for ring in rings {
            for pair in ring.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let cross = a.x * b.y - b.x * a.y;
                moments.area += cross / 2.0;
                moments.first[0] += (a.x + b.x) * cross / 6.0;
                moments.first[1] += (a.y + b.y) * cross / 6.0;
                moments.second[0] += (a.y * a.y + a.y * b.y + b.y * b.y) * cross / 12.0;
                moments.second[1] += (a.x * a.x + a.x * b.x + b.x * b.x) * cross / 12.0;
            }
        }
        moments
    }

    /// `(Ixx, Iyy)` about the area's own centroid.
    pub fn central_second(&self) -> (f64, f64) {
        if self.area <= 0.0 {
            return (0.0, 0.0);
        }
        let centroid = [self.first[0] / self.area, self.first[1] / self.area];
        (
            self.second[0] - self.area * centroid[1] * centroid[1],
            self.second[1] - self.area * centroid[0] * centroid[0],
        )
    }
}

/// What the width rule reads about one volume group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupShape {
    /// Z extent of the group's volumes.
    pub height: f64,
    /// Second moments of the group's first-layer slices about their joint centroid.
    pub central_second: (f64, f64),
    /// XY size of the group's first-layer contours.
    pub size: (f64, f64),
}

/// The material terms of the rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// 2 for PETG/PCTG, 0.5 for TPU, 1 otherwise.
    pub adhesion: f64,
    /// Thermal length λ of the group (the shortest of its volumes).
    pub thermal_length: f64,
}

/// The unquantised auto width of a group (03 §5.2), 0 when the rule drops it.
pub fn auto_width(
    shape: &GroupShape,
    material: Material,
    max_speed: f64,
    build: SlicerBuild,
) -> f64 {
    let (ixx, iyy) = shape.central_second;
    if ixx <= 0.0 || iyy <= 0.0 {
        return 0.0;
    }
    let (size_x, size_y) = shape.size;
    let diagonal = libm::hypot(size_x, size_y);
    let height = shape.height;
    let tipping = (height * size_y / ixx).max(height * size_x / iyy) * height / 1920.0;
    // BambuStudio multiplies the thermal term by zero (03 §5.2, B `Brim.cpp:798`).
    let thermal = match build {
        SlicerBuild::BambuStudio => 0.0,
        _ => 8.0 * diagonal / material.thermal_length * height.min(30.0) / 30.0,
    };
    let width = material.adhesion
        * (tipping * max_speed)
            .max(thermal)
            .min(MAX_AUTO_BRIM_MM)
            .min(1.5 * diagonal);
    if width < MIN_AUTO_BRIM_MM && width < 1.5 * diagonal {
        return 0.0;
    }
    width.min(MAX_AUTO_BRIM_MM)
}

/// Whole pairs of brim lines: `⌊w / (2 s_b)⌋ · 2 s_b`.
pub fn quantised(width: f64, spacing: f64) -> f64 {
    if spacing <= 0.0 {
        return 0.0;
    }
    (width / (2.0 * spacing)).floor() * 2.0 * spacing
}

/// The width one island of an auto-brim group gets: the thin-island extra (O/R/S only; an
/// always-false guard disables it in BambuStudio), then the cap at the island's larger bbox
/// side, then quantisation again (03 §5.2).
pub fn island_width(
    group_width: f64,
    group_height: f64,
    island: &IslandShape,
    spacing: f64,
    build: SlicerBuild,
) -> f64 {
    let thin = build != SlicerBuild::BambuStudio
        && group_width < MIN_AUTO_BRIM_MM
        && group_height > THIN_ISLAND_MIN_HEIGHT_MM
        && island.area / island.contour_length < THIN_ISLAND_RATIO_MM
        && group_width < spacing;
    let width = if thin {
        group_width + quantised(THIN_ISLAND_EXTRA_MM, spacing)
    } else {
        group_width
    };
    quantised(width.min(island.size.0.max(island.size.1)), spacing)
}

/// What the per-island rules read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IslandShape {
    pub area: f64,
    pub contour_length: f64,
    pub size: (f64, f64),
}

/// The object's fastest configured speed (B `Model.cpp:4305-4339`): walls, infills, support
/// and the small-perimeter speed (a percentage of the outer wall speed).
pub fn max_print_speed(settings: &ProjectSettings) -> f64 {
    let outer = settings.number("outer_wall_speed").unwrap_or(0.0);
    let small = settings
        .length_or_percent("small_perimeter_speed", outer)
        .unwrap_or(0.0);
    let speed = [
        "inner_wall_speed",
        "sparse_infill_speed",
        "internal_solid_infill_speed",
        "top_surface_speed",
        "support_speed",
    ]
    .iter()
    .filter_map(|key| settings.number(key))
    .fold(outer.max(small), f64::max);
    if speed > 0.0 {
        speed
    } else {
        FALLBACK_SPEED_MM_S
    }
}

/// Adhesion coefficient of a filament type (03 §5.2).
pub fn adhesion_of(filament_type: &str) -> f64 {
    match filament_type.trim().to_ascii_uppercase().as_str() {
        "PETG" | "PCTG" => 2.0,
        "TPU" | "TPU-AMS" => 0.5,
        _ => 1.0,
    }
}

/// Thermal length of a filament type: 200 mm unless the slicer knows a value for the material
/// (the per-material values of B `Model.cpp:4342-4360`; Orca's table was not in the checkout,
/// so the same values are assumed there).
pub fn thermal_length_of(filament_type: &str) -> f64 {
    match filament_type.trim().to_ascii_uppercase().as_str() {
        "ABS" | "PA-CF" | "PET-CF" => 100.0,
        "PC" => 40.0,
        "TPU" | "TPU-AMS" => 1000.0,
        _ => 200.0,
    }
}

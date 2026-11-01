//! The packed footprints in bed coordinates and the plate thumbnails drawn from them.

use super::AssembleOptions;
use super::archive::{Entries, set_entry};
use super::bed::BedPolygon;
use crate::{PackedPlacement, Point, plate_preview, polygon_area};

/// Canonicalizes one extracted boundary ring: it starts at the lexicographically smallest
/// vertex and runs counter-clockwise. `extract_footprint` walks the boundary through
/// hash-ordered adjacency, so the same STL can yield a ring with a different start vertex or
/// direction on every call — harmless for packing, but the previews (and therefore the output
/// bytes, spec acceptance 6: byte-identical projects on re-run) must not vary.
fn canonical_ring(ring: &[Point]) -> Vec<Point> {
    let mut closed = ring.to_vec();
    if closed.last().is_some_and(|last| *last == closed[0]) {
        closed.pop(); // the explicit closing point is re-appended after rotation
    }
    if closed.len() < 3 {
        return ring.to_vec();
    }
    // Direction first, then the start vertex: both orders must agree for the canonical form
    // to be unique.
    if polygon_area(&closed) < 0.0 {
        closed.reverse();
    }
    let pivot = closed
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.x.total_cmp(&b.1.x).then(a.1.y.total_cmp(&b.1.y)))
        .map(|(index, _)| index)
        .expect("non-empty ring");
    let mut rotated = closed[pivot..].to_vec();
    rotated.extend_from_slice(&closed[..pivot]);
    rotated.push(rotated[0]);
    rotated
}

/// The footprint loops of every placement in bed coordinates. Mirrors `_placed_polygons`:
/// shapely `rotate(origin=(0, 0))` turns the centered footprint counter-clockwise about the
/// local origin, then `translate` moves it to the bed position (rotate first, then translate,
/// so bounding boxes stay float-identical). Canonical rings keep the assembled bytes
/// deterministic across runs (spec acceptance 6).
pub(super) fn placed_polygons(
    footprint_loops: &[Vec<Point>],
    placements: &[PackedPlacement],
) -> Vec<Vec<Vec<Point>>> {
    let loops: Vec<Vec<Point>> = footprint_loops
        .iter()
        .map(|ring| canonical_ring(ring))
        .collect();
    placements
        .iter()
        .map(|placement| placement.place_loops(&loops))
        .collect()
}

/// Contract 7: previews are cosmetic. The Python drew each layout through PIL; here each
/// size/pick combination renders once and feeds the entries BambuStudio expects.
pub(super) fn write_previews(
    entries: &mut Entries,
    options: &AssembleOptions,
    printable: &BedPolygon,
    exclusions: &[BedPolygon],
    world_polygons: &[Vec<Vec<Point>>],
) {
    let purge_line_tuples: Vec<(f64, f64, f64, f64, f64)> = options
        .purge_lines
        .iter()
        .map(|line| (line[0], line[1], line[2], line[3], line[4]))
        .collect();
    let keepout_tuples: Vec<(f64, f64, f64, f64)> = options
        .keepouts
        .iter()
        .map(|rect| (rect.min_x, rect.min_y, rect.max_x, rect.max_y))
        .collect();
    let preview = |pick: bool| plate_preview::PreviewInput {
        polygons: world_polygons,
        printable,
        exclusions,
        purge_lines: &purge_line_tuples,
        keepouts: &keepout_tuples,
        edge_margin: options.edge_margin,
        pick,
    };
    let plate_png = plate_preview::render(&preview(false), 512);
    let pick_png = plate_preview::render(&preview(true), 512);
    let small_png = plate_preview::render(&preview(false), 128);
    for name in [
        "Metadata/plate_1.png",
        "Metadata/plate_no_light_1.png",
        "Metadata/top_1.png",
    ] {
        set_entry(entries, name, plate_png.clone());
    }
    set_entry(entries, "Metadata/pick_1.png", pick_png);
    set_entry(entries, "Metadata/plate_1_small.png", small_png);
}

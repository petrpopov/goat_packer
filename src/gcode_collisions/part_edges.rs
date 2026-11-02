//! The part-bbox rules of the slicer GUIs, checked on the layout (no G-code needed): every
//! model part inside the printable bbox, and Snapmaker Orca's spiral-lift band. Part of
//! `gui_checks`.

use serde::Serialize;

use crate::geometry::Rect;
use crate::object_print::ObjectPrint;

/// Snapmaker Orca's spiral-lift margin: a part whose bbox comes closer than this to a printable
/// edge of a rectangular bed raises a serious warning (equality passes). It is a constant of
/// that GUI, not of a printer (Snapmaker Orca 2.4.0 `3DScene.cpp:1381-1401`).
pub const SNAPMAKER_SPIRAL_LIFT_BAND_MM: f64 = 3.5;

/// A part bbox further out than this is "laid over the boundary" (a GUI error).
const PART_INSIDE_EPSILON_MM: f64 = 1e-4;

/// What is wrong with one model part's bbox against the printable edge.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartEdgeIssue {
    /// Not inside the printable bbox: the GUI refuses the plate ("laid over the boundary").
    OutsidePlate,
    /// Inside, but closer than the band to an edge (the smallest distance).
    InsideBand { distance_mm: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct PartEdgeHit {
    pub object: usize,
    pub part: usize,
    pub issue: PartEdgeIssue,
}

/// The part bboxes of one placed object, plate frame. An `ObjectPrint` is built for one
/// rotation class with its loops and boxes already in that rotation's plate-aligned frame, so
/// placing it is a pure translation by `origin`.
pub fn placed_part_boxes(print: &ObjectPrint, origin: (f64, f64)) -> Vec<Rect> {
    print
        .part_boxes
        .iter()
        .map(|part| part.translated(origin.0, origin.1))
        .collect()
}

/// Part bboxes outside the printable bbox, or inside but closer than `band_mm` to an edge
/// (Snapmaker Orca's spiral-lift band; 0 = no band). `objects[i]` holds object `i`'s placed
/// part boxes.
pub fn part_edge_hits(objects: &[Vec<Rect>], printable: Rect, band_mm: f64) -> Vec<PartEdgeHit> {
    let mut hits = Vec::new();
    for (object, parts) in objects.iter().enumerate() {
        for (part, bounds) in parts.iter().enumerate() {
            let issue = if bounds.sticks_out_of(printable, PART_INSIDE_EPSILON_MM) {
                PartEdgeIssue::OutsidePlate
            } else {
                let distance_mm = [
                    bounds.min_x - printable.min_x,
                    bounds.min_y - printable.min_y,
                    printable.max_x - bounds.max_x,
                    printable.max_y - bounds.max_y,
                ]
                .into_iter()
                .fold(f64::INFINITY, f64::min);
                if band_mm <= 0.0 || distance_mm >= band_mm {
                    continue;
                }
                PartEdgeIssue::InsideBand { distance_mm }
            };
            hits.push(PartEdgeHit {
                object,
                part,
                issue,
            });
        }
    }
    hits
}

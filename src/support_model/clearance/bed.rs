//! Bambu's bed obstacle (spec 02 §2.8): everything outside the bed (and inside its excluded
//! rectangles) is model for the branches of the slicing object, at clearance K + xy_min. The
//! obstacle is a 1 m ring around the bed polygon B_obj, cut into four pieces by 2 mm slits on
//! the axes through B_obj's area centroid, and expressed in the slicing object's frame. Orca
//! builds have no such obstacle (they clip later).

use serde::{Deserialize, Serialize};

use crate::polygon_ops::{
    DEFAULT_MITER_LIMIT, Grid, IntPoint, Join, Ring, Rings, bbox, difference_islands,
    intersection_islands, offset, ring_doubled_area,
};

/// Width of the obstacle ring around the bed.
const RING_MM: f64 = 1000.0;

/// Half width of the slits cut through the ring.
const SLIT_HALF_MM: f64 = 1.0;

/// Where the bed is relative to the slicing object.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct BedPlacement {
    /// `printable_area` corners, plate frame (mm).
    pub printable_area: Vec<[f64; 2]>,
    /// `bed_exclude_area` points, plate frame; B reads groups of four as rectangles.
    pub exclude_area: Vec<[f64; 2]>,
    /// Plate position of the slicing object's frame origin (its normal parts' XY box
    /// centre): the first instance's shift (02 §2.8 step 4).
    pub object_origin: [f64; 2],
}

/// The ring pieces' outer contours on `grid`, in the object frame; empty without a bed.
pub fn bed_obstacle(placement: &BedPlacement, grid: Grid) -> Rings {
    let Some(bed) = bed_in_frame(placement, grid) else {
        return Rings::new();
    };
    let mut hole = bed.clone();
    hole.reverse();
    let outer = offset(
        std::slice::from_ref(&bed),
        grid.scaled(RING_MM),
        Join::Miter(DEFAULT_MITER_LIMIT),
    );
    let mut ring = outer;
    ring.push(hole);
    let Some(extent) = bbox(&ring) else {
        return Rings::new();
    };
    let centre = area_centroid(&bed);
    let slit = grid.scaled(SLIT_HALF_MM);
    let (min, max) = (
        [f64::from(extent.min.x), f64::from(extent.min.y)],
        [f64::from(extent.max.x), f64::from(extent.max.y)],
    );
    let (low, high) = (
        [centre[0] - slit, centre[1] - slit],
        [centre[0] + slit, centre[1] + slit],
    );
    let quadrants = [
        (min, low),
        (high, max),
        ([min[0], high[1]], [low[0], max[1]]),
        ([high[0], min[1]], [max[0], low[1]]),
    ];
    quadrants
        .iter()
        .flat_map(|&(from, to)| intersection_islands(&ring, &[rectangle(from, to)]))
        .map(|island| island[0].clone())
        .collect()
}

/// B_obj: the printable area minus the excluded rectangles, first polygon, translated into
/// the object frame.
fn bed_in_frame(placement: &BedPlacement, grid: Grid) -> Option<Ring> {
    if placement.printable_area.len() < 3 {
        return None;
    }
    let shift = placement.object_origin;
    let point = |corner: &[f64; 2]| grid.point(corner[0] - shift[0], corner[1] - shift[1]);
    let mut bed: Ring = placement.printable_area.iter().map(point).collect();
    if ring_doubled_area(&bed) < 0 {
        bed.reverse();
    }
    let exclusions: Rings = placement
        .exclude_area
        .chunks_exact(4)
        .map(|corners| {
            let mut rect: Ring = corners.iter().map(point).collect();
            if ring_doubled_area(&rect) < 0 {
                rect.reverse();
            }
            rect
        })
        .filter(|rect| ring_doubled_area(rect) != 0)
        .collect();
    let remaining = difference_islands(&[bed], &exclusions);
    remaining.into_iter().next().map(|island| island[0].clone())
}

fn area_centroid(ring: &Ring) -> [f64; 2] {
    let count = ring.len();
    let (mut cx, mut cy, mut doubled) = (0.0, 0.0, 0.0);
    for index in 0..count {
        let (a, b) = (ring[index], ring[(index + 1) % count]);
        let (ax, ay, bx, by) = (
            f64::from(a.x),
            f64::from(a.y),
            f64::from(b.x),
            f64::from(b.y),
        );
        let cross = ax * by - bx * ay;
        doubled += cross;
        cx += (ax + bx) * cross;
        cy += (ay + by) * cross;
    }
    [cx / (3.0 * doubled), cy / (3.0 * doubled)]
}

fn rectangle(from: [f64; 2], to: [f64; 2]) -> Ring {
    let corner = |x: f64, y: f64| IntPoint::new(x as i32, y as i32);
    vec![
        corner(from[0], from[1]),
        corner(to[0], from[1]),
        corner(to[0], to[1]),
        corner(from[0], to[1]),
    ]
}

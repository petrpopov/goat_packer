//! A resolved area split into islands (one outer contour with the holes inside it), the unit
//! the slicer sizes and draws brims for.

use super::brim_width::{AreaMoments, IslandShape};
use crate::geometry::{Point, Rect, point_in_footprint, polygon_area};

/// One outer contour and its holes, closed rings (first point repeated).
#[derive(Clone, Debug, PartialEq)]
pub struct Island {
    pub contour: Vec<Point>,
    pub holes: Vec<Vec<Point>>,
}

impl Island {
    pub fn shape(&self) -> IslandShape {
        let mut rings = vec![self.contour.clone()];
        rings.extend(self.holes.iter().cloned());
        let bounds = Rect::of_points(self.contour.iter());
        IslandShape {
            area: AreaMoments::of_rings(&rings).area,
            contour_length: ring_length(&self.contour),
            size: (bounds.max_x - bounds.min_x, bounds.max_y - bounds.min_y),
        }
    }
}

/// Splits resolved loops (outer rings counter-clockwise, holes clockwise, as
/// `mesh_section::area` returns them) into islands. A hole belongs to the smallest outer ring
/// around its first point.
pub fn islands(loops: &[Vec<Point>]) -> Vec<Island> {
    let (outers, holes): (Vec<&Vec<Point>>, Vec<&Vec<Point>>) =
        loops.iter().partition(|ring| polygon_area(ring) > 0.0);
    let mut islands: Vec<Island> = outers
        .iter()
        .map(|contour| Island {
            contour: (*contour).clone(),
            holes: Vec::new(),
        })
        .collect();
    for hole in holes {
        let Some(first) = hole.first() else { continue };
        let owner = islands
            .iter_mut()
            .filter(|island| {
                point_in_footprint(std::slice::from_ref(&island.contour), first.x, first.y)
            })
            .min_by(|a, b| polygon_area(&a.contour).total_cmp(&polygon_area(&b.contour)));
        if let Some(island) = owner {
            island.holes.push(hole.clone());
        }
    }
    islands
}

fn ring_length(ring: &[Point]) -> f64 {
    ring.windows(2)
        .map(|pair| libm::hypot(pair[1].x - pair[0].x, pair[1].y - pair[0].y))
        .sum()
}

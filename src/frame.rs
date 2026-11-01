//! The canonical frame of a footprint: rotated and re-centered by its own geometry, so the
//! search sees the same shape however the model was oriented or positioned on export.

use crate::footprint::Footprint;
use crate::geometry::{Point, Rect, rotate_point};
use crate::options::PackedPlacement;
use geo::{ConvexHull, MultiPoint};

/// Numerical noise below a tenth of a micron must not move a raster boundary.
const CANONICAL_ROUNDING: f64 = 1e7;

/// Coordinates are compared at micron scale when picking the canonical angle.
const SIGNATURE_QUANTUM: f64 = 1e6;

pub(crate) struct Frame {
    pub(crate) footprint: Footprint,
    angle: f64,
    center: Point,
}

impl Frame {
    pub(crate) fn new(fp: &Footprint, locked: bool) -> Self {
        if locked {
            return Self {
                footprint: fp.clone(),
                angle: 0.0,
                center: Point { x: 0.0, y: 0.0 },
            };
        }
        let (angle, rotated_center) = canonical_rotation(fp);
        let center = rotate_point(rotated_center, -angle);
        let transform = |loops: &[Vec<Point>]| -> Vec<Vec<Point>> {
            loops
                .iter()
                .map(|ring| {
                    ring.iter()
                        .map(|&p| {
                            let q = rotate_point(
                                Point {
                                    x: p.x - center.x,
                                    y: p.y - center.y,
                                },
                                angle,
                            );
                            Point {
                                x: (q.x * CANONICAL_ROUNDING).round() / CANONICAL_ROUNDING,
                                y: (q.y * CANONICAL_ROUNDING).round() / CANONICAL_ROUNDING,
                            }
                        })
                        .collect()
                })
                .collect()
        };
        let mut footprint = fp.map_loops(transform);
        let bounds = Rect::of_points(footprint.loops.iter().flatten());
        footprint.width = bounds.max_x - bounds.min_x;
        footprint.height = bounds.max_y - bounds.min_y;
        Self {
            footprint,
            angle,
            center,
        }
    }

    pub(crate) fn to_input(&self, p: &PackedPlacement) -> PackedPlacement {
        let angle = (p.angle_deg + self.angle).rem_euclid(360.0);
        let offset = rotate_point(self.center, angle);
        PackedPlacement {
            x_mm: p.x_mm - offset.x,
            y_mm: p.y_mm - offset.y,
            angle_deg: angle,
        }
    }

    pub(crate) fn to_canonical(&self, p: &PackedPlacement) -> PackedPlacement {
        let offset = rotate_point(self.center, p.angle_deg);
        PackedPlacement {
            x_mm: p.x_mm + offset.x,
            y_mm: p.y_mm + offset.y,
            angle_deg: ((p.angle_deg - self.angle).rem_euclid(360.0) * 1e9).round() / 1e9,
        }
    }
}

/// The angle (degrees) that lays one convex-hull edge of the footprint along the x axis and
/// gives the tightest, longest-side-horizontal box, with the center of that box in the
/// rotated frame. Ties between hull edges are resolved using geometry, never vertex order,
/// winding, absolute coordinates, or the input angle.
fn canonical_rotation(fp: &Footprint) -> (f64, Point) {
    let points: Vec<Point> = fp
        .loops
        .iter()
        .chain(&fp.body_loops)
        .chain(&fp.support_landing_loops)
        .chain(fp.bed_loops.iter().flatten())
        .flatten()
        .copied()
        .collect();
    let hull =
        MultiPoint::from(points.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>()).convex_hull();
    let vertices: Vec<Point> = hull
        .exterior()
        .0
        .iter()
        .map(|p| Point { x: p.x, y: p.y })
        .collect();
    let quant = |v: f64| (v * SIGNATURE_QUANTUM).round() as i64;
    let mut best = None;
    for edge in vertices.windows(2) {
        let angle = -(edge[1].y - edge[0].y)
            .atan2(edge[1].x - edge[0].x)
            .to_degrees();
        let rotated: Vec<_> = vertices.iter().map(|&p| rotate_point(p, angle)).collect();
        let bounds = Rect::of_points(rotated.iter());
        let (w, h) = (bounds.max_x - bounds.min_x, bounds.max_y - bounds.min_y);
        let center = Point {
            x: (bounds.min_x + bounds.max_x) / 2.0,
            y: (bounds.min_y + bounds.max_y) / 2.0,
        };
        // Prefer the long side horizontal; resolve half-turn/symmetry ties by the sorted set
        // of all points relative to the box center.
        let mut signature: Vec<_> = points
            .iter()
            .map(|&p| {
                let q = rotate_point(p, angle);
                (quant(q.x - center.x), quant(q.y - center.y))
            })
            .collect();
        signature.sort_unstable();
        signature.dedup();
        let key = (quant(w * h), quant(-w), signature);
        if best.as_ref().is_none_or(|(old, _, _)| key < *old) {
            best = Some((key, angle, center));
        }
    }
    let (_, angle, rotated_center) = best.expect("nondegenerate footprint");
    (angle, rotated_center)
}

#![allow(dead_code)]
use goat_packer::{Footprint, Point};

/// Rigid XY transform plus a changed ring start/winding, as an exporter might produce.
pub fn transform(fp: &Footprint, degrees: f64, dx: f64, dy: f64, reorder: bool) -> Footprint {
    let (s, c) = degrees.to_radians().sin_cos();
    let map = |loops: &[Vec<Point>]| -> Vec<Vec<Point>> {
        let mut result: Vec<_> = loops
            .iter()
            .map(|ring| {
                let mut points: Vec<_> = ring[..ring.len() - 1]
                    .iter()
                    .map(|p| Point {
                        x: p.x * c - p.y * s + dx,
                        y: p.x * s + p.y * c + dy,
                    })
                    .collect();
                if reorder {
                    points.reverse();
                    points.rotate_left(1);
                }
                points.push(points[0]);
                points
            })
            .collect();
        if reorder {
            result.reverse();
        }
        result
    };
    let mut result = fp.clone();
    result.loops = map(&fp.loops);
    result.body_loops = map(&fp.body_loops);
    result.bed_loops = fp.bed_loops.as_ref().map(|l| map(l));
    let points: Vec<_> = result.loops.iter().flatten().collect();
    result.width = points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max)
        - points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    result.height = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max)
        - points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    result
}

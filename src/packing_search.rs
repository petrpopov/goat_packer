//! The deterministic search behind `pack_footprint`: a thin wrapper over the packing
//! portfolio's legacy run (`pack::legacy`), which sweeps every angle family, a mixed palette
//! and the brick rows, and validates the winner.

use crate::footprint::Footprint;
use crate::options::{PackOptions, PackResult};
use crate::pack::legacy::{Portfolio, pack_legacy};

pub(crate) fn search(fp: &Footprint, options: &PackOptions) -> PackResult {
    let result = pack_legacy(fp, options, Portfolio::Search);
    // The webapp's progress tracker reads "best so far: N objects" from this line.
    eprintln!(
        "pack_footprint: {0}/{0} attempts, best so far: {1} objects",
        result.attempts_run,
        result.placements.len()
    );
    result
}

#[cfg(test)]
mod tests {
    use crate::geometry::{Point, point_in_loops, rotate_point};
    use crate::pixel_mask::{PixelMask, masks_overlap};
    use crate::span_raster::SpanRaster;
    use std::collections::HashSet;

    fn shape() -> Vec<Vec<Point>> {
        let ring = |points: &[(f64, f64)]| {
            let mut ring: Vec<_> = points.iter().map(|&(x, y)| Point { x, y }).collect();
            ring.push(ring[0]);
            ring
        };
        vec![
            ring(&[
                (-3., -2.),
                (3., -2.),
                (3., -1.),
                (0., -1.),
                (0., 2.),
                (-3., 2.),
            ]),
            ring(&[(-2.5, -0.5), (-1., -0.5), (-1., 1.), (-2.5, 1.)]),
        ]
    }

    #[test]
    fn scanline_matches_original_point_sampler() {
        for angle in [0., 2., 13., 45., 90., 137., 180.] {
            let loops: Vec<Vec<_>> = shape()
                .iter()
                .map(|l| l.iter().map(|&p| rotate_point(p, angle)).collect())
                .collect();
            for resolution in [0.25, 0.5, 1.] {
                let pixels: HashSet<_> = (-32..32)
                    .flat_map(|y| (-32..32).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        point_in_loops(
                            Point {
                                x: (f64::from(x) + 0.5) * resolution,
                                y: (f64::from(y) + 0.5) * resolution,
                            },
                            &loops,
                        )
                    })
                    .collect();
                let spans = SpanRaster::scanline(&loops, resolution, (-32, 31, -32, 31));
                assert_eq!(
                    PixelMask::from_spans(&spans)
                        .pixels()
                        .collect::<HashSet<_>>(),
                    pixels
                );
            }
        }
    }

    #[test]
    fn word_overlap_matches_pixel_oracle_across_word_boundaries() {
        let a = PixelMask::from_spans(&SpanRaster {
            min_y: -2,
            rows: vec![vec![(-70, -63), (-1, 2), (63, 70)], vec![(-35, 41)]],
        });
        let b = PixelMask::from_spans(&SpanRaster {
            min_y: 1,
            rows: vec![vec![(-65, -62), (0, 65)], vec![(-3, 7)]],
        });
        for y in -5..5 {
            for x in -140..140 {
                let expected = a.pixels().any(|(ax, ay)| b.contains_local(ax - x, ay - y));
                assert_eq!(masks_overlap(&a, 0, 0, &b, x, y), expected, "{x},{y}");
            }
        }
    }
}

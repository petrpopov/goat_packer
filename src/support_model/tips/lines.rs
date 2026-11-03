//! The sparse line pattern tips are seeded on (spec 01 §5.5): parallel chords through the
//! shrunk area, joined into polylines along its boundary.
//!
//! Behaviour kept: lines run along `angle`, are `spacing` apart and pass through the centre of
//! the area's bounding box; the area is first shrunk by half a flow spacing; chords of
//! neighbouring lines that overlap are chained in a serpentine through the boundary arc
//! between their ends, unless that arc is a bulge longer than the family's limit. Not kept:
//! upstream's full arc-selection classes (same-line, contrast, caps, T-joints) and its
//! free-ring and excursion pieces; the recorded vectors of spec 01 §5.5 do not exist yet, so
//! the difference is part of δ.

use crate::polygon_ops::{IntPoint, Join, Ring, bbox, offset};

pub type Polyline = Vec<IntPoint>;

/// Arcs longer than this many line spacings are never taken as a chord link (a detour, not
/// a turn).
const LONGEST_LINK_SPACINGS: f64 = 4.0;

/// One chord of the pattern: its line index and its two ends, `low` first along the line.
struct Chord {
    line: i64,
    low: IntPoint,
    high: IntPoint,
    along_low: f64,
    along_high: f64,
}

/// The side of a chord a link uses.
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Low,
    High,
}

/// A link of a chord: the other chord and the boundary points between the two ends.
struct Link {
    other: usize,
    arc: Vec<IntPoint>,
}

/// The pattern polylines of one island (outer ring first). `bulge_limit` is the largest bulge
/// of a link arc (grid units).
pub fn pattern_polylines(
    island: &[Ring],
    spacing: i64,
    angle: f64,
    shrink: i64,
    bulge_limit: f64,
) -> Vec<Polyline> {
    let Some(outer) = bbox(&island[..1.min(island.len())]) else {
        return Vec::new();
    };
    let shrunk = offset(island, -(shrink as f64), Join::Miter(3.0));
    let frame = Frame::new(angle);
    let centre = (
        (f64::from(outer.min.x) + f64::from(outer.max.x)) * 0.5,
        (f64::from(outer.min.y) + f64::from(outer.max.y)) * 0.5,
    );
    let chords = chords_of(&shrunk, &frame, centre, spacing as f64);
    chain(&chords, &shrunk, spacing as f64, bulge_limit)
}

/// The line frame: `along` is the line direction, `across` the normal.
struct Frame {
    along: (f64, f64),
    across: (f64, f64),
}

impl Frame {
    fn new(angle: f64) -> Frame {
        let (sin, cos) = (libm::sin(angle), libm::cos(angle));
        Frame {
            along: (cos, sin),
            across: (-sin, cos),
        }
    }

    fn coordinates(&self, p: IntPoint) -> (f64, f64) {
        let (x, y) = (f64::from(p.x), f64::from(p.y));
        (
            self.across.0 * x + self.across.1 * y,
            self.along.0 * x + self.along.1 * y,
        )
    }

    fn point(&self, across: f64, along: f64) -> IntPoint {
        IntPoint::new(
            libm::round(self.across.0 * across + self.along.0 * along) as i32,
            libm::round(self.across.1 * across + self.along.1 * along) as i32,
        )
    }
}

/// Every line's chords inside `region`, ordered by line then position.
fn chords_of(region: &[Ring], frame: &Frame, centre: (f64, f64), spacing: f64) -> Vec<Chord> {
    let origin = frame
        .coordinates(IntPoint::new(centre.0 as i32, centre.1 as i32))
        .0;
    let edges: Vec<((f64, f64), (f64, f64))> = region
        .iter()
        .flat_map(|ring| {
            (0..ring.len()).map(|index| {
                (
                    frame.coordinates(ring[index]),
                    frame.coordinates(ring[(index + 1) % ring.len()]),
                )
            })
        })
        .collect();
    let (low, high) = edges
        .iter()
        .fold((f64::MAX, f64::MIN), |(low, high), edge| {
            (
                low.min(edge.0.0).min(edge.1.0),
                high.max(edge.0.0).max(edge.1.0),
            )
        });
    if low > high || spacing <= 0.0 {
        return Vec::new();
    }
    let first = libm::ceil((low - origin) / spacing) as i64;
    let last = libm::floor((high - origin) / spacing) as i64;
    let mut chords = Vec::new();
    for line in first..=last {
        let across = origin + line as f64 * spacing;
        let mut crossings: Vec<f64> = edges
            .iter()
            .filter(|(a, b)| (a.0 <= across) != (b.0 <= across))
            .map(|(a, b)| a.1 + (across - a.0) / (b.0 - a.0) * (b.1 - a.1))
            .collect();
        crossings.sort_by(f64::total_cmp);
        for pair in crossings.chunks_exact(2) {
            if pair[1] - pair[0] < 1.0 {
                continue;
            }
            chords.push(Chord {
                line,
                low: frame.point(across, pair[0]),
                high: frame.point(across, pair[1]),
                along_low: pair[0],
                along_high: pair[1],
            });
        }
    }
    chords
}

/// Chords joined into serpentine chains, as polylines in chord order.
fn chain(chords: &[Chord], region: &[Ring], spacing: f64, bulge_limit: f64) -> Vec<Polyline> {
    let mut links: Vec<[Option<Link>; 2]> = chords.iter().map(|_| [None, None]).collect();
    let mut sets: Vec<usize> = (0..chords.len()).collect();
    for first in 0..chords.len() {
        let side = if chords[first].line % 2 == 0 {
            Side::High
        } else {
            Side::Low
        };
        for second in first + 1..chords.len() {
            if chords[second].line > chords[first].line + 1 {
                break;
            }
            if chords[second].line != chords[first].line + 1
                || !overlap(&chords[first], &chords[second])
            {
                continue;
            }
            let (a, b) = (end(&chords[first], side), end(&chords[second], side));
            if links[first][slot(side)].is_some()
                || links[second][slot(side)].is_some()
                || find(&mut sets, first) == find(&mut sets, second)
            {
                continue;
            }
            let Some(arc) = boundary_arc(region, a, b) else {
                continue;
            };
            if arc_length(&arc) > LONGEST_LINK_SPACINGS * spacing || bulge(&arc) > bulge_limit {
                continue;
            }
            let reverse: Vec<IntPoint> = arc.iter().rev().copied().collect();
            links[first][slot(side)] = Some(Link { other: second, arc });
            links[second][slot(side)] = Some(Link {
                other: first,
                arc: reverse,
            });
            let (root_a, root_b) = (find(&mut sets, first), find(&mut sets, second));
            sets[root_b] = root_a;
            break;
        }
    }
    polylines(chords, &links)
}

fn slot(side: Side) -> usize {
    usize::from(side == Side::High)
}

fn end(chord: &Chord, side: Side) -> IntPoint {
    if side == Side::Low {
        chord.low
    } else {
        chord.high
    }
}

fn overlap(a: &Chord, b: &Chord) -> bool {
    a.along_high.min(b.along_high) - a.along_low.max(b.along_low) > 0.0
}

fn find(sets: &mut [usize], mut index: usize) -> usize {
    while sets[index] != index {
        sets[index] = sets[sets[index]];
        index = sets[index];
    }
    index
}

/// Walks every chain from one of its free ends.
fn polylines(chords: &[Chord], links: &[[Option<Link>; 2]]) -> Vec<Polyline> {
    let mut seen = vec![false; chords.len()];
    let mut out = Vec::new();
    for start in 0..chords.len() {
        let linked = links[start].iter().flatten().count();
        if seen[start] || linked == 2 {
            continue;
        }
        let mut line: Polyline = Vec::new();
        let mut current = start;
        // Enter through the end that has no link.
        let mut entering = if links[start][slot(Side::Low)].is_none() {
            Side::Low
        } else {
            Side::High
        };
        loop {
            seen[current] = true;
            let (from, to, leaving) = match entering {
                Side::Low => (chords[current].low, chords[current].high, Side::High),
                Side::High => (chords[current].high, chords[current].low, Side::Low),
            };
            line.push(from);
            line.push(to);
            let Some(link) = &links[current][slot(leaving)] else {
                break;
            };
            line.extend(link.arc.iter().copied());
            entering = leaving;
            current = link.other;
            if seen[current] {
                break;
            }
        }
        line.dedup();
        out.push(line);
    }
    out
}

/// The shorter boundary path between two points on the same ring of `region`.
fn boundary_arc(region: &[Ring], from: IntPoint, to: IntPoint) -> Option<Vec<IntPoint>> {
    let (ring_a, edge_a) = nearest_edge(region, from)?;
    let (ring_b, edge_b) = nearest_edge(region, to)?;
    if ring_a != ring_b {
        return None;
    }
    let ring = &region[ring_a];
    let count = ring.len();
    let walk = |forward: bool| -> Vec<IntPoint> {
        let mut path = vec![from];
        let mut edge = edge_a;
        while edge != edge_b {
            if forward {
                edge = (edge + 1) % count;
                path.push(ring[edge]);
            } else {
                path.push(ring[edge]);
                edge = (edge + count - 1) % count;
            }
        }
        path.push(to);
        path
    };
    if edge_a == edge_b {
        return Some(vec![from, to]);
    }
    let (forward, backward) = (walk(true), walk(false));
    Some(if arc_length(&forward) <= arc_length(&backward) {
        forward
    } else {
        backward
    })
}

/// The ring and edge (from vertex `edge` to the next) nearest to `point`.
fn nearest_edge(region: &[Ring], point: IntPoint) -> Option<(usize, usize)> {
    let mut best: Option<(f64, usize, usize)> = None;
    for (ring_index, ring) in region.iter().enumerate() {
        for edge in 0..ring.len() {
            let distance = segment_distance2(ring[edge], ring[(edge + 1) % ring.len()], point);
            if best.is_none_or(|(known, _, _)| distance < known) {
                best = Some((distance, ring_index, edge));
            }
        }
    }
    best.map(|(_, ring, edge)| (ring, edge))
}

fn segment_distance2(a: IntPoint, b: IntPoint, p: IntPoint) -> f64 {
    let (ax, ay) = (f64::from(a.x), f64::from(a.y));
    let (dx, dy) = (f64::from(b.x) - ax, f64::from(b.y) - ay);
    let (px, py) = (f64::from(p.x) - ax, f64::from(p.y) - ay);
    let length2 = dx * dx + dy * dy;
    let t = if length2 == 0.0 {
        0.0
    } else {
        ((px * dx + py * dy) / length2).clamp(0.0, 1.0)
    };
    let (ex, ey) = (px - t * dx, py - t * dy);
    ex * ex + ey * ey
}

fn arc_length(path: &[IntPoint]) -> f64 {
    path.windows(2)
        .map(|pair| {
            let (dx, dy) = (
                f64::from(pair[1].x) - f64::from(pair[0].x),
                f64::from(pair[1].y) - f64::from(pair[0].y),
            );
            libm::sqrt(dx * dx + dy * dy)
        })
        .sum()
}

/// The largest distance of the arc from the straight line through its two ends.
fn bulge(path: &[IntPoint]) -> f64 {
    let (first, last) = (path[0], path[path.len() - 1]);
    path.iter()
        .map(|&point| libm::sqrt(segment_distance2(first, last, point)))
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(width: i32, height: i32) -> Vec<Ring> {
        vec![vec![
            IntPoint::new(0, 0),
            IntPoint::new(width, 0),
            IntPoint::new(width, height),
            IntPoint::new(0, height),
        ]]
    }

    #[test]
    fn lines_pass_through_the_box_centre_and_chain_in_a_serpentine() {
        // 100 x 40 units, lines along x every 10: centre line y = 20, shrunk by 2.
        let lines = pattern_polylines(&rectangle(100, 40), 10, 0.0, 2, 1e9);
        assert_eq!(lines.len(), 1, "{lines:?}");
        let polyline = &lines[0];
        assert!(polyline.iter().any(|p| p.y == 20), "{polyline:?}");
        let rows: std::collections::BTreeSet<i32> = polyline.iter().map(|p| p.y).collect();
        assert!(rows.contains(&0) || rows.len() >= 3);
    }

    #[test]
    fn a_zero_bulge_limit_leaves_every_chord_alone() {
        let lines = pattern_polylines(&rectangle(100, 40), 10, 0.0, 2, -1.0);
        assert!(lines.len() >= 3);
        assert!(lines.iter().all(|line| line.len() == 2));
    }

    #[test]
    fn a_thin_area_has_no_lines() {
        assert!(pattern_polylines(&rectangle(100, 3), 10, 0.0, 2, 1e9).is_empty());
    }
}

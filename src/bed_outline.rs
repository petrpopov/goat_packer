//! Our own simplified plate outline traced from a slicer bed model (top face boundary). The
//! model file is read at catalog-build time and never stored or shipped (AGPL); only the
//! resulting polygon goes into `profiles/*.json` as `plate_outline`.
// BTree, not Hash: iteration order picks the ring's start vertex, and `simplify` keeps different
// vertices from a different start, so a HashMap made every re-extract rewrite every outline.
use std::collections::{BTreeMap, BTreeSet};

use geo::{Area, Coord, LineString, Polygon, Simplify};

type Key = (i64, i64);

fn triangles(stl: &[u8]) -> Option<Vec<[[f64; 3]; 3]>> {
    if stl.len() >= 84 {
        let n = u32::from_le_bytes(stl[80..84].try_into().ok()?) as usize;
        if 84 + n * 50 == stl.len() {
            let f = |o: usize| f32::from_le_bytes(stl[o..o + 4].try_into().unwrap()) as f64;
            return Some((0..n).map(|i| { let b = 84 + i * 50 + 12; [0, 1, 2].map(|v| [f(b + v * 12), f(b + v * 12 + 4), f(b + v * 12 + 8)]) }).collect());
        }
    }
    let text = std::str::from_utf8(stl).ok()?;
    let verts: Vec<[f64; 3]> = text
        .lines()
        .filter_map(|l| {
            let mut it = l.trim().strip_prefix("vertex")?.split_whitespace().map(|s| s.parse::<f64>().ok());
            Some([it.next()??, it.next()??, it.next()??])
        })
        .collect();
    (!verts.is_empty() && verts.len() % 3 == 0).then(|| verts.chunks(3).map(|c| [c[0], c[1], c[2]]).collect())
}

pub fn outline_from_stl(stl: &[u8], printable: [f64; 4]) -> Option<Vec<[f64; 2]>> {
    let tris = triangles(stl)?;
    // Real bed models carry small raised details (logos, clips) above the plate: the plate face is
    // the horizontal level with the largest total area, not the topmost one.
    let mut level_area: Vec<(f64, f64)> = Vec::new();
    for t in tris.iter().filter(|t| t.iter().all(|v| (v[2] - t[0][2]).abs() < 1e-3)) {
        let area = ((t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1])).abs() / 2.0;
        match level_area.iter_mut().find(|(z, _)| (*z - t[0][2]).abs() < 1e-3) {
            Some((_, a)) => *a += area,
            None => level_area.push((t[0][2], area)),
        }
    }
    let z_top = level_area.iter().max_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0))).map(|l| l.0)?;
    let key = |v: &[f64; 3]| -> Key { ((v[0] * 100.0).round() as i64, (v[1] * 100.0).round() as i64) };
    let mut edges: BTreeMap<(Key, Key), u32> = BTreeMap::new();
    for t in tris.iter().filter(|t| t.iter().all(|v| (v[2] - z_top).abs() < 1e-3)) {
        for i in 0..3 {
            let (a, b) = (key(&t[i]), key(&t[(i + 1) % 3]));
            *edges.entry(if a < b { (a, b) } else { (b, a) }).or_default() += 1;
        }
    }
    let mut adjacency: BTreeMap<Key, Vec<Key>> = BTreeMap::new();
    for ((a, b), count) in edges {
        if count == 1 {
            adjacency.entry(a).or_default().push(b);
            adjacency.entry(b).or_default().push(a);
        }
    }
    let mut seen = BTreeSet::new();
    let mut best: Option<(f64, Vec<Key>)> = None;
    for &start in adjacency.keys() {
        if seen.contains(&start) {
            continue;
        }
        let mut ring = vec![start];
        seen.insert(start);
        let mut current = start;
        while let Some(&next) = adjacency.get(&current).and_then(|n| n.iter().find(|k| !seen.contains(*k))) {
            seen.insert(next);
            ring.push(next);
            current = next;
        }
        if ring.len() < 3 {
            continue;
        }
        let poly = Polygon::new(LineString::from(ring.iter().map(|k| Coord { x: k.0 as f64 / 100.0, y: k.1 as f64 / 100.0 }).collect::<Vec<_>>()), vec![]);
        let area = poly.unsigned_area();
        if best.as_ref().is_none_or(|(a, _)| area > *a) {
            best = Some((area, ring));
        }
    }
    let (_, ring) = best?;
    let mut line = LineString::from(ring.iter().map(|k| Coord { x: k.0 as f64 / 100.0, y: k.1 as f64 / 100.0 }).collect::<Vec<_>>());
    line.close();
    let simple = line.simplify(0.5);
    let mut points: Vec<[f64; 2]> = simple.0.iter().map(|c| [c.x, c.y]).collect();
    points.pop(); // drop the closing duplicate
    let (min_x, max_x) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p[0]), b.max(p[0])));
    let (min_y, max_y) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p[1]), b.max(p[1])));
    let [ax0, ay0, ax1, ay1] = printable;
    let contains = min_x <= ax0 + 1e-6 && min_y <= ay0 + 1e-6 && max_x >= ax1 - 1e-6 && max_y >= ay1 - 1e-6;
    let (dx, dy) = if contains { (0.0, 0.0) } else { ((ax0 + ax1) / 2.0 - (min_x + max_x) / 2.0, (ay0 + ay1) / 2.0 - (min_y + max_y) / 2.0) };
    let round = |v: f64| (v * 10.0).round() / 10.0;
    (points.len() >= 3 && points.len() <= 60).then(|| points.iter().map(|p| [round(p[0] + dx), round(p[1] + dy)]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Binary STL of an axis-aligned slab [x0,x1]×[y0,y1]×[-1,0] (12 triangles).
    fn slab(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<u8> {
        let p = |x: f32, y: f32, z: f32| [x, y, z];
        let (a, b, c, d) = (p(x0, y0, 0.0), p(x1, y0, 0.0), p(x1, y1, 0.0), p(x0, y1, 0.0));
        let (e, f, g, h) = (p(x0, y0, -1.0), p(x1, y0, -1.0), p(x1, y1, -1.0), p(x0, y1, -1.0));
        let tris = [[a, b, c], [a, c, d], [e, g, f], [e, h, g], [a, e, f], [a, f, b], [b, f, g], [b, g, c], [c, g, h], [c, h, d], [d, h, e], [d, e, a]];
        let mut out = vec![0u8; 80];
        out.extend((tris.len() as u32).to_le_bytes());
        for t in tris {
            out.extend([0u8; 12]);
            for v in t { for c in v { out.extend(c.to_le_bytes()); } }
            out.extend([0u8; 2]);
        }
        out
    }

    #[test]
    fn slab_top_face_is_the_outline() {
        let outline = outline_from_stl(&slab(-2.0, -5.0, 258.0, 260.0), [0.0, 0.0, 256.0, 256.0]).unwrap();
        let xs: Vec<f64> = outline.iter().map(|p| p[0]).collect();
        let ys: Vec<f64> = outline.iter().map(|p| p[1]).collect();
        assert_eq!(outline.len(), 4);
        assert_eq!(xs.iter().cloned().fold(f64::INFINITY, f64::min), -2.0);
        assert_eq!(ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max), 260.0);
    }

    #[test]
    fn centred_bed_model_is_moved_onto_the_printable_area() {
        let outline = outline_from_stl(&slab(-140.0, -146.5, 140.0, 146.5), [0.5, 1.0, 270.5, 271.0]).unwrap();
        let cx = (outline.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min) + outline.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max)) / 2.0;
        assert!((cx - 135.5).abs() < 0.11);
    }

    #[test]
    fn outline_is_deterministic() {
        let stl = slab(-2.0, -5.0, 258.0, 260.0);
        let first = outline_from_stl(&stl, [0.0, 0.0, 256.0, 256.0]).unwrap();
        for _ in 0..16 {
            assert_eq!(outline_from_stl(&stl, [0.0, 0.0, 256.0, 256.0]).unwrap(), first);
        }
    }

    #[test]
    fn garbage_is_none() {
        assert!(outline_from_stl(b"not an stl", [0.0, 0.0, 100.0, 100.0]).is_none());
    }
}

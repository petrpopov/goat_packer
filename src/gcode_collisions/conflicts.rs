//! The slicers' "G-code path conflicts" rule, written from its behaviour (design 2026-11-02
//! §10.2; zones research §6), not from their code. On one layer, two centre lines of different
//! owners (two objects, or an object and the prime tower) conflict when
//! - they cross at a point farther than `CONFLICT_TOLERANCE_MM` from every end of both, or
//! - they run along the same line and overlap by more than `CONFLICT_TOLERANCE_MM`.
//!
//! Lines of one owner never conflict, and two support lines never do (the slicers raise their
//! crossing threshold for support pairs until it never fires; the owner allows supports to
//! merge). Objects bring walls, infill and supports (a tree support's first-layer base and its
//! brim-like expansion are support lines); object brims and the skirt take no part. Which of
//! the tower's lines count differs between the slicer builds (`TowerLines`; behaviour in
//! `docs/superpowers/specs/2026-11-03-gcode-conflict-check.md`).
//!
//! Two deliberate differences from the slicers, both on the strict side: the overlap of
//! collinear lines is measured along the line (the slicers measure it along X, which divides by
//! zero for vertical lines and so never fires for them), and support pairs are exempt from the
//! overlap rule too, as the owner's permission matrix says (`keepout::matrix`).

use std::collections::BTreeMap;

use serde::Serialize;

use super::plane::{Vec2, cross, dot, length, sub};

/// Distance from a segment end inside which a crossing does not count, and the shortest
/// collinear overlap that does: 0.01 mm in the slicers' rule.
pub(super) const CONFLICT_TOLERANCE_MM: f64 = 0.01;

/// G-code rounds coordinates to 3 decimals, so lines the slicer laid on one line can appear up
/// to ~0.0007 mm apart; ends closer than this to the other line count as on it.
const COLLINEAR_TOLERANCE_MM: f64 = 1e-3;

/// Spatial hash cell: the slicers bucket lines on a 1 mm grid too.
const CELL_MM: f64 = 1.0;

/// Shorter segments are points, which cannot cross anything.
const MIN_SEGMENT_MM: f64 = 1e-6;

/// Which of the prime tower's lines the slicer's rule compares with the objects' lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TowerLines {
    /// The rule never sees the tower: the build leaves its fake tower empty (OrcaSlicer on a
    /// machine that is not a Bambu one).
    None,
    /// Only the brim loops: the first layer's and every chamfer loop above it (Snapmaker Orca
    /// 2.4.0). The tower's wall is not among them.
    BrimLoops,
    /// The wall of every layer and the brim loops (BambuStudio).
    All,
}

/// Who printed a line.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Owner {
    Object(usize),
    Tower,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    Crossing,
    CollinearOverlap,
}

/// One centre-line segment taking part in the rule.
pub(super) struct ConflictLine {
    pub(super) owner: Owner,
    pub(super) from: Vec2,
    pub(super) to: Vec2,
    pub(super) support: bool,
}

/// The two owners of a conflicting pair, ordered.
pub(super) type HitKey = (Owner, Owner);

/// The first conflicting pair of lines (lowest indices) of one key on one layer.
#[derive(Clone, Copy, Debug)]
pub(super) struct LayerHit {
    pub(super) kind: ConflictKind,
    pub(super) at: Vec2,
    lines: (usize, usize),
}

/// Every conflicting owner pair on one layer, each with its first (lowest-index) pair of lines,
/// so the answer does not depend on the order cells are visited in.
pub(super) fn layer_conflicts(lines: &[ConflictLine]) -> BTreeMap<HitKey, LayerHit> {
    let cells = cell_entries(lines);
    let mut hits: BTreeMap<HitKey, LayerHit> = BTreeMap::new();
    for group in cells.chunk_by(|a, b| (a.0, a.1) == (b.0, b.1)) {
        let first_owner = lines[group[0].2].owner;
        if group
            .iter()
            .all(|entry| lines[entry.2].owner == first_owner)
        {
            continue;
        }
        for (position, &(_, _, i)) in group.iter().enumerate() {
            for &(_, _, j) in &group[position + 1..] {
                record_pair(lines, i, j, &mut hits);
            }
        }
    }
    hits
}

fn record_pair(lines: &[ConflictLine], i: usize, j: usize, hits: &mut BTreeMap<HitKey, LayerHit>) {
    let (a, b) = (&lines[i], &lines[j]);
    if a.owner == b.owner || (a.support && b.support) {
        return;
    }
    let Some((kind, at)) = conflict_between(a.from, a.to, b.from, b.to) else {
        return;
    };
    let key = (a.owner.min(b.owner), a.owner.max(b.owner));
    let hit = LayerHit {
        kind,
        at,
        lines: (i, j),
    };
    hits.entry(key)
        .and_modify(|seen| {
            if hit.lines < seen.lines {
                *seen = hit;
            }
        })
        .or_insert(hit);
}

/// `(cell x, cell y, line)` for every grid cell a line passes, sorted and without repeats. A
/// line is cut into pieces no longer than a cell and each piece claims the cells of its bbox,
/// so two lines that meet always share the cell of the meeting point.
fn cell_entries(lines: &[ConflictLine]) -> Vec<(i32, i32, usize)> {
    let mut entries = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let span = sub(line.to, line.from);
        let pieces = ((length(span) / CELL_MM).ceil() as usize).max(1);
        let mut start = line.from;
        for piece in 1..=pieces {
            let t = piece as f64 / pieces as f64;
            let end = (line.from.0 + span.0 * t, line.from.1 + span.1 * t);
            let (low, high) = (
                cell(start.0.min(end.0), start.1.min(end.1)),
                cell(start.0.max(end.0), start.1.max(end.1)),
            );
            for cx in low.0..=high.0 {
                for cy in low.1..=high.1 {
                    entries.push((cx, cy, index));
                }
            }
            start = end;
        }
    }
    entries.sort_unstable();
    entries.dedup();
    entries
}

fn cell(x: f64, y: f64) -> (i32, i32) {
    ((x / CELL_MM).floor() as i32, (y / CELL_MM).floor() as i32)
}

/// Whether two segments conflict, and where (the crossing point, or the middle of the shared
/// run).
pub(super) fn conflict_between(
    p0: Vec2,
    p1: Vec2,
    q0: Vec2,
    q1: Vec2,
) -> Option<(ConflictKind, Vec2)> {
    let (d, e) = (sub(p1, p0), sub(q1, q0));
    let (length_d, length_e) = (length(d), length(e));
    if length_d < MIN_SEGMENT_MM || length_e < MIN_SEGMENT_MM {
        return None;
    }
    let off_p = |point: Vec2| cross(d, sub(point, p0)).abs() / length_d;
    let off_q = |point: Vec2| cross(e, sub(point, q0)).abs() / length_e;
    let collinear =
        off_p(q0).max(off_p(q1)).max(off_q(p0)).max(off_q(p1)) <= COLLINEAR_TOLERANCE_MM;
    if collinear {
        return collinear_overlap(p0, d, length_d, q0, q1);
    }
    let denominator = cross(d, e);
    if denominator == 0.0 {
        return None;
    }
    let start = sub(q0, p0);
    let t = cross(start, e) / denominator;
    let u = cross(start, d) / denominator;
    if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
        return None;
    }
    let at = (p0.0 + d.0 * t, p0.1 + d.1 * t);
    let nearest_end = [p0, p1, q0, q1]
        .into_iter()
        .map(|end| length(sub(at, end)))
        .fold(f64::INFINITY, f64::min);
    (nearest_end > CONFLICT_TOLERANCE_MM).then_some((ConflictKind::Crossing, at))
}

/// The run two collinear segments share, measured along the first one.
fn collinear_overlap(
    p0: Vec2,
    d: Vec2,
    length_d: f64,
    q0: Vec2,
    q1: Vec2,
) -> Option<(ConflictKind, Vec2)> {
    let along = |point: Vec2| dot(sub(point, p0), d) / length_d;
    let (a, b) = (along(q0), along(q1));
    let low = a.min(b).max(0.0);
    let high = a.max(b).min(length_d);
    if high - low <= CONFLICT_TOLERANCE_MM {
        return None;
    }
    let middle = (low + high) * 0.5 / length_d;
    Some((
        ConflictKind::CollinearOverlap,
        (p0.0 + d.0 * middle, p0.1 + d.1 * middle),
    ))
}

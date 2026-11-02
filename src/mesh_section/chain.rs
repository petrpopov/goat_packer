//! Chaining section segments into loops (spec 01 §1.3 step 2): first by shared mesh
//! topology, then by identical end points (forward, then reversed), then across gaps up to
//! `MAX_GAP_MM` (forward, then reversed). Chains still open after that are dropped, as the
//! slicer drops them.

use std::collections::BTreeMap;

use super::plane::{EndKey, Segment};

/// Widest gap an open chain is closed across (spec 01 §1.3 step 2).
pub const MAX_GAP_MM: f64 = 2.0;

/// A run of segments; `points` holds every vertex including both ends.
#[derive(Clone, Debug)]
struct Chain {
    points: Vec<[f64; 2]>,
}

impl Chain {
    fn start(&self) -> [f64; 2] {
        self.points[0]
    }

    fn end(&self) -> [f64; 2] {
        self.points[self.points.len() - 1]
    }
}

/// Closed loops (not repeating the first point), in the order their first segment appears.
pub fn chain_loops(segments: &[Segment]) -> Vec<Vec<[f64; 2]>> {
    let (mut loops, open) = chain_by_topology(segments);
    let open = join_open(open, &mut loops, exact_match, false);
    let open = join_open(open, &mut loops, exact_match, true);
    let open = join_open(open, &mut loops, gap_match, false);
    join_open(open, &mut loops, gap_match, true);
    loops
}

/// Follows end keys: a segment ending on key K continues with the lowest-indexed unused
/// segment starting on K. Returns closed loops and the chains that dead-ended.
fn chain_by_topology(segments: &[Segment]) -> (Vec<Vec<[f64; 2]>>, Vec<Chain>) {
    let mut starting_at: BTreeMap<EndKey, Vec<usize>> = BTreeMap::new();
    for (index, segment) in segments.iter().enumerate() {
        starting_at.entry(segment.from.0).or_default().push(index);
    }
    let mut used = vec![false; segments.len()];
    let mut loops = Vec::new();
    let mut open = Vec::new();
    for first in 0..segments.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        let start_key = segments[first].from.0;
        let mut points = vec![segments[first].from.1];
        let mut current = first;
        loop {
            let end = segments[current].to;
            if end.0 == start_key {
                loops.push(points);
                break;
            }
            points.push(end.1);
            let next = starting_at
                .get(&end.0)
                .and_then(|candidates| candidates.iter().copied().find(|&index| !used[index]));
            match next {
                Some(index) => {
                    used[index] = true;
                    current = index;
                }
                None => {
                    open.push(Chain { points });
                    break;
                }
            }
        }
    }
    (loops, open)
}

/// How far an end is from a candidate point, if they may be joined.
type Matcher = fn([f64; 2], [f64; 2]) -> Option<f64>;

fn exact_match(end: [f64; 2], start: [f64; 2]) -> Option<f64> {
    (end == start).then_some(0.0)
}

fn gap_match(end: [f64; 2], start: [f64; 2]) -> Option<f64> {
    let gap = libm::hypot(end[0] - start[0], end[1] - start[1]);
    (gap <= MAX_GAP_MM).then_some(gap)
}

/// One joining pass: each open chain (in order) repeatedly takes the nearest matching chain
/// (its own start closes it). `reversed` matches ends against ends and appends the other
/// chain backwards.
fn join_open(
    mut open: Vec<Chain>,
    loops: &mut Vec<Vec<[f64; 2]>>,
    matcher: Matcher,
    reversed: bool,
) -> Vec<Chain> {
    let mut index = 0;
    while index < open.len() {
        let end = open[index].end();
        let candidate = nearest(&open, index, end, matcher, reversed);
        match candidate {
            None => index += 1,
            Some(other) if other == index => {
                let mut chain = open.remove(index);
                if chain.points.len() > 1 && chain.start() == chain.end() {
                    chain.points.pop();
                }
                if chain.points.len() >= 3 {
                    loops.push(chain.points);
                }
            }
            Some(other) => {
                let mut tail = open.remove(other).points;
                let target = if other < index { index - 1 } else { index };
                if reversed {
                    tail.reverse();
                }
                let points = &mut open[target].points;
                if points.last() == tail.first() {
                    tail.remove(0);
                }
                points.extend(tail);
                index = target;
            }
        }
    }
    open
}

/// The open chain whose start (end, when `reversed`) is nearest to `end`; ties keep the
/// lower index. A chain matches its own start only in the forward pass.
fn nearest(
    open: &[Chain],
    index: usize,
    end: [f64; 2],
    matcher: Matcher,
    reversed: bool,
) -> Option<usize> {
    let mut best: Option<(f64, usize)> = None;
    for (other, chain) in open.iter().enumerate() {
        let point = match (reversed, other == index) {
            (true, true) => continue,
            (true, false) => chain.end(),
            (false, _) => chain.start(),
        };
        if let Some(gap) = matcher(end, point)
            && best.is_none_or(|(best_gap, _)| gap < best_gap)
        {
            best = Some((gap, other));
        }
    }
    best.map(|(_, other)| other)
}

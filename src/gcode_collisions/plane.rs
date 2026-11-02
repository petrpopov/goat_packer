//! Small plane-geometry helpers for the G-code checks: segments against rings and rectangles.

use crate::geometry::{Point, Rect};

/// A point or a vector, mm.
pub(super) type Vec2 = (f64, f64);

pub(super) fn sub(a: Vec2, b: Vec2) -> Vec2 {
    (a.0 - b.0, a.1 - b.1)
}

pub(super) fn cross(a: Vec2, b: Vec2) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

pub(super) fn dot(a: Vec2, b: Vec2) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

pub(super) fn length(a: Vec2) -> f64 {
    a.0.hypot(a.1)
}

/// Whether segments `a0 a1` and `b0 b1` share a point, touching included.
pub(super) fn segments_meet(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> bool {
    let side = |o: Vec2, p: Vec2, q: Vec2| cross(sub(p, o), sub(q, o));
    let (d1, d2) = (side(b0, b1, a0), side(b0, b1, a1));
    let (d3, d4) = (side(a0, a1, b0), side(a0, a1, b1));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    let on = |p: Vec2, s0: Vec2, s1: Vec2, turn: f64| {
        turn == 0.0
            && p.0 >= s0.0.min(s1.0)
            && p.0 <= s0.0.max(s1.0)
            && p.1 >= s0.1.min(s1.1)
            && p.1 <= s0.1.max(s1.1)
    };
    on(a0, b0, b1, d1) || on(a1, b0, b1, d2) || on(b0, a0, a1, d3) || on(b1, a0, a1, d4)
}

/// Even-odd containment of a point in one closed ring (the closing edge is implied).
pub(super) fn ring_contains(ring: &[Point], (x, y): Vec2) -> bool {
    let mut inside = false;
    let mut previous = match ring.last() {
        Some(point) => *point,
        None => return false,
    };
    for &point in ring {
        if (point.y > y) != (previous.y > y)
            && x < (previous.x - point.x) * (y - point.y) / (previous.y - point.y) + point.x
        {
            inside = !inside;
        }
        previous = point;
    }
    inside
}

/// Whether segment `a b` has any point inside or on the ring.
pub(super) fn segment_meets_ring(a: Vec2, b: Vec2, ring: &[Point]) -> bool {
    if ring_contains(ring, a) || ring_contains(ring, b) {
        return true;
    }
    let Some(last) = ring.last() else {
        return false;
    };
    let mut previous = (last.x, last.y);
    ring.iter().any(|point| {
        let current = (point.x, point.y);
        let meets = segments_meet(a, b, previous, current);
        previous = current;
        meets
    })
}

/// Whether segment `a b` has any point inside or on `rect` (the segment clipped to the
/// rectangle's slabs is not empty).
pub(super) fn segment_meets_rect(a: Vec2, b: Vec2, rect: Rect) -> bool {
    let (mut enter, mut leave) = (0.0f64, 1.0f64);
    let axes = [
        (a.0, b.0 - a.0, rect.min_x, rect.max_x),
        (a.1, b.1 - a.1, rect.min_y, rect.max_y),
    ];
    for (start, delta, low, high) in axes {
        if delta == 0.0 {
            if start < low || start > high {
                return false;
            }
            continue;
        }
        let (t0, t1) = ((low - start) / delta, (high - start) / delta);
        enter = enter.max(t0.min(t1));
        leave = leave.min(t0.max(t1));
        if enter > leave {
            return false;
        }
    }
    true
}

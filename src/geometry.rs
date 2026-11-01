//! Plane geometry shared by the whole packer: points, rectangles and loops (closed rings of
//! points; several loops together follow the even-odd rule, so holes are just more loops).

use geo::{Coord, LineString, Polygon, algorithm::bool_ops::unary_union};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Micron-scale polygons survive a union as separate specks — triangulation noise, not
/// geometry the packer can collide with. Anything smaller than a printed feature (a 0.1 mm
/// square is already below a 0.4 mm nozzle) is dropped.
pub(crate) const MIN_LOOP_AREA_MM2: f64 = 0.01;

pub fn polygon_area(points: &[Point]) -> f64 {
    points
        .windows(2)
        .map(|pair| pair[0].x * pair[1].y - pair[1].x * pair[0].y)
        .sum::<f64>()
        * 0.5
}

/// Drops loops smaller than `MIN_LOOP_AREA_MM2`.
pub(crate) fn drop_specks(loops: &mut Vec<Vec<Point>>) {
    loops.retain(|loop_| polygon_area(loop_).abs() >= MIN_LOOP_AREA_MM2);
}

/// Largest loop first (the outer ring of a footprint leads, holes and islands follow).
pub(crate) fn sort_largest_first(loops: &mut [Vec<Point>]) {
    loops.sort_by(|a, b| polygon_area(b).abs().total_cmp(&polygon_area(a).abs()));
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Rect {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Rect {
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    /// Bounding box of the points; infinite (`min > max`) when there are none.
    pub fn of_points<'a>(points: impl Iterator<Item = &'a Point>) -> Self {
        let mut bounds = Self::new(
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for point in points {
            bounds.min_x = bounds.min_x.min(point.x);
            bounds.min_y = bounds.min_y.min(point.y);
            bounds.max_x = bounds.max_x.max(point.x);
            bounds.max_y = bounds.max_y.max(point.y);
        }
        bounds
    }

    /// Smallest rectangle covering both.
    pub fn union(self, other: Self) -> Self {
        Self::new(
            self.min_x.min(other.min_x),
            self.min_y.min(other.min_y),
            self.max_x.max(other.max_x),
            self.max_y.max(other.max_y),
        )
    }

    pub fn translated(self, dx: f64, dy: f64) -> Self {
        Self::new(
            self.min_x + dx,
            self.min_y + dy,
            self.max_x + dx,
            self.max_y + dy,
        )
    }

    /// Whether any side lies farther than `tolerance` outside `area`.
    pub fn sticks_out_of(self, area: Self, tolerance: f64) -> bool {
        self.min_x < area.min_x - tolerance
            || self.min_y < area.min_y - tolerance
            || self.max_x > area.max_x + tolerance
            || self.max_y > area.max_y + tolerance
    }

    pub fn inset(self, margin: f64) -> Result<Self, String> {
        if margin < 0.0 {
            return Err("edge margin must be non-negative".to_owned());
        }
        let inset = Self::new(
            self.min_x + margin,
            self.min_y + margin,
            self.max_x - margin,
            self.max_y - margin,
        );
        if inset.min_x >= inset.max_x || inset.min_y >= inset.max_y {
            return Err(format!(
                "edge margin {margin} mm consumes printable rectangle {self:?}"
            ));
        }
        Ok(inset)
    }
}

pub fn axis_aligned_line_keepout(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    line_width: f64,
) -> Result<Rect, String> {
    if line_width <= 0.0 {
        return Err("purge-line width must be positive".to_owned());
    }
    let horizontal = (y0 - y1).abs() <= 1e-9;
    let vertical = (x0 - x1).abs() <= 1e-9;
    if !horizontal && !vertical {
        return Err("only axis-aligned purge-line segments are supported".to_owned());
    }
    if horizontal && vertical {
        return Err("purge-line segment must have non-zero length".to_owned());
    }
    let radius = line_width * 0.5;
    Ok(Rect::new(
        x0.min(x1) - radius,
        y0.min(y1) - radius,
        x0.max(x1) + radius,
        y0.max(y1) + radius,
    ))
}

/// `point` rotated counter-clockwise by `angle_deg` around the origin.
pub(crate) fn rotate_point(point: Point, angle_deg: f64) -> Point {
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    Point {
        x: point.x * cos - point.y * sin,
        y: point.x * sin + point.y * cos,
    }
}

pub(crate) fn rotate_loops(loops: &[Vec<Point>], angle_deg: f64) -> Vec<Vec<Point>> {
    loops
        .iter()
        .map(|loop_| loop_.iter().map(|&p| rotate_point(p, angle_deg)).collect())
        .collect()
}

pub(crate) fn point_in_loops(point: Point, loops: &[Vec<Point>]) -> bool {
    let mut inside = false;
    for loop_ in loops {
        for edge in loop_.windows(2) {
            let a = edge[0];
            let b = edge[1];
            if (a.y > point.y) != (b.y > point.y)
                && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
            {
                inside = !inside;
            }
        }
    }
    inside
}

/// Even-odd containment of a point in a footprint's loops — public for diagnostic tools.
pub fn point_in_footprint(loops: &[Vec<Point>], x: f64, y: f64) -> bool {
    point_in_loops(Point { x, y }, loops)
}

/// A geo ring as a closed loop shifted by `-origin`; `None` for fewer than three points.
pub(crate) fn closed_loop<'a>(
    ring: impl Iterator<Item = &'a Coord>,
    origin: (f64, f64),
) -> Option<Vec<Point>> {
    let mut loop_: Vec<Point> = ring
        .map(|coord| Point {
            x: coord.x - origin.0,
            y: coord.y - origin.1,
        })
        .collect();
    if loop_.len() < 3 {
        return None;
    }
    let first = loop_[0];
    if loop_.last() != Some(&first) {
        loop_.push(first);
    }
    Some(loop_)
}

/// Unions two footprints' loops into exterior rings (bed-extent loops: first-layer patch
/// plus whole-model silhouette). Both must share the same center — the assembler's model
/// center — which the caller guarantees.
pub fn union_loops(first: &crate::Footprint, second: &crate::Footprint) -> Vec<Vec<Point>> {
    let owned: Vec<Polygon> = first
        .loops
        .iter()
        .chain(second.loops.iter())
        .map(|loop_| polygon_from_loop(loop_))
        .collect();
    union_exterior_loops(&owned)
}

/// `loops` with the part outside `inner` grown by `reach` mm (exterior rings, holes filled):
/// a bed-extent zone whose supports (the patch beyond the body) may print farther in a later
/// slice than in the ones they were learned from. The body itself does not grow.
pub fn grow_beyond(loops: &[Vec<Point>], inner: &[Vec<Point>], reach: f64) -> Vec<Vec<Point>> {
    use geo::{BooleanOps, Buffer, MultiPolygon};
    assert!(
        reach.is_finite() && reach >= 0.0,
        "reach must be non-negative"
    );
    let polygons_of = |loops: &[Vec<Point>]| -> Vec<Polygon> {
        filled_exterior_loops(loops)
            .iter()
            .map(|loop_| polygon_from_loop(loop_))
            .collect()
    };
    let outer = polygons_of(loops);
    let beyond =
        MultiPolygon::new(outer.clone()).difference(&MultiPolygon::new(polygons_of(inner)));
    let grown = beyond.buffer(reach);
    let owned: Vec<Polygon> = outer.into_iter().chain(grown.0).collect();
    union_exterior_loops(&owned)
}

/// Union of polygons as exterior rings, largest first, slivers dropped. Every polygon is
/// oriented counter-clockwise first: geo's `unary_union` picks its fill rule from the FIRST
/// ring's winding and silently drops rings wound the other way, so a clockwise strip
/// (`strip_polygon`) after a counter-clockwise loop vanished from a learned support patch.
pub(crate) fn union_exterior_loops(owned: &[Polygon]) -> Vec<Vec<Point>> {
    use geo::orient::{Direction, Orient};
    let oriented: Vec<Polygon> = owned
        .iter()
        .map(|polygon| polygon.orient(Direction::Default))
        .collect();
    let polygons: Vec<&Polygon> = oriented.iter().collect();
    let union = unary_union(polygons);
    let mut loops: Vec<Vec<Point>> = union
        .0
        .iter()
        .filter_map(|polygon| closed_loop(polygon.exterior().coords(), (0.0, 0.0)))
        .collect();
    drop_specks(&mut loops);
    sort_largest_first(&mut loops);
    loops
}

/// The loops as hole-free exterior rings: each loop becomes a polygon and their union covers
/// the holes (a hole ring's own polygon sits inside the body's). Comparisons against a
/// footprint's holes treat them as filled, matching the collision masks.
pub(crate) fn filled_exterior_loops(loops: &[Vec<Point>]) -> Vec<Vec<Point>> {
    let polygons: Vec<Polygon> = loops.iter().map(|loop_| polygon_from_loop(loop_)).collect();
    let union = unary_union(polygons.iter());
    union
        .0
        .iter()
        .map(|polygon| {
            polygon
                .exterior()
                .coords()
                .map(|coord| Point {
                    x: coord.x,
                    y: coord.y,
                })
                .collect()
        })
        .filter(|loop_: &Vec<Point>| loop_.len() >= 3)
        .collect()
}

/// Largest sampling step along a keepout's sides in `loops_touch_rect`, mm.
const TOUCH_SAMPLE_STEP_MM: f64 = 2.0;

/// Whether footprint loops physically touch an axis-aligned `rect` (a tower keepout): any
/// loop vertex inside the rect, or any sampled point of the rect inside the loops. The rect
/// is sampled on a ≤2 mm grid — both shapes in the tower-growth check are tens of
/// millimetres across, so a thinner miss is not physical.
pub fn loops_touch_rect(loops: &[Vec<Point>], rect: Rect) -> bool {
    for point in loops.iter().flatten() {
        if point.x >= rect.min_x
            && point.x <= rect.max_x
            && point.y >= rect.min_y
            && point.y <= rect.max_y
        {
            return true;
        }
    }
    let width = rect.max_x - rect.min_x;
    let height = rect.max_y - rect.min_y;
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    let columns = ((width / TOUCH_SAMPLE_STEP_MM).ceil() as usize).max(1);
    let rows = ((height / TOUCH_SAMPLE_STEP_MM).ceil() as usize).max(1);
    for row in 0..=rows {
        let y = rect.min_y + height * f64::from(row as u32) / f64::from(rows as u32);
        for column in 0..=columns {
            let x = rect.min_x + width * f64::from(column as u32) / f64::from(columns as u32);
            if point_in_loops(Point { x, y }, loops) {
                return true;
            }
        }
    }
    false
}

pub(crate) fn polygon_from_loop(loop_: &[Point]) -> Polygon {
    let coords: Vec<Coord> = loop_
        .iter()
        .map(|point| Coord {
            x: point.x,
            y: point.y,
        })
        .collect();
    Polygon::new(LineString::from(coords), Vec::new())
}

/// A printed line as geometry: the rectangle swept by the nozzle along the move.
pub(crate) fn strip_polygon(x0: f64, y0: f64, x1: f64, y1: f64, width: f64) -> Polygon {
    let length = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    let (dx, dy) = if length > 0.0 {
        ((x1 - x0) / length, (y1 - y0) / length)
    } else {
        (1.0, 0.0)
    };
    let half = width * 0.5;
    let px = -dy * half;
    let py = dx * half;
    let corner = |x: f64, y: f64| Coord { x, y };
    let corners = [
        corner(x0 + px, y0 + py),
        corner(x1 + px, y1 + py),
        corner(x1 - px, y1 - py),
        corner(x0 - px, y0 - py),
        corner(x0 + px, y0 + py),
    ];
    Polygon::new(LineString::from(corners.to_vec()), Vec::new())
}

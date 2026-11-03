//! Sampling one area into roofs and tips (spec 01 §5.4): the roof stack under a roof
//! candidate, which line pattern the tips are seeded on, and the small-area fallback.

use super::classify::{TipKind, status, still_valid, tip};
use super::lines::{Polyline, pattern_polylines};
use super::resample::resample;
use super::{ArcFamily, Context, RoofPiece};
use crate::polygon_ops::{IntPoint, Island, Rings, area, difference, islands};
use crate::support_model::node::Node;
use crate::support_model::settings::MINIMUM_ROOF_AREA_MM2;

/// The floor of every resampling distance, mm.
const SMALLEST_SPACING_MM: f64 = 0.1;

/// Arcs bulging more than this many reference line spacings are cut (spec 01 §5.5 item 3).
const BULGE_SPACINGS: f64 = 3.0;

pub(super) struct Sampled {
    pub tips: Vec<Node>,
    pub roofs: Vec<RoofPiece>,
}

/// Which pattern and where the points are classified.
#[derive(Clone, Copy)]
enum Plan {
    /// Base pattern on the area at the insertion layer, resampled at `connect_length`.
    Base,
    /// Base pattern, resampled at r_min/2 ("continuous tips" under a stack of depth 0).
    Continuous,
    /// Roof pattern of the stack area `index`, classified at `layer`; with `recheck`, points
    /// must also be valid one layer further down.
    Roof {
        index: usize,
        layer: usize,
        recheck: bool,
        roof_depth: usize,
    },
}

pub(super) fn sample(context: &Context, island: &Island, layer: usize, roof: bool) -> Sampled {
    let derived = &context.stage.derived;
    let mut stack = vec![island.clone()];
    let mut depth = 0;
    if roof {
        let limit = derived.roof_layers.min(layer);
        while depth < limit {
            let next = difference(&stack[depth], &context.forbidden(layer - depth - 1));
            if derived.grid.mm2(area(&next)) < MINIMUM_ROOF_AREA_MM2 {
                break;
            }
            stack.push(next);
            depth += 1;
        }
    }
    let roofs = roof_pieces(&stack, depth, layer);
    if roof && depth > 0 && depth == layer {
        // The stack reaches the bed: the remaining area is a roof on layer 0, no tips.
        let mut roofs = roofs;
        roofs.push(RoofPiece {
            layer: 0,
            interface: true,
            rings: stack[depth].clone(),
        });
        return Sampled {
            tips: Vec::new(),
            roofs,
        };
    }
    let (tips, depth) = tips_for(context, &stack, depth, layer, roof);
    let roofs = roof_pieces(&stack, depth, layer);
    Sampled { tips, roofs }
}

/// A_s on layer m − s for s < depth: the contact roof first, interfaces below it.
fn roof_pieces(stack: &[Rings], depth: usize, layer: usize) -> Vec<RoofPiece> {
    (0..depth)
        .map(|step| RoofPiece {
            layer: layer - step,
            interface: step > 0,
            rings: stack[step].clone(),
        })
        .collect()
}

/// The tips of the stack, and the depth the stack ends up with (a stack whose lines yield no
/// point at all is cut where its own lines run out).
fn tips_for(
    context: &Context,
    stack: &[Rings],
    mut depth: usize,
    layer: usize,
    roof: bool,
) -> (Vec<Node>, usize) {
    let rows = context.stage.derived.roof_layers;
    loop {
        let plan = plan_of(roof, depth, rows, layer);
        let tips = plan_tips(context, stack, plan, layer);
        if !tips.is_empty() || depth == 0 {
            return (tips, depth);
        }
        // Shorten the stack to end at the first roof whose own lines are empty.
        let cut = (1..=depth).find(|&index| roof_lines(context, &stack[index]).is_empty());
        depth = cut.map_or(0, |index| index.min(depth - 1));
    }
}

fn plan_of(roof: bool, depth: usize, rows: usize, layer: usize) -> Plan {
    if !roof {
        Plan::Base
    } else if depth == 0 {
        Plan::Continuous
    } else if depth < rows {
        Plan::Roof {
            index: depth - 1,
            layer: layer - depth + 1,
            recheck: true,
            roof_depth: depth - 1,
        }
    } else {
        Plan::Roof {
            index: depth,
            layer: layer - depth,
            recheck: false,
            roof_depth: depth,
        }
    }
}

fn plan_tips(context: &Context, stack: &[Rings], plan: Plan, layer: usize) -> Vec<Node> {
    let derived = &context.stage.derived;
    let (area_rings, class_layer, spacing, roof_depth, recheck) = match plan {
        Plan::Base => (&stack[0], layer, derived.connect_length, 0, None),
        Plan::Continuous => (&stack[0], layer, derived.tip_radius / 2, 0, None),
        Plan::Roof {
            index,
            layer: at,
            recheck,
            roof_depth,
        } => (
            &stack[index],
            at,
            derived.connect_length,
            roof_depth,
            recheck.then(|| at - 1),
        ),
    };
    let lines = match plan {
        Plan::Roof { .. } => roof_lines(context, area_rings),
        _ => base_lines(context, area_rings),
    };
    let points = line_points(context, &lines, area_rings, spacing);
    points
        .into_iter()
        .filter_map(|(point, long_line)| {
            let standing = status(context, point, class_layer)?;
            if recheck.is_some_and(|below| !still_valid(context, point, standing, below)) {
                return None;
            }
            Some(tip(
                derived,
                point,
                class_layer,
                &TipKind {
                    status: standing,
                    roof_depth,
                    long_line,
                },
            ))
        })
        .collect()
}

fn base_lines(context: &Context, region: &Rings) -> Vec<Polyline> {
    let fill = &context.stage.derived.fill;
    pattern_lines(context, region, fill.base_line_spacing, fill.base_angle)
}

fn roof_lines(context: &Context, region: &Rings) -> Vec<Polyline> {
    pattern_lines(
        context,
        region,
        context.stage.derived.fill.roof_line_spacing,
        context.stage.flavour.roof_line_angle_degrees().to_radians(),
    )
}

fn pattern_lines(context: &Context, region: &Rings, spacing: i64, angle: f64) -> Vec<Polyline> {
    let derived = &context.stage.derived;
    let reference = match context.family {
        ArcFamily::Roof => derived.fill.roof_line_spacing,
        ArcFamily::Base => derived.fill.base_line_spacing,
        ArcFamily::PerCall => spacing,
    };
    let shrink = (derived.grid.scaled(f64::from(derived.fill.flow_spacing)) / 2.0) as i64;
    islands(region)
        .iter()
        .flat_map(|part| {
            pattern_polylines(
                part,
                spacing,
                angle,
                shrink,
                BULGE_SPACINGS * reference as f64,
            )
        })
        .collect()
}

/// Tip positions along the lines; the outline of the area when the lines carry no more than
/// `need` points (spec 01 §5.4 small-area fallback).
fn line_points(
    context: &Context,
    lines: &[Polyline],
    region: &Rings,
    spacing: i64,
) -> Vec<(IntPoint, bool)> {
    let derived = &context.stage.derived;
    let smallest = derived.grid.scaled(SMALLEST_SPACING_MM);
    let mut points = Vec::new();
    for line in lines {
        let sampled = resample(line, false, spacing as f64, 1, smallest);
        let long = sampled.len() > 5;
        points.extend(sampled.into_iter().map(|point| (point, long)));
    }
    let perimeter: f64 = region.iter().map(|ring| ring_length(ring)).sum();
    let need = ((perimeter / derived.connect_length as f64) as usize).clamp(1, 3);
    if points.len() > need {
        return points;
    }
    region
        .iter()
        .flat_map(|ring| resample(ring, true, derived.connect_length as f64, need, smallest))
        .map(|point| (point, false))
        .collect()
}

fn ring_length(ring: &[IntPoint]) -> f64 {
    (0..ring.len())
        .map(|index| {
            let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
            let (dx, dy) = (
                f64::from(a.x) - f64::from(b.x),
                f64::from(a.y) - f64::from(b.y),
            );
            libm::sqrt(dx * dx + dy * dy)
        })
        .sum()
}

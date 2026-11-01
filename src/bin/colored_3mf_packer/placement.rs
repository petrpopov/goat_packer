//! Where packed objects sit on the bed: footprint loops in bed coordinates, centre-out order,
//! heat zones.

use goat_packer::{
    Footprint, PackResult, PackedPlacement, Point, Rect,
    colored_packer::PrinterProfile,
    plate_centre::{centre_out_order, loops_bounds, smallest_containing_zone, union_rect},
};

/// The footprint loops of one placement in bed coordinates — rotated by the placement's
/// angle, then translated to its position. For checking a placed object against a
/// bed-space rect such as a grown tower keepout.
pub fn placement_world_loops(footprint: &Footprint, placement: &PackedPlacement) -> Vec<Vec<Point>> {
    placement.place_loops(&footprint.loops)
}

/// World-XY bbox of one placement's footprint loops (`None` only for an empty footprint).
pub fn placement_world_bounds(footprint: &Footprint, placement: &PackedPlacement) -> Option<Rect> {
    loops_bounds(&placement_world_loops(footprint, placement))
}

/// The body loops of one placement in bed coordinates (the whole footprint when it has no
/// separate body): what a grown tower must not reach.
pub fn placement_body_loops(footprint: &Footprint, placement: &PackedPlacement) -> Vec<Vec<Point>> {
    let loops = if footprint.body_loops.is_empty() {
        &footprint.loops
    } else {
        &footprint.body_loops
    };
    placement.place_loops(loops)
}

/// World-XY bounding box of the packed layout, from the exact footprint loops rotated per
/// placement. The packer's own bed-fit check works on rasterized masks; this recomputation is
/// for the operator-facing log line that says where the layout actually sits on the bed.
pub fn layout_world_bounds(footprint: &Footprint, result: &PackResult) -> Rect {
    result
        .placements
        .iter()
        .map(|placement| Rect::of_points(placement_world_loops(footprint, placement).iter().flatten()))
        .fold(Rect::of_points([].iter()), Rect::union)
}

/// Reorders the layout centre-out: by the distance from `centre` to each placement's world
/// footprint bbox centre, ties by packing order. Any subset of a valid layout is valid, so the
/// time window's prefix then always drops the farthest object first.
pub fn order_centre_out(footprint: &Footprint, packed: &mut PackResult, centre: (f64, f64)) {
    let centres: Vec<(f64, f64)> = packed
        .placements
        .iter()
        .map(|placement| {
            placement_world_bounds(footprint, placement).map_or(
                (placement.x_mm, placement.y_mm),
                |bounds| {
                    ((bounds.min_x + bounds.max_x) * 0.5, (bounds.min_y + bounds.max_y) * 0.5)
                },
            )
        })
        .collect();
    let order = centre_out_order(&centres, centre);
    packed.placements = order.into_iter().map(|index| packed.placements[index].clone()).collect();
}

/// Spec "zone report": bbox of the kept placements ∪ the tower keepout (the first layer the
/// A2L start G-code heat-soaks) and the smallest heat zone containing it.
pub fn heat_zone_of(
    profile: &PrinterProfile,
    footprint: &Footprint,
    packed: &PackResult,
    tower_keepout: Option<Rect>,
) -> (Option<Rect>, Option<String>) {
    let bbox = packed
        .placements
        .iter()
        .filter_map(|placement| placement_world_bounds(footprint, placement))
        .chain(tower_keepout)
        .reduce(union_rect);
    let zone = bbox
        .and_then(|bbox| smallest_containing_zone(&profile.heat_zones, bbox))
        .map(|zone| zone.name.clone());
    (bbox, zone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_out_orders_placements_by_world_bbox_centre() {
        let square = [
            Point { x: -1.0, y: -1.0 },
            Point { x: 1.0, y: -1.0 },
            Point { x: 1.0, y: 1.0 },
            Point { x: -1.0, y: 1.0 },
        ];
        // Footprint off its own origin: the bbox centre, not the placement point, is ranked.
        let shifted: Vec<Point> = square.iter().map(|p| Point { x: p.x + 10.0, y: p.y }).collect();
        let footprint = Footprint {
            support_landing_loops: Vec::new(),
            layer_zones: Vec::new(),
            body_loops: vec![shifted.clone()],
            bed_loops: None,
            loops: vec![shifted],
            width: 2.0,
            height: 2.0,
            min_z: 0.0,
            max_z: 0.0,
            triangle_count: 0,
        };
        let at = |x_mm: f64, y_mm: f64, angle_deg: f64| PackedPlacement {
            x_mm,
            y_mm,
            angle_deg,
        };
        let mut packed = PackResult {
            placements: vec![at(0.0, 0.0, 0.0), at(90.0, 100.0, 0.0), at(100.0, 90.0, 180.0)],
            resolution_mm: 0.25,
            requested_clearance_mm: 0.7,
            angle_step_deg: 2.0,
            attempts: 1,
            attempts_run: 1,
            seed: 1,
        };
        // Centres: (10,0), (100,100), (90,90) -> nearest to (90,90) first.
        order_centre_out(&footprint, &mut packed, (90.0, 90.0));
        let xs: Vec<f64> = packed.placements.iter().map(|p| p.x_mm).collect();
        assert_eq!(xs, vec![100.0, 90.0, 0.0]);
    }
}

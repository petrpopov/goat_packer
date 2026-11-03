//! The order of the copies in the 3MF (design §9.7). The slicer builds one set of supports per
//! rotation class and prints it under every copy, but it clips (and, in BambuStudio, routes
//! around the bed edge) by the FIRST copy of the class. So the copy farthest from the bed
//! edge, the hard zones and the tower goes first in its class: the support prediction, made
//! for an object standing clear of everything, is then the one every copy prints.

use goat_packer::{
    PrintPlacement, Rect,
    keepout::ZoneClass,
    kind_masks::{PlateZone, ZoneArea},
    object_print::ObjectPrint,
    plate_centre::{centre_out_order, loops_bounds},
};
use serde::{Deserialize, Serialize};

/// BambuStudio routes branches around the bed edge, exclusion zones and the P1S cutter as one
/// obstacle (design §9.7, 02 §2.2): a branch's drawn material stays at least this far from it
/// for a full 3.73 mm foot (2.70 mm for the widest radius key), so a canonical copy closer
/// than this may be bent by the obstacle and its realization is not the interior one.
pub const BAMBU_OBSTACLE_MM: f64 = 2.7;

/// What the canonical copy of one class stands clear of.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Canonical {
    pub rotation_deg: f64,
    /// Position of the copy in the 3MF's object order.
    pub index: usize,
    /// Distance from its envelope's box to the nearest barrier, mm.
    pub clearance_mm: f64,
    /// Closer than the flavour's obstacle reach: verification is recommended.
    pub near_edge: bool,
}

/// Something the slicer treats as an obstacle or clips by: the printable boundary and hard
/// edge bands (stay inside), hard zones and the tower body (stay out).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Barrier {
    Inside { inside: Rect, inset_mm: f64 },
    Outside(Rect),
}

/// The barriers of the plate: its hard and boundary zones and the tower's body.
pub fn barriers(zones: &[PlateZone]) -> Vec<Barrier> {
    zones
        .iter()
        .filter(|zone| matches!(zone.class, ZoneClass::Boundary | ZoneClass::Hard | ZoneClass::TowerBody))
        .filter_map(|zone| match &zone.area {
            ZoneArea::Outside { inside, material_inset_mm } => Some(Barrier::Inside {
                inside: *inside,
                inset_mm: *material_inset_mm,
            }),
            ZoneArea::Rect(rect) => Some(Barrier::Outside(*rect)),
            ZoneArea::Loops(loops) => loops_bounds(loops).map(Barrier::Outside),
        })
        .collect()
}

/// The box around everything the print puts on the plate, relative to its frame origin.
fn envelope(print: &ObjectPrint) -> Option<Rect> {
    let flat = [&print.silhouette, &print.landing, &print.support_base];
    let banded = print.support.iter().chain(&print.interface).chain(&print.body);
    let loops: Vec<_> = flat.into_iter().chain(banded).flatten().cloned().collect();
    loops_bounds(&loops)
}

fn distance_between(a: Rect, b: Rect) -> f64 {
    let dx = (a.min_x - b.max_x).max(b.min_x - a.max_x).max(0.0);
    let dy = (a.min_y - b.max_y).max(b.min_y - a.max_y).max(0.0);
    dx.hypot(dy)
}

/// How far `envelope` stands from `barrier`: the margin left inside a boundary, the gap to a
/// zone; negative when it breaks the barrier.
fn clearance(envelope: Rect, barrier: Barrier) -> f64 {
    match barrier {
        Barrier::Inside { inside, inset_mm } => [
            envelope.min_x - inside.min_x,
            envelope.min_y - inside.min_y,
            inside.max_x - envelope.max_x,
            inside.max_y - envelope.max_y,
        ]
        .into_iter()
        .fold(f64::MAX, f64::min)
            - inset_mm,
        Barrier::Outside(zone) => distance_between(envelope, zone),
    }
}

/// Moves the copy of each class with the most clearance to the first position the class has,
/// swapping it with the copy that stood there. Returns what was chosen per class, classes in
/// order of their first appearance.
pub fn canonical_first(
    prints: &[ObjectPrint],
    barriers: &[Barrier],
    near_edge_mm: f64,
    placements: &mut [PrintPlacement],
) -> Vec<Canonical> {
    let envelopes: Vec<Option<Rect>> = prints.iter().map(envelope).collect();
    let clearance_of = |placement: &PrintPlacement| {
        let Some(own) = envelopes[placement.print] else {
            return f64::MAX;
        };
        let placed = own.translated(placement.x_mm, placement.y_mm);
        barriers
            .iter()
            .map(|&barrier| clearance(placed, barrier))
            .fold(f64::MAX, f64::min)
    };
    let mut chosen = Vec::new();
    for (class, print) in prints.iter().enumerate() {
        let members: Vec<usize> = (0..placements.len())
            .filter(|&index| placements[index].print == class)
            .collect();
        let Some(&first) = members.first() else {
            continue;
        };
        let clearances: Vec<f64> = members.iter().map(|&i| clearance_of(&placements[i])).collect();
        // The first of equals: the order stays as the packer made it where it cannot matter.
        let best = (0..members.len())
            .reduce(|best, next| if clearances[next] > clearances[best] { next } else { best })
            .expect("a class with members");
        placements.swap(first, members[best]);
        chosen.push(Canonical {
            rotation_deg: print.rotation_deg,
            index: first,
            clearance_mm: clearances[best],
            near_edge: clearances[best] < near_edge_mm,
        });
    }
    chosen.sort_by_key(|canonical| canonical.index);
    chosen
}

/// Orders the copies by the distance of their envelope's centre from `centre`, so that a time
/// window's prefix is a compact disc (any subset of a valid layout is valid).
pub fn centre_out(prints: &[ObjectPrint], placements: &mut Vec<PrintPlacement>, centre: (f64, f64)) {
    let centres: Vec<(f64, f64)> = placements
        .iter()
        .map(|placement| {
            envelope(&prints[placement.print]).map_or((placement.x_mm, placement.y_mm), |own| {
                let placed = own.translated(placement.x_mm, placement.y_mm);
                ((placed.min_x + placed.max_x) * 0.5, (placed.min_y + placed.max_y) * 0.5)
            })
        })
        .collect();
    let order = centre_out_order(&centres, centre);
    *placements = order.into_iter().map(|index| placements[index]).collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use goat_packer::Point;

    fn square_print(rotation_deg: f64) -> ObjectPrint {
        let p = |x: f64, y: f64| Point { x, y };
        ObjectPrint {
            rotation_deg,
            silhouette: vec![vec![p(-5.0, -5.0), p(5.0, -5.0), p(5.0, 5.0), p(-5.0, 5.0), p(-5.0, -5.0)]],
            ..ObjectPrint::default()
        }
    }

    fn at(print: usize, x_mm: f64, y_mm: f64) -> PrintPlacement {
        PrintPlacement { print, x_mm, y_mm }
    }

    fn plate() -> Vec<Barrier> {
        vec![Barrier::Inside {
            inside: Rect::new(0.0, 0.0, 100.0, 100.0),
            inset_mm: 0.0,
        }]
    }

    #[test]
    fn the_copy_with_the_most_clearance_takes_the_first_place_of_its_class() {
        let prints = [square_print(0.0), square_print(180.0)];
        // Class 0 at the edge, class 1, class 0 in the middle, class 0 near the edge.
        let mut placements = vec![at(0, 6.0, 50.0), at(1, 20.0, 20.0), at(0, 50.0, 50.0), at(0, 90.0, 50.0)];
        let chosen = canonical_first(&prints, &plate(), BAMBU_OBSTACLE_MM, &mut placements);
        assert_eq!(placements[0], at(0, 50.0, 50.0));
        assert_eq!(placements[2], at(0, 6.0, 50.0));
        assert_eq!(chosen.len(), 2);
        assert_eq!((chosen[0].rotation_deg, chosen[0].index), (0.0, 0));
        assert!((chosen[0].clearance_mm - 45.0).abs() < 1e-9);
        assert_eq!((chosen[1].rotation_deg, chosen[1].index), (180.0, 1));
        assert!(!chosen[0].near_edge && !chosen[1].near_edge);
    }

    #[test]
    fn a_lone_copy_near_the_edge_is_flagged_and_a_tower_counts_as_a_barrier() {
        let prints = [square_print(0.0)];
        let mut placements = vec![at(0, 6.0, 50.0)];
        let chosen = canonical_first(&prints, &plate(), BAMBU_OBSTACLE_MM, &mut placements);
        assert!(chosen[0].near_edge);
        let tower = Barrier::Outside(Rect::new(16.0, 40.0, 30.0, 60.0));
        let both = [plate()[0], tower];
        let chosen = canonical_first(&prints, &both, 0.0, &mut placements);
        assert!((chosen[0].clearance_mm - 1.0).abs() < 1e-9, "{chosen:?}");
    }

    #[test]
    fn centre_out_puts_the_nearest_envelope_first() {
        let prints = [square_print(0.0)];
        let mut placements = vec![at(0, 90.0, 90.0), at(0, 50.0, 55.0), at(0, 10.0, 20.0)];
        centre_out(&prints, &mut placements, (50.0, 50.0));
        assert_eq!(placements, vec![at(0, 50.0, 55.0), at(0, 10.0, 20.0), at(0, 90.0, 90.0)]);
    }
}

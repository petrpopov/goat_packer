//! Moving touching objects apart instead of repacking the plate.

use std::collections::BTreeMap;

use super::touching::TOUCH_EPSILON_MM;
use crate::options::PackedPlacement;

/// Extra distance a nudge adds on top of the measured overlap: the slicer regenerates the
/// supports on the nudged plate, and their reach drifts by a few tenths of a millimetre.
pub const NUDGE_MARGIN_MM: f64 = 0.3;

/// Directions tried for one contact, as offsets from the line between the two objects.
const NUDGE_ANGLES_DEG: [f64; 7] = [0.0, 30.0, -30.0, 60.0, -60.0, 90.0, -90.0];

/// Moves touching objects apart instead of repacking the plate: for each contact (support of
/// `contacts[i].0` touching the body of `.1`, smallest gap `.2` mm, negative = overlap) the pair
/// separates by the overlap plus `NUDGE_MARGIN_MM`, along the line between the two objects or
/// up to 90° off it, by the first move that keeps the whole layout `valid` (the packer's own
/// rule: neighbours, bed, keepouts, tower). Moves, in order: one of the two objects alone, both
/// by half, then everything on one side of the pair (the object and every placement beyond it
/// in that direction) — dense plates touch in chains, where a lone object only runs into the
/// next one. Pairs are handled once (both directions, smallest gap), and a pair an earlier move
/// already separated enough is skipped. `None` when a contact finds no valid move: the caller
/// falls back to a repack.
pub fn nudge_apart(
    placements: &[PackedPlacement],
    contacts: &[(usize, usize, f64)],
    valid: impl Fn(&[PackedPlacement]) -> bool,
) -> Option<Vec<PackedPlacement>> {
    let mut pairs: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for &(a, b, gap) in contacts {
        if a == b || a >= placements.len() || b >= placements.len() {
            return None;
        }
        let entry = pairs.entry((a.min(b), a.max(b))).or_insert(gap);
        *entry = entry.min(gap);
    }
    let offset = |layout: &[PackedPlacement], a: usize, b: usize| {
        (
            layout[b].x_mm - layout[a].x_mm,
            layout[b].y_mm - layout[a].y_mm,
        )
    };
    let mut current = placements.to_vec();
    for ((a, b), gap) in pairs {
        let need = (TOUCH_EPSILON_MM - gap).max(0.0) + NUDGE_MARGIN_MM;
        let (ox, oy) = offset(placements, a, b);
        let length = ox.hypot(oy);
        let (ux, uy) = if length > 1e-9 {
            (ox / length, oy / length)
        } else {
            (1.0, 0.0)
        };
        // Separation already gained along the pair's original direction.
        let (cx, cy) = offset(&current, a, b);
        if (cx - ox) * ux + (cy - oy) * uy >= need - 1e-9 {
            continue;
        }
        let base = uy.atan2(ux);
        let moved = NUDGE_ANGLES_DEG.iter().find_map(|degrees| {
            let angle = base + degrees.to_radians();
            let (dx, dy) = (angle.cos(), angle.sin());
            let side = |layout: &[PackedPlacement], pivot: usize, sign: f64| -> Vec<usize> {
                (0..layout.len())
                    .filter(|&index| {
                        let along = (layout[index].x_mm - layout[pivot].x_mm) * dx
                            + (layout[index].y_mm - layout[pivot].y_mm) * dy;
                        index == pivot || along * sign > 1e-9
                    })
                    .collect()
            };
            // (objects moved forward along d, objects moved back, distance each).
            let moves: [(Vec<usize>, Vec<usize>, f64); 5] = [
                (vec![b], vec![], need),
                (vec![], vec![a], need),
                (vec![b], vec![a], need * 0.5),
                (side(&current, b, 1.0), vec![], need),
                (vec![], side(&current, a, -1.0), need),
            ];
            moves.into_iter().find_map(|(forward, back, distance)| {
                if forward.contains(&a) || back.contains(&b) {
                    return None;
                }
                let mut candidate = current.clone();
                for &index in &forward {
                    candidate[index].x_mm += dx * distance;
                    candidate[index].y_mm += dy * distance;
                }
                for &index in &back {
                    candidate[index].x_mm -= dx * distance;
                    candidate[index].y_mm -= dy * distance;
                }
                valid(&candidate).then_some(candidate)
            })
        })?;
        current = moved;
    }
    Some(current)
}

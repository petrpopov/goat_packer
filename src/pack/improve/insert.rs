//! Where one more copy could be squeezed in: the origins with the least overlap against the
//! layout, over the whole plate; rebuilt whenever the layout changes.

use super::separate::Field;
use super::windows::Window;
use crate::layout::PixelPlacement;

/// Two tries start at least this far apart (max norm, pixels).
const SPREAD: i32 = 12;
/// Origins whose summed depth is above this never start a try: nothing a few moves could
/// resolve, and keeping them would make the map as large as the plate.
const MAX_COST: u32 = 300;
/// Items are bucketed in squares of this many pixels so each origin sees only its neighbours.
const BUCKET: i32 = 64;

/// Every sampled free origin with its cost, cheapest first (ties: lower row, column, class).
pub(super) struct CostMap {
    cells: Vec<(u32, PixelPlacement)>,
}

impl CostMap {
    /// The origins of `area` sampled every `step` pixels, each class, with their summed
    /// penetration depth against `items`.
    pub(super) fn new(
        field: &Field,
        items: &[PixelPlacement],
        area: Window,
        step: usize,
    ) -> CostMap {
        let mut cells: Vec<(u32, PixelPlacement)> = Vec::new();
        // The neighbours of each bucket of the window, found on first use (shared by the
        // classes).
        let (bx0, by0) = (area.x0.div_euclid(BUCKET), area.y0.div_euclid(BUCKET));
        let columns = (area.x1.div_euclid(BUCKET) - bx0 + 1).max(0) as usize;
        let rows = (area.y1.div_euclid(BUCKET) - by0 + 1).max(0) as usize;
        let mut buckets: Vec<Option<Vec<usize>>> = vec![None; columns * rows];
        for class in 0..field.depth.classes() {
            for y in (area.y0..=area.y1).step_by(step) {
                for x in (area.x0..=area.x1).step_by(step) {
                    if !field.free[class].contains(x, y) {
                        continue;
                    }
                    let key = (x.div_euclid(BUCKET), y.div_euclid(BUCKET));
                    let slot = (key.1 - by0) as usize * columns + (key.0 - bx0) as usize;
                    let near = buckets[slot].get_or_insert_with(|| {
                        let centre = (key.0 * BUCKET + BUCKET / 2, key.1 * BUCKET + BUCKET / 2);
                        field.neighbours(items, centre.0, centre.1, BUCKET / 2, None)
                    });
                    if let Some(cost) = field.overlap_at_most(items, near, class, x, y, MAX_COST) {
                        cells.push((cost, PixelPlacement { class, x, y }));
                    }
                }
            }
        }
        cells.sort_unstable_by_key(|&(cost, p)| (cost, p.y, p.x, p.class));
        CostMap { cells }
    }

    /// The cheapest origins, at most `limit`, spread apart.
    pub(super) fn spots(&self, limit: usize) -> Vec<(u32, PixelPlacement)> {
        let mut picked: Vec<(u32, PixelPlacement)> = Vec::new();
        if limit == 0 {
            return picked;
        }
        for &(cost, p) in &self.cells {
            let apart = picked
                .iter()
                .all(|(_, q)| (q.x - p.x).abs().max((q.y - p.y).abs()) >= SPREAD);
            if apart {
                picked.push((cost, p));
                if picked.len() == limit {
                    break;
                }
            }
        }
        picked
    }
}

//! What every member of a run shares (design 2026-11-02, §9.1): the kind masks of each
//! rotation class, the no-fit set of each class pair (computed once, on first use), the base
//! free map of each class and the tower as a removable layer per anchor.
//!
//! Frame convention: a class is an `ObjectPrint` already in the plate-aligned frame of its
//! rotation (the mesh turned by `rotation_deg` about the frame centre; supports are computed
//! in that frame because they are not rotation-equivariant), or a legacy footprint turned
//! rigidly to one angle. Either way a copy is a pure translation of its class: a pixel
//! placement is the position of the class's frame origin on the origin grid.

use super::free_map::FreeMap;
use super::no_fit::no_fit;
use super::select::{Candidate, Member, RankKey, TieKey};
use crate::geometry::Point;
use crate::kind_masks::{
    FeasibleOrigins, KindMasks, OriginGrid, OriginSet, PlateZone, feasible_origins,
    forbidden_origins,
};
use crate::layout::{PixelPlacement, bbox_area_px, extent_px};
use crate::span_raster::SpanRaster;
use rayon::prelude::*;
use std::sync::OnceLock;

/// The tower at one anchor: its zones (plate frame) and, per class, the origins they forbid.
pub struct TowerLayer {
    /// The anchor (`wipe_tower_x/y`: the body's min corner as the slicer names it), mm.
    pub anchor: Point,
    pub zones: Vec<PlateZone>,
    forbidden: Vec<OriginSet>,
}

impl TowerLayer {
    /// The origins of `class` the tower forbids at this anchor (inside the base map).
    pub fn forbidden(&self, class: usize) -> &OriginSet {
        &self.forbidden[class]
    }
}

/// The shared state of one run over a set of rotation classes.
pub struct Ctx<'a> {
    zones: &'a [PlateZone],
    grid: OriginGrid,
    /// The run's class id of each local class (a legacy run numbers its angles globally).
    ids: Vec<usize>,
    masks: Vec<KindMasks>,
    base: Vec<FeasibleOrigins>,
    free: Vec<FreeMap>,
    extents: Vec<(i64, i64, i64, i64)>,
    no_fit: Vec<OnceLock<SpanRaster>>,
    towers: Vec<TowerLayer>,
}

impl<'a> Ctx<'a> {
    /// Classes given by their masks (phase 0) and run ids, on `zones` and `grid`. The base
    /// maps are built in parallel and collected in class order.
    pub(crate) fn new(
        masks: Vec<KindMasks>,
        ids: Vec<usize>,
        zones: &'a [PlateZone],
        grid: OriginGrid,
    ) -> Ctx<'a> {
        assert_eq!(masks.len(), ids.len());
        let base: Vec<FeasibleOrigins> = masks
            .par_iter()
            .map(|m| feasible_origins(m, zones, &grid))
            .collect();
        let free = base.iter().map(FreeMap::new).collect();
        let extents = masks
            .iter()
            .map(|m| extent_px(m.extent_bounds, grid.resolution))
            .collect();
        let pairs = masks.len() * masks.len();
        Ctx {
            zones,
            grid,
            ids,
            masks,
            base,
            free,
            extents,
            no_fit: (0..pairs).map(|_| OnceLock::new()).collect(),
            towers: Vec::new(),
        }
    }

    /// Adds the tower at each `(anchor, zones)` as a removable layer: the origins its zones
    /// forbid each class, inside the base map. Units (anchor, class) run in parallel and are
    /// collected in order.
    pub(crate) fn add_towers(&mut self, anchors: Vec<(Point, Vec<PlateZone>)>) {
        let classes = self.masks.len();
        let units: Vec<(usize, usize)> = (0..anchors.len())
            .flat_map(|a| (0..classes).map(move |c| (a, c)))
            .collect();
        let sets: Vec<OriginSet> = units
            .par_iter()
            .map(|&(a, c)| {
                forbidden_origins(&self.masks[c], &anchors[a].1, &self.grid, &self.base[c])
            })
            .collect();
        let mut sets = sets.into_iter();
        for (anchor, zones) in anchors {
            let forbidden = sets.by_ref().take(classes).collect();
            self.towers.push(TowerLayer {
                anchor,
                zones,
                forbidden,
            });
        }
    }

    pub fn classes(&self) -> usize {
        self.masks.len()
    }

    pub fn grid(&self) -> &OriginGrid {
        &self.grid
    }

    pub fn zones(&self) -> &[PlateZone] {
        self.zones
    }

    pub fn masks(&self, class: usize) -> &KindMasks {
        &self.masks[class]
    }

    pub fn towers(&self) -> &[TowerLayer] {
        &self.towers
    }

    /// The tower anchors as the members see them: `None` once without a tower.
    pub fn anchor_slots(&self) -> Vec<Option<usize>> {
        if self.towers.is_empty() {
            vec![None]
        } else {
            (0..self.towers.len()).map(Some).collect()
        }
    }

    /// Where a copy of class `b` may not go relative to one of class `a`.
    pub(crate) fn no_fit(&self, a: usize, b: usize) -> &SpanRaster {
        let n = self.masks.len();
        self.no_fit[a * n + b].get_or_init(|| no_fit(&self.masks[a], &self.masks[b]))
    }

    /// Every no-fit set, computed in parallel: members that need all pairs call this first
    /// so the sets are not built one by one on demand.
    pub(crate) fn prepare_no_fit(&self) {
        let n = self.masks.len();
        (0..n * n).into_par_iter().for_each(|i| {
            self.no_fit(i / n, i % n);
        });
    }

    /// The free map of `class` before anything is placed, with the tower at `anchor`.
    pub fn free_map(&self, class: usize, anchor: Option<usize>) -> FreeMap {
        let mut free = self.free[class].clone();
        if let Some(anchor) = anchor {
            free.remove(self.towers[anchor].forbidden(class));
        }
        free
    }

    /// The base map of `class` (plate zones only), with its bounds.
    pub fn origins(&self, class: usize) -> &FeasibleOrigins {
        &self.base[class]
    }

    /// Area in pixels of the box around every copy's printed extent (local classes).
    pub(crate) fn bbox_area(&self, layout: &[PixelPlacement]) -> u64 {
        bbox_area_px(layout, &self.extents)
    }

    /// A member's layout (local classes) as a ranked candidate in the run's class ids.
    pub(crate) fn candidate(
        &self,
        member: Member,
        anchor: Option<usize>,
        detail: Vec<i64>,
        layout: &[PixelPlacement],
    ) -> Candidate {
        let key = RankKey {
            count: layout.len(),
            robustness_quarter_mm: 0,
            canonical_clear: true,
            bbox_area_px: self.bbox_area(layout),
            tie: TieKey {
                member,
                anchor: anchor.map_or(0, |a| a as u32),
                detail,
            },
        };
        let placements = layout
            .iter()
            .map(|p| PixelPlacement {
                class: self.ids[p.class],
                ..*p
            })
            .collect();
        Candidate {
            key,
            anchor,
            placements,
        }
    }
}

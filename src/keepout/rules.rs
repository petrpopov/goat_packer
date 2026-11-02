//! The rules of one plate: the printable area, the edge bands and the profile's keepouts.

use super::{Keepout, KeepoutClass, KeepoutFrame, KeepoutShape};
use crate::geometry::Rect;
use serde::{Deserialize, Serialize};

/// Which bodies of different objects keep clearance from each other (design §7.3, Q14).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyRule {
    /// Whole silhouettes: strict, and what every layout has been validated against so far.
    #[default]
    Silhouette,
    /// Body against body within the same height band: physically valid for FDM, looser.
    Bands,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlateRules {
    pub printable_area: Rect,
    /// Restricted band along the printable edges: part boxes keep at least this distance
    /// from the edge (equality passes).
    pub body_edge_band_mm: f64,
    /// Supports and the tower stay inside the printable area inset by this.
    pub print_edge_reserve_mm: f64,
    pub keepouts: Vec<Keepout>,
    #[serde(default)]
    pub body_rule: BodyRule,
}

impl PlateRules {
    /// Today's `PackOptions` bed fields as rules, so the new core can run on a legacy job.
    /// The mapping is lossy in two known ways. Purge-line strips arrive as ordinary
    /// exclusions, so they become hard instead of soft; and `bed` is already inset by the
    /// edge margin, so the reserve is 0. A `body_bed` that is inset unevenly becomes the
    /// widest of its insets, which is the safe side.
    pub fn from_legacy(
        bed: Rect,
        body_bed: Option<Rect>,
        exclusions: &[Rect],
        body_exclusions: &[Rect],
    ) -> Self {
        let keepouts_of = |class, prefix: &str, rects: &[Rect]| {
            let keepouts = rects.iter().enumerate().map(|(index, rect)| Keepout {
                id: format!("{prefix}_{index}"),
                class,
                shape: KeepoutShape::Rect(*rect),
                z_range_mm: None,
                frame: KeepoutFrame::Plate,
            });
            keepouts.collect::<Vec<_>>()
        };
        // The tower keepout is checked against the body only: supports may enter it.
        let keepouts = [
            keepouts_of(KeepoutClass::Hard, "legacy_exclusion", exclusions),
            keepouts_of(
                KeepoutClass::Restricted,
                "legacy_body_exclusion",
                body_exclusions,
            ),
        ]
        .concat();
        Self {
            printable_area: bed,
            body_edge_band_mm: body_bed.map_or(0.0, |inner| widest_inset(bed, inner)),
            print_edge_reserve_mm: 0.0,
            keepouts,
            body_rule: BodyRule::default(),
        }
    }
}

/// The largest distance from a side of `outer` to the same side of `inner`, never negative.
fn widest_inset(outer: Rect, inner: Rect) -> f64 {
    [
        inner.min_x - outer.min_x,
        inner.min_y - outer.min_y,
        outer.max_x - inner.max_x,
        outer.max_y - inner.max_y,
    ]
    .into_iter()
    .fold(0.0, f64::max)
}

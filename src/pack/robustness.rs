//! Robustness of a layout (design 2026-11-02, §9.4, work package P6): how much extra gap every
//! pair rule could take before the layout breaks. Supports are only known as the union of a
//! few probe slices, so among layouts with the same count the one with the most spare room
//! is the safer one; this module measures that room, the improvement pass spends spare room
//! on it (`improve::polish`), and `select` ranks by it right after the count.
//!
//! The measure is the Chebyshev slack of a pair's offset to its no-fit set: the largest `s`
//! such that every offset within `s` pixels (max norm) still keeps the pair legal, which is
//! the margin a uniform inflation of the pair's zones by `s` pixels would leave. The slack of
//! a layout is the least slack of any pair of copies and of any copy to the tower; the plate
//! edges are not counted (their gaps are the plate rules', not support noise).

use super::context::Ctx;
use super::select::Candidate;
use crate::layout::PixelPlacement;
use crate::span_raster::SpanRaster;

/// Slack beyond this many millimetres is not told apart: the support probes scatter by far
/// less, and the search for more room stops here.
pub(crate) const MAX_MARGIN_MM: f64 = 1.0;
/// The rank key counts slack in units of this many millimetres.
const QUARTER_MM: f64 = 0.25;

/// The slack cap in pixels at `resolution`.
pub(crate) fn cap_px(resolution: f64) -> i32 {
    (MAX_MARGIN_MM / resolution).ceil() as i32
}

/// A layout's slack in pixels, at most `cap_px`.
pub(crate) fn layout_slack(ctx: &Ctx, anchor: Option<usize>, layout: &[PixelPlacement]) -> i32 {
    let cap = cap_px(ctx.grid().resolution);
    let mut slack = cap;
    for (i, a) in layout.iter().enumerate() {
        for b in &layout[i + 1..] {
            let nfp = ctx.no_fit(a.class, b.class);
            slack = slack.min(offset_slack(nfp, b.x - a.x, b.y - a.y, slack));
        }
        if let Some(t) = anchor {
            let forbidden = &ctx.towers()[t].forbidden(a.class).spans;
            slack = slack.min(offset_slack(forbidden, a.x, a.y, slack));
        }
    }
    slack
}

/// The slack as the rank key counts it: quarter millimetres.
pub(crate) fn quarter_mm(slack_px: i32, resolution: f64) -> u8 {
    (f64::from(slack_px.max(0)) * resolution / QUARTER_MM).floor() as u8
}

/// Sets the robustness of the candidates that can reach the shortlist: those whose count is
/// at least the `keep`-th best.
pub(crate) fn annotate(ctx: &Ctx, candidates: &mut [Candidate], keep: usize) {
    let mut counts: Vec<usize> = candidates.iter().map(|c| c.key.count).collect();
    counts.sort_unstable_by(|a, b| b.cmp(a));
    let Some(&floor) = counts.get(keep.saturating_sub(1)).or(counts.last()) else {
        return;
    };
    let resolution = ctx.grid().resolution;
    for c in candidates.iter_mut().filter(|c| c.key.count >= floor) {
        let slack = layout_slack(ctx, c.anchor, &c.placements);
        c.key.robustness_quarter_mm = quarter_mm(slack, resolution);
    }
}

/// Slack of the offset `(dx, dy)` to `set`, at most `cap`: the number of rings around the
/// offset that hold no pixel of the set. 0 for an offset inside the set.
fn offset_slack(set: &SpanRaster, dx: i32, dy: i32, cap: i32) -> i32 {
    if set.contains(dx, dy) {
        return 0;
    }
    (1..=cap)
        .find(|&r| square_hits(set, dx, dy, r))
        .map_or(cap, |r| r - 1)
}

/// True when the square of radius `r` (max norm) around `(x, y)` holds a pixel of `set`.
fn square_hits(set: &SpanRaster, x: i32, y: i32, r: i32) -> bool {
    (y - r..=y + r).any(|row| {
        let spans = set.row(row);
        let after = spans.partition_point(|&(start, _)| start <= x + r);
        after > 0 && spans[after - 1].1 > x - r
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_counts_the_free_rings_around_an_offset() {
        let block = SpanRaster::rect(0, 0, 9, 9);
        assert_eq!(offset_slack(&block, 5, 5, 8), 0, "inside");
        assert_eq!(offset_slack(&block, 10, 5, 8), 0, "touching");
        assert_eq!(offset_slack(&block, 11, 5, 8), 1);
        assert_eq!(
            offset_slack(&block, 14, 14, 8),
            4,
            "the diagonal counts in max norm"
        );
        assert_eq!(offset_slack(&block, 40, 5, 8), 8, "capped");
    }

    #[test]
    fn quarter_millimetres_follow_the_resolution() {
        assert_eq!(quarter_mm(0, 0.25), 0);
        assert_eq!(quarter_mm(3, 0.25), 3);
        assert_eq!(quarter_mm(3, 0.5), 6);
        assert_eq!(quarter_mm(-1, 0.25), 0);
    }
}

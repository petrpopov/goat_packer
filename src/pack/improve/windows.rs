//! The plate as a rectangle of origins. The improvement once searched corner, edge and tower
//! windows with only the copies inside them mobile; moving every copy was stronger (a chain of
//! neighbours has to give way, and the slack it needs sits on the far side of the plate) and
//! cheaper, because only copies that overlap move at all (P1S 48 -> 49, U1 59 -> 60, and the
//! A2L plate with the tower in the corner 77 -> 79 where the windows stopped at 77).

use crate::pack::context::Ctx;

/// An inclusive rectangle of origins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Window {
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) x1: i32,
    pub(crate) y1: i32,
}

/// The bounding box of the origins any class may take; `None` when there are none.
pub(super) fn plate_window(ctx: &Ctx) -> Option<Window> {
    (0..ctx.classes())
        .filter_map(|c| ctx.origins(c).bounds())
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
        .map(|(x0, y0, x1, y1)| Window { x0, y0, x1, y1 })
}

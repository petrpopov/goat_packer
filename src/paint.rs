//! Painted support enforcers and blockers (and colour paint) on mesh facets (design §5.2
//! stage A3, spec 01 §2): `decode` turns one triangle's code into leaf sub-triangles with
//! their state; `project` puts the down-facing leaves of one state onto the layer grid.

mod decode;
mod project;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use decode::decode_leaves;
pub use project::{Facing, PaintLayers, facing, project_paint};

/// The state of a painted leaf (spec 01 §2.2 step 3). Support paint uses 0..=2; colour
/// paint (`paint_color`) stores the 1-based filament index in the same field, so read it
/// with `raw` there (the goat: filaments 1-3).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaintState {
    None,
    Enforcer,
    Blocker,
    /// Colour paint: the raw state, 3 and up.
    Colour(u32),
}

impl PaintState {
    pub fn from_raw(raw: u32) -> PaintState {
        match raw {
            0 => PaintState::None,
            1 => PaintState::Enforcer,
            2 => PaintState::Blocker,
            other => PaintState::Colour(other),
        }
    }

    pub fn raw(self) -> u32 {
        match self {
            PaintState::None => 0,
            PaintState::Enforcer => 1,
            PaintState::Blocker => 2,
            PaintState::Colour(raw) => raw,
        }
    }
}

/// How a leaf state of 3 or more continues in the code stream; the two only disagree for
/// states of 18 and up, i.e. colour paint (spec 01 §2.2).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateCoding {
    /// Bambu Studio: every following code equal to 15 adds 15; the first code below 15 ends.
    Bambu,
    /// OrcaSlicer and Snapmaker Orca: exactly one more code.
    Orca,
}

/// One leaf of a painted triangle: object-frame corners in the order the split rules give.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintedLeaf {
    pub corners: [[f64; 3]; 3],
    pub state: PaintState,
    /// 0 for an unsplit triangle.
    pub depth: u8,
}

/// A paint code that cannot be decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaintError {
    /// A character that is not a hex digit.
    NotHex(char),
    /// The code ends inside a node.
    Truncated,
    /// A split names a special corner that does not exist (or a 3-side split not at 0).
    BadSplit,
    /// Deeper than any brush paints: the code is corrupt.
    TooDeep,
}

impl fmt::Display for PaintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaintError::NotHex(character) => {
                write!(
                    formatter,
                    "paint code has a non-hex character {character:?}"
                )
            }
            PaintError::Truncated => write!(formatter, "paint code ends inside a triangle"),
            PaintError::BadSplit => write!(formatter, "paint code has an invalid split"),
            PaintError::TooDeep => write!(formatter, "paint code splits too deep"),
        }
    }
}

impl std::error::Error for PaintError {}

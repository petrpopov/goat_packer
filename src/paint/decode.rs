//! The painted-facet code of one triangle (spec 01 §2.2-2.3) decoded into leaf
//! sub-triangles with their state.
//!
//! The code is a hex string read from its last character to its first; each character is
//! one 4-bit code. A code's low two bits give the number of split sides (0 = leaf). A split
//! node names its special corner in the high two bits and is followed by its children, the
//! last child first. A leaf's high two bits are its state; the value 3 continues in the next
//! code(s), which is where the flavours differ (`StateCoding`).

use super::{PaintError, PaintState, PaintedLeaf, StateCoding};

/// Deeper splits than this are rejected as corrupt; the goat paints to depth 6 and the
/// slicer's brush stops far earlier than 24 levels.
const MAX_DEPTH: u8 = 24;

/// The leaves of `code` on the triangle `corners` (in stored vertex order), in stream order.
pub fn decode_leaves(
    code: &str,
    corners: [[f64; 3]; 3],
    coding: StateCoding,
) -> Result<Vec<PaintedLeaf>, PaintError> {
    let mut stream = CodeStream::new(code);
    let mut leaves = Vec::new();
    read_node(&mut stream, corners, 0, coding, &mut leaves)?;
    Ok(leaves)
}

/// The 4-bit codes of a paint string in reading order (last character first).
struct CodeStream<'a> {
    characters: std::iter::Rev<std::str::Chars<'a>>,
}

impl<'a> CodeStream<'a> {
    fn new(code: &'a str) -> Self {
        CodeStream {
            characters: code.chars().rev(),
        }
    }

    fn next_code(&mut self) -> Result<u8, PaintError> {
        let character = self.characters.next().ok_or(PaintError::Truncated)?;
        character
            .to_digit(16)
            .map(|value| value as u8)
            .ok_or(PaintError::NotHex(character))
    }
}

fn read_node(
    stream: &mut CodeStream<'_>,
    corners: [[f64; 3]; 3],
    depth: u8,
    coding: StateCoding,
    leaves: &mut Vec<PaintedLeaf>,
) -> Result<(), PaintError> {
    let code = stream.next_code()?;
    let split_sides = code & 0b11;
    let high = code >> 2;
    if split_sides == 0 {
        let state = leaf_state(stream, high, coding)?;
        leaves.push(PaintedLeaf {
            corners,
            state,
            depth,
        });
        return Ok(());
    }
    if depth >= MAX_DEPTH {
        return Err(PaintError::TooDeep);
    }
    let children = split(corners, split_sides, high)?;
    for child in children.iter().rev() {
        read_node(stream, *child, depth + 1, coding, leaves)?;
    }
    Ok(())
}

/// A leaf's state: 0 none, 1 enforcer, 2 blocker, 3 and up colours. A high value of 3
/// continues: Orca-family slicers add one more code; Bambu adds 15 for every following code
/// equal to 15 and then the first code below 15 (spec 01 §2.2).
fn leaf_state(
    stream: &mut CodeStream<'_>,
    high: u8,
    coding: StateCoding,
) -> Result<PaintState, PaintError> {
    if high < 3 {
        return Ok(PaintState::from_raw(u32::from(high)));
    }
    let mut extra = 0u32;
    let mut next = stream.next_code()?;
    if coding == StateCoding::Bambu {
        while next == 15 {
            extra += 15;
            next = stream.next_code()?;
        }
    }
    Ok(PaintState::from_raw(3 + extra + u32::from(next)))
}

/// Children of a split node in storage order (spec 01 §2.3). The corners are first turned
/// so the special corner leads: a = v_s, b = v_(s+1), c = v_(s+2).
fn split(
    corners: [[f64; 3]; 3],
    split_sides: u8,
    special: u8,
) -> Result<Vec<[[f64; 3]; 3]>, PaintError> {
    let special = special as usize;
    if special > 2 || (split_sides == 3 && special != 0) {
        return Err(PaintError::BadSplit);
    }
    let a = corners[special];
    let b = corners[(special + 1) % 3];
    let c = corners[(special + 2) % 3];
    Ok(match split_sides {
        1 => {
            let m = midpoint(b, c);
            vec![[a, b, m], [m, c, a]]
        }
        2 => {
            let p = midpoint(a, b);
            let q = midpoint(a, c);
            vec![[a, p, q], [p, b, q], [b, c, q]]
        }
        _ => {
            let p = midpoint(a, b);
            let r = midpoint(b, c);
            let q = midpoint(c, a);
            vec![[a, p, q], [p, b, r], [r, c, q], [p, r, q]]
        }
    })
}

/// Edge midpoint. The slicer averages in f32 in the part's own frame; an affine transform
/// keeps midpoints, so the f64 average in the object frame differs only by f32 rounding
/// (< 1e-5 mm on a 45 mm part).
fn midpoint(p: [f64; 3], q: [f64; 3]) -> [f64; 3] {
    [
        (p[0] + q[0]) * 0.5,
        (p[1] + q[1]) * 0.5,
        (p[2] + q[2]) * 0.5,
    ]
}

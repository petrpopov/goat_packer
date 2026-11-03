//! Lattice members of the portfolio (design 2026-11-02, section 9.2). Owned by work package
//! P4 (this file, `hnf`, `enumerate`, `residue`, `single`) and P5 (`double`, `two_block`).

pub mod double;
mod double_phase;
mod double_search;
pub mod enumerate;
pub mod hnf;
pub mod residue;
pub mod single;
mod span_ops;
pub mod two_block;
#[cfg(test)]
mod test_shapes;

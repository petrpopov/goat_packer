//! The deterministic packing portfolio (design 2026-11-02, section 9): lattices, sweeps, edge
//! fill and improvement over typed kinds. Owned by work package P3 (this file and the
//! skeleton), P4-P6 (members); the submodules are declared here so no one else edits this file.

pub mod context;
pub mod edge_fill;
pub mod free_map;
pub mod improve;
pub mod lattice;
pub mod no_fit;
pub mod robustness;
pub mod select;
pub mod sweep;
pub mod tower_scan;

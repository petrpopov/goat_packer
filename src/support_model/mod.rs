//! The organic tree-support model (design 2026-11-02, section 5): stage A runs once per
//! mesh and settings, stage B once per ensemble member, then the envelope.
//! Owned by work packages S2 (this file, stage A), S3 (stage B) and S4 (ensemble); the
//! submodules are declared here so no one else edits this file.

pub mod base;
pub mod bands;
pub mod cache;
pub mod clearance;
pub mod descend;
pub mod draw;
pub mod ensemble;
pub mod envelope;
pub mod flavour;
pub mod layers;
pub mod merge;
pub mod overhangs;
pub mod place;
pub mod sections;
pub mod settings;
pub mod smooth;
pub mod tips;

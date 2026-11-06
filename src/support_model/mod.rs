//! The organic tree-support model (design 2026-11-02, section 5): stage A runs once per
//! mesh and settings, stage B once per ensemble member, then the envelope.
//! Owned by work packages S2 (this file, stage A), S3 (stage B) and S4 (ensemble); the
//! submodules are declared here so no one else edits this file.
//!
//! Stage A (`stage_a`): `settings` (inputs and derived constants), `flavour` (slicer build
//! and its behaviour switches), `layers` (A1), `sections` (A2), paint projection (A3, in
//! `crate::paint`), `overhangs` (A4) and `clearance` (A5). Everything is computed in the
//! slicer's own object frame for one rotation class: the rotated mesh re-centred on its
//! normal parts' XY box (`ObjectMesh::rotated` keeps the unrotated centre; `StageA::frame_shift`
//! maps results back into the plate-aligned frame of design §4.1).

pub mod bands;
pub mod base;
pub mod cache;
pub mod clearance;
pub mod descend;
pub mod draw;
pub mod ensemble;
pub mod envelope;
pub mod exceedance;
pub mod flavour;
pub mod graph;
pub mod layers;
pub mod memo;
pub mod merge;
pub mod node;
pub mod overhangs;
pub mod place;
pub mod predict;
pub mod sections;
pub mod settings;
pub mod smooth;
pub mod tips;
pub mod union;

mod stage_a;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use stage_a::{StageA, StageAInput, stage_a};

/// Why the model cannot predict an object's supports.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportError {
    /// A setting or condition the behaviour specs do not describe (design §5.7): the
    /// pipeline falls back to the slicer or fails naming it. The model never guesses.
    Unsupported {
        setting: String,
        value: String,
        reason: String,
    },
    /// The input is unreadable or inconsistent.
    Invalid(String),
}

impl SupportError {
    pub fn unsupported(setting: &str, value: &str, reason: &str) -> SupportError {
        SupportError::Unsupported {
            setting: setting.to_owned(),
            value: value.to_owned(),
            reason: reason.to_owned(),
        }
    }

    /// The unsupported setting's name, if this is an `Unsupported` error.
    pub fn setting(&self) -> Option<&str> {
        match self {
            SupportError::Unsupported { setting, .. } => Some(setting),
            SupportError::Invalid(_) => None,
        }
    }
}

impl fmt::Display for SupportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SupportError::Unsupported {
                setting,
                value,
                reason,
            } => write!(
                formatter,
                "support model: unsupported {setting} = {value}: {reason}"
            ),
            SupportError::Invalid(message) => write!(formatter, "support model: {message}"),
        }
    }
}

impl std::error::Error for SupportError {}

impl From<String> for SupportError {
    fn from(message: String) -> SupportError {
        SupportError::Invalid(message)
    }
}

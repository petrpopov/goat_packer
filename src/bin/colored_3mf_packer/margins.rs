//! How far the measured support geometry is grown before the packer treats it as what a
//! later slice may print. One place, because these two numbers decide the plate counts and
//! are replaced from the calibration report, not by taste.
//!
//! The probes are a handful of slices of one object at the bed centre; the real plate prints
//! the same trees a little differently (tree supports vary between slices, and a full plate
//! is sliced on many cores). These start values are the lead's, 2026-11-03, until the
//! calibration run reports the exceedance rate:
//! - first layer: the first-layer support of a 44-goat P1S plate printed every base 0.5 mm
//!   wider than the learned patch (the 1.5 mm edge reserve of the old pipeline was three
//!   times that drift);
//! - bands: a sample of a few objects and the full plate differed by up to 0.5 mm per band
//!   (`SUPPORT_BODY_GAP_MM`), and the single-object probes miss the largest realizations of
//!   the family by more than the first layer does.

/// δ_L1: growth of the first-layer support (`ObjectPrint::support_base`), mm.
pub const DELTA_FIRST_LAYER_MM: f64 = 0.5;
/// δ_band: growth of the supports of every band, mm.
pub const DELTA_BAND_MM: f64 = 1.0;

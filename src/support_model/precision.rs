//! How exactly the model draws an ensemble (`--precision`). `Precise` is the calibrated
//! vector drawing; `Fast` unites each layer's pieces on a raster (`draw::raster`): a superset
//! of the vector layer by under 0.05 mm (band area +3 % on P1S), about a third faster per
//! job, and the plate count may lose a copy (2026-11-13, fresh cache, two threads, precise ->
//! fast: P1S 43 -> 43 copies in 78 -> 53 s, U1 50 -> 49 in 38 -> 28 s, A2L 73 -> 72 in 73 ->
//! 51 s; a 0.01 mm pixel gave A2L 71), so it is the user's choice, not the default.

/// The drawing of an ensemble's union.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Precision {
    /// Vector booleans, as calibrated.
    #[default]
    Precise,
    /// Raster union and notch closing.
    Fast,
}

impl Precision {
    pub fn name(self) -> &'static str {
        match self {
            Precision::Precise => "precise",
            Precision::Fast => "fast",
        }
    }

    pub fn parse(value: &str) -> Option<Precision> {
        match value {
            "precise" => Some(Precision::Precise),
            "fast" => Some(Precision::Fast),
            _ => None,
        }
    }
}

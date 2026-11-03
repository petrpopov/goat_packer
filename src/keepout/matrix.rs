//! The permission matrix (design §7.2) and the pair table (§7.3), each coded once as data.
//! The packing core derives its masks and checks from these tables; `verify_outputs` keeps its
//! own copy on purpose (it shares no packing code), and a test asserts the two agree.

use super::{BodyRule, KeepoutClass};
use crate::footprint::SUPPORT_BODY_GAP_MM;
use crate::object_print::PrintKind::{self, *};
use serde::{Deserialize, Serialize};

/// How far a support base keeps from a neighbour's landing (mm). Zero: they may touch, never
/// overlap. Evidence (owner, 2026-11-02): the plates that printed fine (U1 48 goats, A2L 70,
/// P1S 43) have realized support bases touching neighbours' first-layer bodies at -0.04..-0.09 mm
/// with the supports fused to each other and never to a goat, and the owner knows no minimum
/// gap ("what is in these files prints fine"). Supports above layer 0 keep
/// `SUPPORT_BODY_GAP_MM` from a neighbour's body: that one is the sample-versus-plate drift
/// of the band tests, which the first layer does not share (its patch is learned exactly).
pub const SUPPORT_BASE_GAP_MM: f64 = 0.0;

/// The packer-only gaps between one object's support and a NEIGHBOUR's model body (the 3MF
/// knows nothing of them; supports against supports and brims against anything stay free).
/// The defaults are the constants above; `--foreign-gaps` iterates over pairs of them, because
/// the printed plates the owner trusts sit much closer than the strict start values.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ForeignGaps {
    /// A support base (layer 0) to a neighbour's landing.
    pub base_mm: f64,
    /// A support above layer 0 to a neighbour's body in the same 2 mm band.
    pub band_mm: f64,
}

impl Default for ForeignGaps {
    fn default() -> Self {
        Self {
            base_mm: SUPPORT_BASE_GAP_MM,
            band_mm: SUPPORT_BODY_GAP_MM,
        }
    }
}

impl ForeignGaps {
    pub fn check(self) -> Result<(), String> {
        let bad = |v: f64| !v.is_finite() || v < 0.0;
        if bad(self.base_mm) || bad(self.band_mm) {
            return Err(format!("invalid foreign gaps {self:?}"));
        }
        Ok(())
    }
}

/// How far apart two things must stay. Symbolic, because the numbers belong to the job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Gap {
    /// May touch, never overlap.
    Zero,
    /// `PackOptions::clearance`: body to body.
    Clearance,
    /// `ForeignGaps::band_mm`: a support above layer 0 to a body. The tower body is a body
    /// too, so it follows the same value.
    Support,
    /// `ForeignGaps::base_mm`: a support base to a neighbour's landing.
    SupportBase,
}

impl Gap {
    pub fn mm(self, clearance_mm: f64, foreign: ForeignGaps) -> f64 {
        match self {
            Gap::Zero => 0.0,
            Gap::Clearance => clearance_mm,
            Gap::Support => foreign.band_mm,
            Gap::SupportBase => foreign.base_mm,
        }
    }
}

/// What one kind may do against one zone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permission {
    Allowed,
    /// The kind's material stays `gap` clear of the zone. Against the printable boundary the
    /// zone is everything outside it.
    Forbidden {
        gap: Gap,
    },
    /// Only each part's convex hull is tested, and touching passes: the slicer's rule for
    /// exclusion areas.
    PartHull,
    /// Only each part's bounding box is tested, and equality passes: printable bounds and the
    /// GUI's edge band.
    PartBox,
}

/// The matrix columns; the discriminant is the column index in `row`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ZoneClass {
    /// Outside the printable area (implicit; its edge band comes from `PlateRules`).
    Boundary,
    Hard,
    Restricted,
    Soft,
    TowerBody,
    /// Tower brim and chamfer rings.
    TowerBrim,
}

impl ZoneClass {
    pub const ALL: [ZoneClass; 6] = [
        ZoneClass::Boundary,
        ZoneClass::Hard,
        ZoneClass::Restricted,
        ZoneClass::Soft,
        ZoneClass::TowerBody,
        ZoneClass::TowerBrim,
    ];
}

impl From<KeepoutClass> for ZoneClass {
    fn from(class: KeepoutClass) -> Self {
        match class {
            KeepoutClass::Hard => ZoneClass::Hard,
            KeepoutClass::Restricted => ZoneClass::Restricted,
            KeepoutClass::Soft => ZoneClass::Soft,
        }
    }
}

/// `kind` against `zone`. A zone with a z range is only tested in the bands it meets
/// (`Keepout::meets_band`); first-layer kinds are tested at layer 0.
pub fn permission(kind: PrintKind, zone: ZoneClass) -> Permission {
    row(kind)[zone as usize]
}

const fn forbidden(gap: Gap) -> Permission {
    Permission::Forbidden { gap }
}

/// One matrix row per kind, columns in `ZoneClass::ALL` order. Where the slicer is looser than
/// the owner's rule, the rule follows the owner and the physics: the CLI accepts a copy's
/// support 2 mm off the bed, but copies are not clipped, so the GUI would refuse the plate.
#[rustfmt::skip]
const fn row(kind: PrintKind) -> [Permission; 6] {
    use Gap::{Clearance, Support, Zero};
    use Permission::{Allowed, PartBox, PartHull};
    match kind {
        //                 boundary           hard                restricted  soft                tower body              tower brim
        // Bodies: the tower brim only matters in the bands below its chamfer top (its z range).
        Silhouette | BodyBand
                       => [PartBox,           PartHull,           PartBox,    Allowed,            forbidden(Clearance),   forbidden(Clearance)],
        // The landing is body material (its thin-tip overshoot can leave the silhouette), so it
        // keeps the body clearance; boundary, hard and restricted are covered by the body rows.
        Landing        => [Allowed,           Allowed,            Allowed,    forbidden(Zero),    forbidden(Clearance),   forbidden(Clearance)],
        // The slicer clips and trims brims.
        Brim           => [Allowed,           Allowed,            Allowed,    Allowed,            Allowed,                Allowed],
        // Supports must stay inside the printable area inset by `print_edge_reserve_mm`; the
        // tower brim may grow over them (owner; the GUI warns, the CLI conflicts without --no-check).
        SupportBase | SupportBand | InterfaceBand
                       => [forbidden(Zero),   forbidden(Zero),    Allowed,    Allowed,            forbidden(Support),     Allowed],
        // Tower body at the soft zones means its layer 0.
        TowerBody      => [forbidden(Zero),   forbidden(Zero),    Allowed,    forbidden(Zero),    Allowed,                Allowed],
        // The GUI counts tower lines against the printable area.
        TowerBrim      => [forbidden(Zero),   forbidden(Zero),    Allowed,    Allowed,            Allowed,                Allowed],
    }
}

/// Two kinds of different objects that must keep `gap` apart. Applied both ways, and within
/// the same band for band kinds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairRule {
    pub a: PrintKind,
    pub b: PrintKind,
    pub gap: Gap,
}

const fn pair(a: PrintKind, b: PrintKind, gap: Gap) -> PairRule {
    PairRule { a, b, gap }
}

/// The pairs with silhouettes as the body rule.
const SILHOUETTE_PAIRS: [PairRule; 4] = [
    pair(Silhouette, Silhouette, Gap::Clearance),
    pair(SupportBase, Landing, Gap::SupportBase),
    pair(SupportBand, BodyBand, Gap::Support),
    pair(InterfaceBand, BodyBand, Gap::Support),
];

/// The same with per-band bodies: physically valid for FDM, looser than the silhouette.
const BAND_PAIRS: [PairRule; 4] = [
    pair(BodyBand, BodyBand, Gap::Clearance),
    pair(SupportBase, Landing, Gap::SupportBase),
    pair(SupportBand, BodyBand, Gap::Support),
    pair(InterfaceBand, BodyBand, Gap::Support),
];

/// Every forbidden pair of kinds of two different objects. Everything else is allowed:
/// supports overlap supports, and brims overlap anything (the slicer trims them).
pub fn pair_rules(body_rule: BodyRule) -> &'static [PairRule] {
    match body_rule {
        BodyRule::Silhouette => &SILHOUETTE_PAIRS,
        BodyRule::Bands => &BAND_PAIRS,
    }
}

/// The gap `a` and `b` of different objects must keep, or `None` when the pair is allowed.
pub fn pair_gap(a: PrintKind, b: PrintKind, body_rule: BodyRule) -> Option<Gap> {
    pair_rules(body_rule)
        .iter()
        .find(|rule| (rule.a, rule.b) == (a, b) || (rule.a, rule.b) == (b, a))
        .map(|rule| rule.gap)
}

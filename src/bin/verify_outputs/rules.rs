//! verify_outputs' own copy of the permission matrix and the pair table (design 2026-11-02,
//! §7.2 and §7.3). The duplication is deliberate: this binary shares no packing code, so a
//! wrong row in the packer cannot also hide here. A test pins both tables cell by cell against
//! the library's (`goat_packer::keepout`), so a change to one must be made to both.
//! The checks read the rows of the outline they have (`Kind::Silhouette`); the other rows
//! are kept whole so the copy stays a copy.
#![cfg_attr(not(test), allow(dead_code))]

/// What an object prints, as the matrix rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Silhouette,
    BodyBand,
    Landing,
    Brim,
    SupportBase,
    SupportBand,
    InterfaceBand,
    TowerBody,
    TowerBrim,
}

/// The matrix columns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Zone {
    /// Outside the printable area.
    Boundary,
    Hard,
    Restricted,
    Soft,
    TowerBody,
    TowerBrim,
}

/// How far apart two things must stay, symbolically.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Gap {
    /// May touch, never overlap.
    Zero,
    /// `--minimum-gap`: body to body.
    Clearance,
    /// A support above layer 0 to a foreign body.
    Support,
    /// A support base to a foreign landing.
    SupportBase,
}

/// A support keeps this far from a foreign body (mm): the packer's `SUPPORT_BODY_GAP_MM`.
pub const SUPPORT_GAP_MM: f64 = 0.5;

/// A support base keeps this far from a foreign landing (mm): the packer's
/// `SUPPORT_BASE_GAP_MM` (touching allowed; evidence at its definition).
pub const SUPPORT_BASE_GAP_MM: f64 = 0.0;

/// The packer's `ForeignGaps` (`--foreign-base-gap`, `--foreign-band-gap`), own copy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Foreign {
    pub base_mm: f64,
    pub band_mm: f64,
}

impl Default for Foreign {
    fn default() -> Self {
        Self {
            base_mm: SUPPORT_BASE_GAP_MM,
            band_mm: SUPPORT_GAP_MM,
        }
    }
}

impl Gap {
    pub fn mm(self, clearance_mm: f64, foreign: Foreign) -> f64 {
        match self {
            Gap::Zero => 0.0,
            Gap::Clearance => clearance_mm,
            Gap::Support => foreign.band_mm,
            Gap::SupportBase => foreign.base_mm,
        }
    }
}

/// What one kind may do against one zone class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rule {
    Allowed,
    /// The material keeps the gap from the zone.
    Forbidden(Gap),
    /// Each part's convex hull must not overlap the zone; touching passes (the slicer's
    /// exclusion rule).
    PartHull,
    /// Each part's bounding box stays out (or, at the boundary, inside); equality passes.
    PartBox,
}

/// `kind` against `zone`.
#[rustfmt::skip]
pub fn rule(kind: Kind, zone: Zone) -> Rule {
    use Gap::{Clearance, Support, Zero};
    use Rule::{Allowed, Forbidden, PartBox, PartHull};
    let row = match kind {
        //                                 boundary         hard             restricted  soft             tower body           tower brim
        Kind::Silhouette | Kind::BodyBand => [PartBox,         PartHull,        PartBox,    Allowed,         Forbidden(Clearance), Forbidden(Clearance)],
        Kind::Landing                     => [Allowed,         Allowed,         Allowed,    Forbidden(Zero), Forbidden(Clearance), Forbidden(Clearance)],
        Kind::Brim                        => [Allowed,         Allowed,         Allowed,    Allowed,         Allowed,              Allowed],
        Kind::SupportBase | Kind::SupportBand | Kind::InterfaceBand
                                          => [Forbidden(Zero), Forbidden(Zero), Allowed,    Allowed,         Forbidden(Support),   Allowed],
        Kind::TowerBody                   => [Forbidden(Zero), Forbidden(Zero), Allowed,    Forbidden(Zero), Allowed,              Allowed],
        Kind::TowerBrim                   => [Forbidden(Zero), Forbidden(Zero), Allowed,    Allowed,         Allowed,              Allowed],
    };
    let column = match zone {
        Zone::Boundary => 0,
        Zone::Hard => 1,
        Zone::Restricted => 2,
        Zone::Soft => 3,
        Zone::TowerBody => 4,
        Zone::TowerBrim => 5,
    };
    row[column]
}

/// The gap two different objects' kinds keep, or `None` when the pair is allowed (supports
/// overlap supports, brims overlap anything). `bands` = bodies compared band by band instead
/// of as whole silhouettes.
pub fn pair_gap(a: Kind, b: Kind, bands: bool) -> Option<Gap> {
    let body = if bands {
        Kind::BodyBand
    } else {
        Kind::Silhouette
    };
    let pairs = [
        (body, body, Gap::Clearance),
        (Kind::SupportBase, Kind::Landing, Gap::SupportBase),
        (Kind::SupportBand, Kind::BodyBand, Gap::Support),
        (Kind::InterfaceBand, Kind::BodyBand, Gap::Support),
    ];
    pairs
        .iter()
        .find(|&&(x, y, _)| (x, y) == (a, b) || (x, y) == (b, a))
        .map(|&(.., gap)| gap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use goat_packer::PrintKind;
    use goat_packer::keepout::{self, BodyRule, Permission, ZoneClass};

    const KINDS: [(Kind, PrintKind); 9] = [
        (Kind::Silhouette, PrintKind::Silhouette),
        (Kind::BodyBand, PrintKind::BodyBand),
        (Kind::Landing, PrintKind::Landing),
        (Kind::Brim, PrintKind::Brim),
        (Kind::SupportBase, PrintKind::SupportBase),
        (Kind::SupportBand, PrintKind::SupportBand),
        (Kind::InterfaceBand, PrintKind::InterfaceBand),
        (Kind::TowerBody, PrintKind::TowerBody),
        (Kind::TowerBrim, PrintKind::TowerBrim),
    ];

    const ZONES: [(Zone, ZoneClass); 6] = [
        (Zone::Boundary, ZoneClass::Boundary),
        (Zone::Hard, ZoneClass::Hard),
        (Zone::Restricted, ZoneClass::Restricted),
        (Zone::Soft, ZoneClass::Soft),
        (Zone::TowerBody, ZoneClass::TowerBody),
        (Zone::TowerBrim, ZoneClass::TowerBrim),
    ];

    fn gap_of(gap: keepout::Gap) -> Gap {
        match gap {
            keepout::Gap::Zero => Gap::Zero,
            keepout::Gap::Clearance => Gap::Clearance,
            keepout::Gap::Support => Gap::Support,
            keepout::Gap::SupportBase => Gap::SupportBase,
        }
    }

    fn rule_of(permission: Permission) -> Rule {
        match permission {
            Permission::Allowed => Rule::Allowed,
            Permission::Forbidden { gap } => Rule::Forbidden(gap_of(gap)),
            Permission::PartHull => Rule::PartHull,
            Permission::PartBox => Rule::PartBox,
        }
    }

    #[test]
    fn the_matrix_copy_agrees_with_the_packer_cell_by_cell() {
        assert_eq!(KINDS.len(), PrintKind::ALL.len());
        assert_eq!(ZONES.len(), ZoneClass::ALL.len());
        for (kind, print_kind) in KINDS {
            for (zone, zone_class) in ZONES {
                assert_eq!(
                    rule(kind, zone),
                    rule_of(keepout::permission(print_kind, zone_class)),
                    "{kind:?} against {zone:?}"
                );
            }
        }
    }

    #[test]
    fn the_pair_table_copy_agrees_with_the_packer_both_body_rules() {
        assert_eq!(SUPPORT_GAP_MM, goat_packer::SUPPORT_BODY_GAP_MM);
        assert_eq!(SUPPORT_BASE_GAP_MM, keepout::SUPPORT_BASE_GAP_MM);
        let packer = keepout::ForeignGaps::default();
        assert_eq!((Foreign::default().base_mm, Foreign::default().band_mm), (packer.base_mm, packer.band_mm));
        // The same non-default values mean the same millimetres in both tables.
        let ours = Foreign { base_mm: 0.13, band_mm: 0.37 };
        let theirs = keepout::ForeignGaps { base_mm: 0.13, band_mm: 0.37 };
        for gap in [Gap::Zero, Gap::Clearance, Gap::Support, Gap::SupportBase] {
            let packer_gap = match gap {
                Gap::Zero => keepout::Gap::Zero,
                Gap::Clearance => keepout::Gap::Clearance,
                Gap::Support => keepout::Gap::Support,
                Gap::SupportBase => keepout::Gap::SupportBase,
            };
            assert_eq!(gap.mm(0.7, ours), packer_gap.mm(0.7, theirs), "{gap:?}");
            assert_eq!(gap_of(packer_gap), gap);
        }
        for (bands, body_rule) in [(false, BodyRule::Silhouette), (true, BodyRule::Bands)] {
            for (a, print_a) in KINDS {
                for (b, print_b) in KINDS {
                    assert_eq!(
                        pair_gap(a, b, bands),
                        keepout::pair_gap(print_a, print_b, body_rule).map(gap_of),
                        "{a:?} against {b:?}, {body_rule:?}"
                    );
                }
            }
        }
    }
}

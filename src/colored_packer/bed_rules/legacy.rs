//! Profiles without `bed_rules`: their old fields through the legacy table (design §7.5). And
//! the other direction for profiles with `bed_rules`: the old fields are still read (the legacy
//! pipeline, the webapp, `source_project --edge-margin`), so they must say the same thing.

use super::schema::{ProfileBedRules, ProfileKeepout};
use crate::colored_packer::PrinterProfile;
use crate::colored_packer::profile::SLICER_KEEPOUT_SAFETY_MM;
use crate::keepout::{KeepoutClass, KeepoutFrame};

/// Growth of every legacy `exclusion_rectangles` entry, mm: the same empirical blanket the
/// legacy pipeline puts around bed exclusions (`SLICER_KEEPOUT_SAFETY_MM`). Dense real P1S
/// plates hit false "gcode path conflicts" at the exclusion corner below it, while isolated
/// single-object tests passed at 0.7-0.8 mm; do not lower it from single-object evidence.
/// The exact hull rule plus "copies' supports are not clipped" (design §7.2) explain those
/// conflicts, so explicit `bed_rules` zones are exact; this stays for old profiles until the
/// offline full-plate audit near the P1S cutter passes.
pub const LEGACY_EXCLUSION_GROWTH_MM: f64 = SLICER_KEEPOUT_SAFETY_MM;

/// Values closer than this are the same number written twice.
const SAME_MM: f64 = 1e-9;

/// The legacy table: `edge_margin_mm` alone is a hard band (it binds everything, as a macro
/// purge may run anywhere near the edge); with `support_edge_margin_mm` it binds only the
/// bodies and the support margin becomes the print reserve; exclusion rectangles are hard and
/// grown by [`LEGACY_EXCLUSION_GROWTH_MM`]; purge lines are soft strips in the G-code frame up
/// to the project's first layer.
pub fn legacy_bed_rules(
    profile: &PrinterProfile,
    first_layer_height_mm: Option<f64>,
) -> ProfileBedRules {
    let mut keepouts = Vec::new();
    let (body_edge_band_mm, print_edge_reserve_mm) = match profile.support_edge_margin_mm {
        Some(reserve) => (profile.edge_margin_mm, reserve),
        None => {
            if profile.edge_margin_mm > 0.0 {
                keepouts.push(
                    ProfileKeepout::new("edge_margin", KeepoutClass::Hard)
                        .with_edge_band(profile.edge_margin_mm)
                        .from_source("edge_margin_mm"),
                );
            }
            (0.0, 0.0)
        }
    };
    for (index, [x0, y0, x1, y1]) in profile.exclusion_rectangles.iter().enumerate() {
        let growth = LEGACY_EXCLUSION_GROWTH_MM;
        keepouts.push(
            ProfileKeepout::new(&format!("exclusion_{index}"), KeepoutClass::Hard)
                .with_rect([x0 - growth, y0 - growth, x1 + growth, y1 + growth])
                .from_source("exclusion_rectangles"),
        );
    }
    if !profile.purge_lines.is_empty() {
        let purge = ProfileKeepout::new("purge", KeepoutClass::Soft)
            .with_strips(profile.purge_lines.clone())
            .in_gcode_frame()
            .from_source("purge_lines");
        keepouts.push(match first_layer_height_mm {
            Some(z_max) => purge.up_to(z_max),
            None => purge,
        });
    }
    ProfileBedRules {
        body_edge_band_mm,
        print_edge_reserve_mm,
        keepouts,
    }
}

/// Where a profile's old fields disagree with its `bed_rules` (empty without `bed_rules`):
/// `edge_margin_mm` must be the distance bodies keep from the edge, `exclusion_rectangles` the
/// hard plate-frame rects, `purge_lines` the soft G-code-frame strips, and a
/// `support_edge_margin_mm` may be stricter than the print reserve, never looser. The legacy
/// slicer-learned path keeps a stricter support margin where the new rules rely on the support
/// envelope's calibrated margin instead.
pub fn old_field_mismatches(profile: &PrinterProfile) -> Vec<String> {
    let Some(rules) = &profile.bed_rules else {
        return Vec::new();
    };
    let mut mismatches = Vec::new();
    let margin = rules.body_margin_mm(profile.gui_flavour.as_ref());
    if (profile.edge_margin_mm - margin).abs() > SAME_MM {
        mismatches.push(format!(
            "edge_margin_mm is {} but the bed rules keep bodies {margin} mm from the edge",
            profile.edge_margin_mm
        ));
    }
    let hard_rects: Vec<[f64; 4]> = zones(rules, KeepoutClass::Hard, KeepoutFrame::Plate)
        .filter_map(|keepout| keepout.rect)
        .collect();
    if !same_numbers(&profile.exclusion_rectangles, &hard_rects) {
        mismatches.push("exclusion_rectangles are not the hard plate-frame rects".to_owned());
    }
    let soft_strips: Vec<[f64; 5]> = zones(rules, KeepoutClass::Soft, KeepoutFrame::Gcode)
        .flat_map(|keepout| keepout.strips.iter().flatten().copied())
        .collect();
    if !same_numbers(&profile.purge_lines, &soft_strips) {
        mismatches.push("purge_lines are not the soft G-code-frame strips".to_owned());
    }
    if let Some(margin) = profile.support_edge_margin_mm
        && margin < rules.print_edge_reserve_mm - SAME_MM
    {
        mismatches.push(format!(
            "support_edge_margin_mm {margin} is looser than print_edge_reserve_mm {}",
            rules.print_edge_reserve_mm
        ));
    }
    mismatches
}

fn zones(
    rules: &ProfileBedRules,
    class: KeepoutClass,
    frame: KeepoutFrame,
) -> impl Iterator<Item = &ProfileKeepout> {
    rules
        .keepouts
        .iter()
        .filter(move |keepout| keepout.class == class && keepout.frame == frame)
}

fn same_numbers<const N: usize>(a: &[[f64; N]], b: &[[f64; N]]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(left, right)| {
            left.iter()
                .zip(right)
                .all(|(l, r)| (l - r).abs() <= SAME_MM)
        })
}

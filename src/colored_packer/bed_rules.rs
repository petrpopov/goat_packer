//! A printer profile's bed rules (design 2026-11-02, §7.4-§7.7) and the `PlateRules` of one job.
//!
//! - `schema`: the profile's optional `bed_rules` block (`ProfileBedRules`, `ProfileKeepout`).
//! - `flavour`: the profile's `gui_flavour`, the slicer GUI the plate is sliced in.
//! - `legacy`: profiles without `bed_rules` mapped from their old fields (§7.5), and the check
//!   that a profile's old fields still say what its `bed_rules` say.
//! - `zone_check`: the §7.2 matrix for the profile zones on one placed object, exact geometry.
//!
//! A profile declares geometry and a class only; what a class forbids lives in
//! `keepout::matrix`. The rules of a job are always in the plate frame: G-code-frame keepouts
//! (start G-code purges) move by the project's `extruder_offset` here, once.

mod flavour;
mod legacy;
mod schema;
mod zone_check;

use std::borrow::Cow;

use serde_json::Value;

use super::PrinterProfile;
use crate::gcode_frame::extruder_offset;
use crate::keepout::{BodyRule, PlateRules};

pub use flavour::{BAMBU_SUPPORT_CLIP_MM, GuiFamily, GuiFlavour};
pub use legacy::{LEGACY_EXCLUSION_GROWTH_MM, legacy_bed_rules, old_field_mismatches};
pub use schema::{ProfileBedRules, ProfileKeepout};
pub use zone_check::{ZoneHit, zone_hits};

/// What the source project adds to a profile's rules.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProjectBed {
    /// The project's `extruder_offset`: plate = G-code + this (P1S (0, 2)), because BambuStudio
    /// writes G-code minus it (`gcode_frame`).
    pub gcode_offset: (f64, f64),
    /// `initial_layer_print_height`: the height legacy purge lines are capped at. `None` = the
    /// project does not say, and a legacy purge zone holds at every height.
    pub first_layer_height_mm: Option<f64>,
}

impl ProjectBed {
    /// From the project's `project_settings.config`.
    pub fn from_settings(settings: &Value) -> Result<Self, String> {
        Ok(Self {
            gcode_offset: extruder_offset(settings)?,
            first_layer_height_mm: setting_number(settings, "initial_layer_print_height"),
        })
    }
}

/// A slicer setting as a number: projects store `"0.2"` or `["0.2"]`.
fn setting_number(settings: &Value, key: &str) -> Option<f64> {
    let text = match settings.get(key)? {
        Value::Array(items) => items.first()?.as_str()?,
        other => other.as_str()?,
    };
    text.trim()
        .parse()
        .ok()
        .filter(|number: &f64| number.is_finite())
}

/// The profile's declared rules: its `bed_rules`, or its old fields through the legacy table.
pub fn declared_bed_rules<'a>(
    profile: &'a PrinterProfile,
    project: &ProjectBed,
) -> Cow<'a, ProfileBedRules> {
    match &profile.bed_rules {
        Some(rules) => Cow::Borrowed(rules),
        None => Cow::Owned(legacy_bed_rules(profile, project.first_layer_height_mm)),
    }
}

/// The plate rules of a job, plate frame. The body band is the wider of the profile's and the
/// GUI's own (`profile.gui_flavour`; a caller that resolves the target GUI from the project
/// sets that field first). The body rule stays the default silhouette.
pub fn plate_rules(profile: &PrinterProfile, project: &ProjectBed) -> Result<PlateRules, String> {
    let declared = declared_bed_rules(profile, project);
    declared.validate()?;
    let gui_band = profile
        .gui_flavour
        .as_ref()
        .map_or(0.0, GuiFlavour::body_edge_band_mm);
    let clip_cap = profile
        .gui_flavour
        .as_ref()
        .map_or(0.0, GuiFlavour::support_clip_cap_mm);
    Ok(PlateRules {
        printable_area: profile.printable_bed(),
        body_edge_band_mm: declared.body_edge_band_mm.max(gui_band),
        print_edge_reserve_mm: declared.print_edge_reserve_mm,
        support_clip_mm: declared.max_support_clip_mm.map_or(0.0, |own| own.min(clip_cap)),
        support_clip_sides: [true; 4],
        keepouts: declared
            .keepouts
            .iter()
            .map(|keepout| keepout.to_plate(project.gcode_offset))
            .collect::<Result<_, _>>()?,
        body_rule: BodyRule::default(),
    })
}

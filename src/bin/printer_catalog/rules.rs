//! The bed rules and GUI flavour of a catalog profile (design 2026-11-02, §7.6), derived
//! without slicing from the profile's own fields and what its machine preset says. `annotate`
//! writes them into existing profiles, `qualify` into new ones; both also set `edge_margin_mm`
//! to what the rules keep bodies from the edge, the number the webapp and `source_project
//! --edge-margin` read.

use goat_packer::KeepoutClass;
use goat_packer::colored_packer::PrinterProfile;
use goat_packer::colored_packer::bed_rules::{
    GuiFamily, GuiFlavour, ProfileBedRules, ProfileKeepout,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::annotate::{MACRO_PURGE_MARGIN_MM, hidden_macro_purge, start_gcode, start_gcode_macros};

/// BambuStudio's front strip on Bambu printers that scan the first layer (X1 family): its
/// arranger keeps models off this extrusion and hand-eye calibration region but lets brims
/// grow over it. As a soft zone only first-layer bodies keep off it; supports and brims may
/// cover it. Orca applies the region to Bambu printers only, so another vendor's
/// `scan_first_layer` (Elegoo Centauri) does not bring it.
const FIRST_LAYER_SCAN_STRIP: [f64; 4] = [18.0, 0.0, 240.0, 15.0];
const BAMBU_VENDOR: &str = "BBL";

/// The GUI versions the support model and the GUI checks follow: the BambuStudio release the
/// owner slices in, the Orca release the catalog was imported from (v2.4.2), and Snapmaker's
/// Orca fork of the same line.
const BAMBU_GUI_VERSION: &str = "02.08";
const ORCA_GUI_VERSION: &str = "2.4";
const SNAPMAKER_ORCA_GUI_VERSION: &str = "2.4";

/// What the machine preset (and, in `qualify`, the probe slice) adds to a profile's fields.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PrinterFacts {
    /// Firmware macros the start G-code calls (`annotate::start_gcode_macros`).
    pub macros: Vec<String>,
    /// The printer scans its first layer ([`FIRST_LAYER_SCAN_STRIP`]).
    pub scan_first_layer: bool,
    /// The height the probe slice printed the purge at; `annotate` does not slice and leaves
    /// the purge zone at every height (only first-layer kinds meet a soft zone anyway).
    pub purge_z_mm: Option<f64>,
}

impl PrinterFacts {
    pub fn of_machine(machine: &Map<String, Value>) -> Self {
        let scan = machine.get("scan_first_layer").and_then(first_text);
        Self {
            macros: start_gcode_macros(&start_gcode(machine)),
            scan_first_layer: scan.is_some_and(|text| text.trim() == "1"),
            purge_z_mm: None,
        }
    }
}

/// Orca stores a flag as `"1"` or `["1"]`.
fn first_text(value: &Value) -> Option<&str> {
    match value {
        Value::Array(items) => items.first()?.as_str(),
        other => other.as_str(),
    }
}

/// The derived part of a catalog profile.
#[derive(Clone, Debug, PartialEq)]
pub struct DerivedRules {
    pub bed_rules: ProfileBedRules,
    pub gui_flavour: GuiFlavour,
    /// How far bodies keep from the printable edge under the rules.
    pub edge_margin_mm: f64,
}

/// The GUI a catalog printer's plates are opened in: Bambu printers in BambuStudio, Snapmaker
/// printers in Snapmaker Orca, everything else in Orca.
pub fn gui_flavour_for_vendor(vendor: &str) -> GuiFlavour {
    match vendor {
        BAMBU_VENDOR => GuiFlavour::new(GuiFamily::Bambu, BAMBU_GUI_VERSION),
        "Snapmaker" => GuiFlavour::new(GuiFamily::SnapmakerOrca, SNAPMAKER_ORCA_GUI_VERSION),
        _ => GuiFlavour::new(GuiFamily::Orca, ORCA_GUI_VERSION),
    }
}

/// Bed exclusions are hard and exact (the slicer tests part hulls against them and never clips
/// copies' supports); detected purges are soft strips in the G-code frame; a purge hidden in a
/// firmware macro is a hard edge band, because where the macro draws its line is unknown; the
/// GUI's own body band (Snapmaker Orca) is the restricted band.
pub fn derive(profile: &PrinterProfile, vendor: &str, facts: &PrinterFacts) -> DerivedRules {
    let gui_flavour = gui_flavour_for_vendor(vendor);
    let mut keepouts: Vec<ProfileKeepout> = profile
        .exclusion_rectangles
        .iter()
        .enumerate()
        .map(|(index, rect)| {
            ProfileKeepout::new(&format!("bed_exclude_{index}"), KeepoutClass::Hard)
                .with_rect(*rect)
                .from_source("bed_exclude_area")
        })
        .collect();
    if !profile.purge_lines.is_empty() {
        let purge = ProfileKeepout::new("purge", KeepoutClass::Soft)
            .with_strips(profile.purge_lines.clone())
            .in_gcode_frame()
            .from_source("machine_start_gcode");
        keepouts.push(match facts.purge_z_mm {
            Some(z_max) => purge.up_to(z_max),
            None => purge,
        });
    }
    if facts.scan_first_layer && vendor == BAMBU_VENDOR {
        keepouts.push(
            ProfileKeepout::new("first_layer_scan", KeepoutClass::Soft)
                .with_rect(FIRST_LAYER_SCAN_STRIP)
                .from_source("scan_first_layer"),
        );
    }
    if hidden_macro_purge(&profile.purge_lines, &facts.macros) {
        keepouts.push(
            ProfileKeepout::new("macro_purge", KeepoutClass::Hard)
                .with_edge_band(MACRO_PURGE_MARGIN_MM)
                .from_source("start_gcode_macro"),
        );
    }
    let bed_rules = ProfileBedRules {
        body_edge_band_mm: gui_flavour.body_edge_band_mm(),
        print_edge_reserve_mm: 0.0,
        keepouts,
    };
    let edge_margin_mm = bed_rules.body_margin_mm(Some(&gui_flavour));
    DerivedRules {
        bed_rules,
        gui_flavour,
        edge_margin_mm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use goat_packer::colored_packer::bed_rules::old_field_mismatches;
    use serde_json::json;

    fn profile(exclusions: Vec<[f64; 4]>, purge: Vec<[f64; 5]>) -> PrinterProfile {
        serde_json::from_value(json!({
            "id": "acme_box", "printer_name": "Acme Box", "slicer_kind": "orca",
            "slicer_path": "/opt/slicers/orca/AppRun", "blank_start_gcode_for_cli": false,
            "no_check_for_final_tower_probe": false, "printable_area": [0.0, 0.0, 220.0, 220.0],
            "edge_margin_mm": 0.0, "exclusion_rectangles": exclusions, "purge_lines": purge,
            "tower_marker_style": "wipe_tower", "tower_line_width_mm": 0.5
        }))
        .unwrap()
    }

    fn written(mut profile: PrinterProfile, derived: &DerivedRules) -> PrinterProfile {
        profile.bed_rules = Some(derived.bed_rules.clone());
        profile.gui_flavour = Some(derived.gui_flavour.clone());
        profile.edge_margin_mm = derived.edge_margin_mm;
        profile
    }

    #[test]
    fn exclusions_are_exact_hard_rects_and_purges_soft_gcode_strips() {
        let source = profile(
            vec![[0.0, 0.0, 18.0, 28.0]],
            vec![[18.0, 1.0, 240.0, 1.0, 0.81]],
        );
        let facts = PrinterFacts {
            purge_z_mm: Some(0.2),
            ..PrinterFacts::default()
        };
        let derived = derive(&source, "Acme", &facts);
        let rules = &derived.bed_rules;
        assert_eq!(rules.keepouts.len(), 2);
        assert_eq!(rules.keepouts[0].class, KeepoutClass::Hard);
        assert_eq!(rules.keepouts[0].rect, Some([0.0, 0.0, 18.0, 28.0]));
        assert_eq!(rules.keepouts[1].class, KeepoutClass::Soft);
        assert_eq!(rules.keepouts[1].z_range_mm, Some([0.0, 0.2]));
        assert_eq!(derived.gui_flavour.to_string(), "orca-2.4");
        assert_eq!(derived.edge_margin_mm, 0.0);
        rules.validate().unwrap();
        assert!(old_field_mismatches(&written(source, &derived)).is_empty());
    }

    #[test]
    fn a_hidden_macro_purge_is_a_hard_five_millimetre_band() {
        let macros = vec!["PRINT_START".to_owned()];
        let facts = PrinterFacts {
            macros: macros.clone(),
            ..PrinterFacts::default()
        };
        let derived = derive(&profile(vec![], vec![]), "Acme", &facts);
        let band = derived.bed_rules.keepouts.last().unwrap();
        assert_eq!(
            (band.class, band.edge_band_mm),
            (KeepoutClass::Hard, Some(MACRO_PURGE_MARGIN_MM))
        );
        assert_eq!(derived.edge_margin_mm, MACRO_PURGE_MARGIN_MM);
        // A purge the probe saw is not hidden: no band.
        let seen = derive(
            &profile(vec![], vec![[1.0, 2.0, 3.0, 2.0, 1.0]]),
            "Acme",
            &facts,
        );
        assert!(
            seen.bed_rules
                .keepouts
                .iter()
                .all(|k| k.edge_band_mm.is_none())
        );
        assert_eq!(seen.edge_margin_mm, 0.0);
    }

    #[test]
    fn vendor_picks_the_gui_and_snapmaker_orca_brings_its_body_band() {
        let snapmaker = derive(
            &profile(vec![], vec![]),
            "Snapmaker",
            &PrinterFacts::default(),
        );
        assert_eq!(snapmaker.gui_flavour.to_string(), "snapmaker-orca-2.4");
        assert_eq!(snapmaker.bed_rules.body_edge_band_mm, 3.5);
        assert_eq!(snapmaker.edge_margin_mm, 3.5);
        let bambu = derive(&profile(vec![], vec![]), "BBL", &PrinterFacts::default());
        assert_eq!(bambu.gui_flavour.to_string(), "bambu-02.08");
        assert_eq!(bambu.bed_rules.body_edge_band_mm, 0.0);
        assert!(old_field_mismatches(&written(profile(vec![], vec![]), &snapmaker)).is_empty());
    }

    #[test]
    fn first_layer_scan_printers_get_bambus_soft_calibration_strip() {
        let machine = json!({"scan_first_layer": "1", "machine_start_gcode": "G28\n"});
        let facts = PrinterFacts::of_machine(machine.as_object().unwrap());
        assert!(facts.scan_first_layer && facts.macros.is_empty());
        let derived = derive(&profile(vec![], vec![]), "BBL", &facts);
        let strip = &derived.bed_rules.keepouts[0];
        assert_eq!(
            (strip.class, strip.rect),
            (KeepoutClass::Soft, Some(FIRST_LAYER_SCAN_STRIP))
        );
        let other = derive(&profile(vec![], vec![]), "Elegoo", &facts);
        assert!(other.bed_rules.keepouts.is_empty());
        let plain = json!({"scan_first_layer": ["0"]});
        assert!(!PrinterFacts::of_machine(plain.as_object().unwrap()).scan_first_layer);
    }
}

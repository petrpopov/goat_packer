//! STL → slicer project (`input.3mf`) on the runner, before `colored_3mf_packer` (plan
//! 2026-11-01-stl-upload-and-printer-picker, Task 3):
//!
//!   source_project --stl model.stl [--presets /opt/slicer/resources/profiles]
//!     [--orca /opt/slicers/orca/AppRun]
//!     (--vendor BBL --machine "Bambu Lab P1S 0.4 nozzle" | --custom-bed W,D,H)
//!     [--edge-margin 5] [--gui-flavour snapmaker-orca-2.4] --output input.3mf --work-dir work
//!
//! Presets are read at run time from the slicer installed in the image (plain JSON) and
//! flattened by `orca_project`; nothing from Orca is shipped by us. A catalog printer loads its
//! profile's `vendor` (Orca folder) + `orca_machine`; a custom bed puts W×D×H on Orca's generic
//! machine (`printer_model` `Custom Printer`). `--orca` is whichever slicer CLI the job image
//! carries: in runner-bambu `/opt/slicers/orca/AppRun` is Bambu Studio, and its own BBL tree
//! exports the same way (verified 2026-11-01 on P1S/A2L: export, pack, final slice OK; the
//! filament falls back to the vendor's `Generic PLA`, see `orca_project::catalog_presets`). The
//! A2L has a machine preset only in Bambu Studio's tree. The model must fit the printable area
//! minus `--edge-margin` (the profile's `edge_margin_mm`, the packer keeps the same margin) and
//! `printable_height`, checked before the export. Every failure exits 1 with one line
//! `error: STL could not be converted: …` (the webapp's CliErrorTranslator keys on it).
//!
//! `--gui-flavour` is the GUI the user picked to print with (webapp: the job profile's
//! `gui_flavour` after the slicer choice). OrcaSlicer writes `BambuStudio-<compat>` into
//! `Application` plus its own `OrcaSlicer` tag; read alone that passed for BambuStudio, so a U1
//! STL project was packed against BambuStudio's GUI rules. The export is re-stamped for the
//! target (Snapmaker Orca: `Snapmaker_Orca-2.4.0`; BambuStudio: its own string), so the GUI
//! loads the project's settings as its own and the packer resolves the same target.
use std::{
    fs,
    path::{Path, PathBuf},
};

use goat_packer::{
    orca_presets::PresetIndex,
    orca_project::{self, CliPresets, Error, failure_line},
    process_output::bounded_output,
    project_application::{project_application, stamp_project},
    support_model::flavour::Flavour,
};

const DEFAULT_PRESETS: &str = "/opt/slicer/resources/profiles";
const DEFAULT_ORCA: &str = "/opt/slicers/orca/AppRun";

#[derive(Debug, PartialEq)]
enum Printer {
    Catalog { vendor: String, machine: String },
    Custom { width: f64, depth: f64, height: f64 },
}

#[derive(Debug)]
struct Args {
    stl: PathBuf,
    presets: PathBuf,
    orca: PathBuf,
    printer: Printer,
    edge_margin: f64,
    /// `--gui-flavour`: the slicer GUI the user opens the project in (`bambu-02.09`,
    /// `orca-2.4`, `snapmaker-orca-2.4`, the job profile's `gui_flavour`); the export is
    /// stamped so that GUI reads it as its own (`project_application::stamp_project`).
    /// `None` = the exporting slicer's own stamp.
    gui_flavour: Option<Flavour>,
    output: PathBuf,
    work_dir: PathBuf,
}

fn main() {
    if let Err(error) = parse_args(std::env::args().skip(1).collect()).and_then(|args| run(&args)) {
        eprintln!("error: STL could not be converted: {error}");
        std::process::exit(1);
    }
}

fn parse_args(raw: Vec<String>) -> Result<Args, Error> {
    let mut args = raw.into_iter();
    let (mut stl, mut vendor, mut machine, mut custom, mut output, mut work_dir) = (None, None, None, None, None, None);
    let mut presets = PathBuf::from(DEFAULT_PRESETS);
    let mut orca = PathBuf::from(DEFAULT_ORCA);
    let mut edge_margin = 0.0;
    let mut gui_flavour = None;
    while let Some(option) = args.next() {
        let value = args.next().ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--stl" => stl = Some(PathBuf::from(value)),
            "--presets" => presets = PathBuf::from(value),
            "--orca" => orca = PathBuf::from(value),
            "--vendor" => vendor = Some(value),
            "--machine" => machine = Some(value),
            "--custom-bed" => custom = Some(parse_bed(&value)?),
            "--edge-margin" => {
                edge_margin = value
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|m| m.is_finite() && *m >= 0.0)
                    .ok_or_else(|| format!("--edge-margin {value}: expected a non-negative number of mm"))?
            }
            "--gui-flavour" => {
                gui_flavour = Some(Flavour::from_name(&value).ok_or_else(|| {
                    format!("--gui-flavour {value}: expected bambu-02.09, bambu-02.08, orca-2.4, orca-2.5 or snapmaker-orca-2.4")
                })?)
            }
            "--output" => output = Some(PathBuf::from(value)),
            "--work-dir" => work_dir = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown option {option}").into()),
        }
    }
    let printer = match (vendor, machine, custom) {
        (Some(vendor), Some(machine), None) => Printer::Catalog { vendor, machine },
        (None, None, Some((width, depth, height))) => Printer::Custom { width, depth, height },
        (None, None, None) => return Err("give --vendor and --machine, or --custom-bed W,D,H".into()),
        (_, _, Some(_)) => return Err("--custom-bed excludes --vendor/--machine".into()),
        _ => return Err("--vendor and --machine go together".into()),
    };
    Ok(Args {
        stl: stl.ok_or("missing --stl")?,
        presets,
        orca,
        printer,
        edge_margin,
        gui_flavour,
        output: output.ok_or("missing --output")?,
        work_dir: work_dir.ok_or("missing --work-dir")?,
    })
}

/// `W,D,H` in millimetres, each finite and positive.
fn parse_bed(value: &str) -> Result<(f64, f64, f64), Error> {
    let numbers: Vec<f64> = value
        .split(',')
        .map(|v| v.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("--custom-bed {value}: expected W,D,H in mm"))?;
    match numbers[..] {
        [w, d, h] if [w, d, h].iter().all(|v| v.is_finite() && *v > 0.0) => Ok((w, d, h)),
        [_, _, _] => Err(format!("--custom-bed {value}: sizes must be positive").into()),
        _ => Err(format!("--custom-bed {value}: expected three numbers W,D,H").into()),
    }
}

/// The printable `area` shrunk by `margin` on every side, if a `size` (W, D, H) model fits it
/// and `height` (the machine's `printable_height`, when known).
fn usable_area(area: [f64; 4], margin: f64, size: [f64; 3], height: Option<f64>) -> Result<[f64; 4], Error> {
    let usable = [area[0] + margin, area[1] + margin, area[2] - margin, area[3] - margin];
    let (bed_w, bed_d) = (area[2] - area[0], area[3] - area[1]);
    let (use_w, use_d) = (usable[2] - usable[0], usable[3] - usable[1]);
    if use_w <= 0.0 || use_d <= 0.0 {
        return Err(format!("the {margin} mm edge margin leaves no printable area on a {bed_w:.1}x{bed_d:.1} mm bed").into());
    }
    if size[0] > use_w || size[1] > use_d {
        let minus = if margin > 0.0 { format!(" ({bed_w:.1}x{bed_d:.1} mm minus the {margin} mm edge margin)") } else { String::new() };
        return Err(format!(
            "the model's footprint {:.1}x{:.1} mm does not fit the printable area {use_w:.1}x{use_d:.1} mm{minus}",
            size[0], size[1]
        )
        .into());
    }
    if let Some(height) = height.filter(|h| size[2] > *h) {
        return Err(format!("the model is taller than the printer: {:.1} mm, printable height {height:.1} mm", size[2]).into());
    }
    Ok(usable)
}

/// `printable_height` of the machine preset (Orca writes numbers as strings).
fn printable_height(presets: &CliPresets) -> Option<f64> {
    match presets.machine.get("printable_height")? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|h: &f64| h.is_finite() && *h > 0.0)
}

fn run(args: &Args) -> Result<(), Error> {
    let stl = fs::read(&args.stl).map_err(|e| format!("cannot read {}: {e}", args.stl.display()))?;
    let size = orca_project::stl_size(&stl).map_err(|e| format!("{}: {e}", args.stl.display()))?;
    let footprint = (size[0], size[1]);
    if !args.presets.is_dir() {
        return Err(format!("slicer presets {} not found", args.presets.display()).into());
    }
    let index = PresetIndex::load(&args.presets)?;
    let presets = match &args.printer {
        Printer::Catalog { vendor, machine } => orca_project::catalog_presets(&index, vendor, machine)?,
        Printer::Custom { width, depth, height } => orca_project::custom_presets(&index, *width, *depth, *height)?,
    };
    let (area, exclusions) = presets.bed()?;
    let usable = usable_area(area, args.edge_margin, size, printable_height(&presets))?;

    fs::create_dir_all(&args.work_dir)?;
    let files = orca_project::write_cli_presets(&presets, &args.work_dir.join("presets"))?;
    // Absolute: with a relative --outputdir OrcaSlicer 2.5 (macOS) fails the export ("Project
    // export to w/export/export.3mf failed", 2026-11-01); the job runner passes relative paths.
    let export_dir = std::path::absolute(args.work_dir.join("export"))?;
    if export_dir.exists() {
        fs::remove_dir_all(&export_dir)?;
    }
    fs::create_dir_all(&export_dir)?;
    export(args, &files, &export_dir)?;
    let exported = orca_project::exported_3mf(&export_dir)?;
    if let Some(parent) = args.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    // Moved, not copied: on the runner the job directory is a size-limited tmpfs, and a second
    // copy of a large project would eat into the packer's room. Copy only across filesystems.
    if fs::rename(&exported, &args.output).is_err() {
        fs::copy(&exported, &args.output)?;
    }
    let origin = orca_project::free_spot(usable, &exclusions, footprint);
    orca_project::place_build_item(&args.output, (origin.0 + footprint.0 / 2.0, origin.1 + footprint.1 / 2.0))?;
    // The export carries the exporting slicer's stamp; the GUI the user prints with must read
    // the project as its own (Snapmaker Orca: `Snapmaker_Orca-2.4.0`), and the packer takes its
    // target GUI from the same metadata.
    if let Some(flavour) = args.gui_flavour {
        stamp_project(&args.output, flavour)?;
    }
    let settings = orca_project::project_settings(&args.output)?;
    orca_project::check_project_settings(&settings, &presets)?;
    // The export's leftovers (result.json, a copied 3MF) are not needed past this point.
    let _ = fs::remove_dir_all(&export_dir);
    print_summary(&args.output, &presets);
    Ok(())
}

fn export(args: &Args, files: &[PathBuf; 3], export_dir: &Path) -> Result<(), Error> {
    let stl = std::path::absolute(&args.stl)?;
    let mut command = orca_project::export_command(&args.orca, files, &stl, export_dir);
    let output = bounded_output(&mut command).map_err(|e| format!("cannot start {}: {e}", args.orca.display()))?;
    fs::write(args.work_dir.join("export.log"), [&output.stdout[..], &output.stderr[..]].concat())?;
    if !output.status.success() {
        let reason = failure_line(&output.stderr).or_else(|| failure_line(&output.stdout)).unwrap_or_default();
        return Err(format!("slicer export failed ({}): {reason}", output.status).into());
    }
    orca_project::check_result_json(export_dir, "slicer export")
}

fn print_summary(output: &Path, presets: &CliPresets) {
    println!(
        "source project {}: machine {}, process {}, filament {}, application {}",
        output.display(),
        presets.machine_name(),
        presets.process_name(),
        presets.filament_name(),
        project_application(output).unwrap_or_else(|| "?".to_owned())
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, Error> {
        parse_args(list.iter().map(|s| s.to_string()).collect())
    }

    const BASE: [&str; 6] = ["--stl", "m.stl", "--output", "o.3mf", "--work-dir", "w"];

    fn with(extra: &[&'static str]) -> Vec<&'static str> {
        BASE.iter().chain(extra).copied().collect()
    }

    #[test]
    fn catalog_printer_needs_vendor_and_machine() {
        let parsed = args(&with(&["--vendor", "Prusa", "--machine", "Prusa MK4S 0.4 nozzle"])).unwrap();
        assert_eq!(parsed.printer, Printer::Catalog { vendor: "Prusa".into(), machine: "Prusa MK4S 0.4 nozzle".into() });
        assert_eq!(parsed.presets, PathBuf::from(DEFAULT_PRESETS));
        assert_eq!(parsed.orca, PathBuf::from(DEFAULT_ORCA));
        assert!(args(&with(&["--machine", "Prusa MK4S 0.4 nozzle"])).is_err());
        assert!(args(&with(&["--vendor", "Prusa"])).is_err());
    }

    #[test]
    fn custom_bed_needs_three_positive_sizes() {
        let parsed = args(&with(&["--custom-bed", "300,250,220"])).unwrap();
        assert_eq!(parsed.printer, Printer::Custom { width: 300.0, depth: 250.0, height: 220.0 });
        for bad in ["300,250", "300,250,220,1", "300,0,220", "300,-1,220", "a,b,c", "300,250,inf"] {
            assert!(args(&with(&["--custom-bed", bad])).is_err(), "{bad}");
        }
        assert!(args(&with(&["--custom-bed", "300,250,220", "--vendor", "Prusa", "--machine", "M"])).is_err());
    }

    #[test]
    fn edge_margin_is_optional_and_non_negative() {
        assert_eq!(args(&with(&["--custom-bed", "300,250,220"])).unwrap().edge_margin, 0.0);
        assert_eq!(args(&with(&["--custom-bed", "300,250,220", "--edge-margin", "5"])).unwrap().edge_margin, 5.0);
        for bad in ["-1", "x", "inf"] {
            assert!(args(&with(&["--custom-bed", "300,250,220", "--edge-margin", bad])).is_err(), "{bad}");
        }
    }

    #[test]
    fn model_must_fit_the_area_minus_the_margin_and_the_height() {
        let area = [0.0, 0.0, 250.0, 210.0];
        assert_eq!(usable_area(area, 5.0, [240.0, 200.0, 100.0], Some(220.0)).unwrap(), [5.0, 5.0, 245.0, 205.0]);
        let wide = usable_area(area, 5.0, [245.0, 100.0, 10.0], None).unwrap_err().to_string();
        assert!(wide.contains("does not fit the printable area 240.0x200.0 mm (250.0x210.0 mm minus the 5 mm edge margin)"), "{wide}");
        assert!(usable_area(area, 0.0, [245.0, 100.0, 10.0], None).is_ok());
        let tall = usable_area(area, 0.0, [10.0, 10.0, 230.0], Some(220.0)).unwrap_err().to_string();
        assert!(tall.starts_with("the model is taller than the printer"), "{tall}");
        assert!(usable_area(area, 0.0, [10.0, 10.0, 230.0], None).is_ok());
        assert!(usable_area(area, 105.0, [1.0, 1.0, 1.0], None).is_err());
    }

    #[test]
    fn gui_flavour_is_optional_and_a_known_build() {
        assert_eq!(args(&with(&["--custom-bed", "300,250,220"])).unwrap().gui_flavour, None);
        let parsed = args(&with(&["--custom-bed", "300,250,220", "--gui-flavour", "snapmaker-orca-2.4"])).unwrap();
        assert_eq!(parsed.gui_flavour, Some(Flavour::Snapmaker240));
        assert!(args(&with(&["--custom-bed", "300,250,220", "--gui-flavour", "cura-5"])).is_err());
    }

    #[test]
    fn printer_and_paths_are_required() {
        assert!(args(&BASE).is_err());
        assert!(args(&["--custom-bed", "300,250,220", "--output", "o", "--work-dir", "w"]).is_err());
        assert!(args(&["--stl", "m", "--custom-bed", "300,250,220", "--work-dir", "w"]).is_err());
        assert!(args(&with(&["--custom-bed", "300,250,220", "--bogus", "x"])).is_err());
        assert!(args(&with(&["--custom-bed"])).is_err());
    }
}

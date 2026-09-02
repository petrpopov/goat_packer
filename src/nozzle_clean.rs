//! Periodic nozzle-clean macro injection into a project's `layer_change_gcode`.
//!
//! The wipe *body* is printer-specific and lives in the printer profile (`nozzle_clean_gcode`):
//! P1/X1 machines have no `G150.x` firmware macros and need raw G1 moves to the back chute,
//! while A2/P2/H2 machines wipe through `G150.3`/`G150.1`. The *conditions* (how often, from
//! what height) are slicer-template logic and composed here, machine-independent. Both halves
//! were validated on a real BambuStudio CLI slice of a P1S project — see
//! `docs/superpowers/specs/2026-09-02-nozzle-clean-injection-design.md` for the firmware
//! evidence behind each move.

use crate::colored_packer::PrinterProfile;
use std::{fs, io::Read, io::Write, path::Path};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const PROJECT_SETTINGS_ENTRY: &str = "Metadata/project_settings.config";
const LAYER_CHANGE_KEY: &str = "layer_change_gcode";

/// When the nozzle-clean macro runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NozzleCleanSpec {
    /// Every `every` layers, whole print.
    Global { every: u32 },
    /// Every `every` layers, only at/above `z_mm`.
    FromZ { z_mm: f64, every: u32 },
}

impl NozzleCleanSpec {
    /// Parses `global`, `global,every=N`, `from-z=H` or `from-z=H,every=N`. `every` defaults to
    /// 10 for `global` (the stock community-snippet cadence) and 1 for `from-z` — a targeted
    /// zone is chosen precisely because it needs active wiping.
    pub fn parse(text: &str) -> Result<Self, String> {
        const USAGE: &str = "expected 'global', 'global,every=N', 'from-z=H' or 'from-z=H,every=N' \
                             (H in mm, N >= 1; every defaults to 10 for global and 1 for from-z)";
        let mut parts = text.split(',');
        let mode = parts.next().unwrap_or_default().trim();
        let mut every = None;
        for part in parts {
            let part = part.trim();
            if let Some(value) = part.strip_prefix("every=") {
                let parsed: u32 = value.parse().map_err(|_| {
                    format!("invalid --nozzle-clean spec {text:?}: every= must be an integer >= 1, got {value:?}; {USAGE}")
                })?;
                if parsed == 0 {
                    return Err(format!(
                        "invalid --nozzle-clean spec {text:?}: every= must be >= 1; {USAGE}"
                    ));
                }
                every = Some(parsed);
            } else {
                return Err(format!(
                    "invalid --nozzle-clean spec {text:?}: unknown part {part:?}, {USAGE}"
                ));
            }
        }
        match mode {
            "global" => Ok(Self::Global {
                every: every.unwrap_or(10),
            }),
            mode if mode.starts_with("from-z=") => {
                let raw = &mode["from-z=".len()..];
                let z_mm: f64 = raw.parse().map_err(|_| {
                    format!("invalid --nozzle-clean spec {text:?}: from-z= must be a height in mm, got {raw:?}; {USAGE}")
                })?;
                if !z_mm.is_finite() || z_mm <= 0.0 {
                    return Err(format!(
                        "invalid --nozzle-clean spec {text:?}: from-z= must be > 0; {USAGE}"
                    ));
                }
                Ok(Self::FromZ {
                    z_mm,
                    every: every.unwrap_or(1),
                })
            }
            _ => Err(format!(
                "invalid --nozzle-clean spec {text:?}: unknown mode {mode:?}; {USAGE}"
            )),
        }
    }

    fn every(&self) -> u32 {
        match *self {
            Self::Global { every } | Self::FromZ { every, .. } => every,
        }
    }
}

/// Millimeters with trailing zeros trimmed (`6.590` → `6.59`, `7.000` → `7`) so composed
/// template conditions stay readable.
fn format_mm(value: f64) -> String {
    format!("{value:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

/// Composes the new `layer_change_gcode`: the conditioned wipe block followed by the project's
/// existing content, so printer-native lines (`M73`, `M991`, A2L's `M201`/`M1007`) survive —
/// the native tail differs per machine and must come from the project, not from code.
pub fn compose_layer_change_gcode(
    existing: &str,
    spec: &NozzleCleanSpec,
    body: &str,
) -> String {
    let mut conditions = 0usize;
    let mut block = String::from(";### START: NOZZLE CLEAN ###\n");
    // Never on the very first layer: the machine start gcode has just wiped the nozzle.
    block.push_str("{if layer_num > 0}\n");
    conditions += 1;
    if spec.every() > 1 {
        block.push_str(&format!("{{if (layer_num + 1) % {} == 0}}\n", spec.every()));
        conditions += 1;
    }
    if let NozzleCleanSpec::FromZ { z_mm, .. } = spec {
        // `layer_z` is accumulated float math (layer_height * n, e.g. 6.6000001), so compare a
        // hair below the requested height; the stock Bambu firmware snippets use the same
        // epsilon trick around `initial_layer_print_height`.
        block.push_str(&format!(
            "{{if layer_z >= {}}}\n",
            format_mm(z_mm - 0.01)
        ));
        conditions += 1;
    }
    for line in body.lines() {
        block.push_str(line);
        block.push('\n');
    }
    for _ in 0..conditions {
        block.push_str("{endif}\n");
    }
    block.push_str(";### END: NOZZLE CLEAN ###");
    let existing = existing.trim_matches('\n');
    if existing.is_empty() {
        block
    } else {
        format!("{block}\n{existing}")
    }
}

/// Rewrites `project` into `output` with the nozzle-clean macro injected into
/// `Metadata/project_settings.config` → `layer_change_gcode`. Every other entry is copied
/// byte-for-byte. Re-serializing the settings JSON through `serde_json::Value` alphabetizes its
/// keys — the same transformation `prepare_cli_copy` already applies to every CLI-sliced copy,
/// and BambuStudio reads those fine (the file is a settings map, order carries no meaning).
pub fn inject_nozzle_clean(
    profile: &PrinterProfile,
    project: &Path,
    output: &Path,
    spec: &NozzleCleanSpec,
) -> Result<(), String> {
    let body = profile.nozzle_clean_gcode.as_deref().ok_or_else(|| {
        format!(
            "printer profile {} ({}) defines no nozzle_clean_gcode, so nozzle-clean injection \
             is not supported for it (Snapmaker profiles intentionally omit it)",
            profile.id, profile.printer_name
        )
    })?;
    let file = fs::File::open(project)
        .map_err(|error| format!("cannot open {}: {error}", project.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| format!("cannot read 3MF {}: {error}", project.display()))?;
    let mut entries: Vec<(String, Vec<u8>)> = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("cannot read entry {index} of {}: {error}", project.display()))?;
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read {name} from {}: {error}", project.display()))?;
        if name == PROJECT_SETTINGS_ENTRY {
            let mut settings: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
                format!(
                    "cannot parse {PROJECT_SETTINGS_ENTRY} inside {}: {error}",
                    project.display()
                )
            })?;
            let existing = settings
                .get(LAYER_CHANGE_KEY)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let composed = compose_layer_change_gcode(&existing, spec, body);
            settings[LAYER_CHANGE_KEY] = serde_json::Value::String(composed);
            bytes = serde_json::to_vec(&settings).map_err(|error| {
                format!(
                    "cannot re-serialize {PROJECT_SETTINGS_ENTRY} for {}: {error}",
                    output.display()
                )
            })?;
        }
        entries.push((name, bytes));
    }
    if !entries.iter().any(|(name, _)| name == PROJECT_SETTINGS_ENTRY) {
        return Err(format!(
            "{project} has no {PROJECT_SETTINGS_ENTRY} entry; it does not look like a \
             BambuStudio/Orca project 3MF",
            project = project.display()
        ));
    }
    let writer = fs::File::create(output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;
    let mut zip = ZipWriter::new(writer);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6));
    for (name, bytes) in entries {
        if name.ends_with('/') {
            zip.add_directory(name.trim_end_matches('/'), options)
                .map_err(|error| format!("cannot write {name} into {}: {error}", output.display()))?;
        } else {
            zip.start_file(&name, options)
                .map_err(|error| format!("cannot write {name} into {}: {error}", output.display()))?;
            zip.write_all(&bytes)
                .map_err(|error| format!("cannot write {name} into {}: {error}", output.display()))?;
        }
    }
    zip.finish()
        .map_err(|error| format!("cannot finalize {}: {error}", output.display()))?;
    Ok(())
}

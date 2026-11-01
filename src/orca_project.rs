//! A slicer project (3MF) from a bare mesh through the Orca CLI, shared by `printer_catalog
//! qualify` (the probe cube) and `source_project` (a user's STL). Presets come from an Orca
//! `resources/profiles` JSON tree at run time and are flattened here for `--load-settings`; they
//! are AGPL and never land in the repo (spec 2026-11-01-printer-catalog-and-stl-design, «CLI
//! spike» appendix for the verified commands and quirks).
use std::{
    fs, io,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Map, Value, json};

use crate::orca_presets::{
    DEFAULT_FILAMENT, FILAMENT_LIBRARY, PresetIndex, exclusion_rects, parse_points, rect_from_points,
    shared_extruder_area,
};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Orca's generic printers live in the `Custom` vendor folder. The base for a user's own bed is
/// its generic Marlin machine, not the bare `fdm_machine_common`: that one is not instantiable,
/// has no `printable_area`, and its start G-code is Bambu's (`M970` vibration calibration) —
/// `MyMarlin 0.4 nozzle` (`Generic Marlin Printer`) inherits it and replaces the start G-code
/// with plain Marlin. Its `0.20mm Standard @MyMarlin` process lists it in
/// `compatible_printers` (Orca 2.4.2 image listing, 2026-11-01).
pub const CUSTOM_VENDOR: &str = "Custom";
pub const CUSTOM_BASE_MACHINE: &str = "MyMarlin 0.4 nozzle";
/// `printer_model` of a custom-bed project; the ad-hoc webapp profile carries the same
/// `printer_name` (the packer requires them equal).
pub const CUSTOM_PRINTER_MODEL: &str = "Custom Printer";
/// File name the export writes inside its `--outputdir`.
pub const EXPORT_NAME: &str = "export.3mf";

const MODEL_ENTRY: &str = "3D/3dmodel.model";
const MODEL_SETTINGS_ENTRY: &str = "Metadata/model_settings.config";
const PROJECT_SETTINGS_ENTRY: &str = "Metadata/project_settings.config";

/// Machine, process and filament presets, flattened for the CLI.
pub struct CliPresets {
    pub machine: Map<String, Value>,
    pub process: Map<String, Value>,
    pub filament: Map<String, Value>,
}

impl CliPresets {
    /// The `name` of the machine preset (the process's `compatible_printers` entry).
    pub fn machine_name(&self) -> &str {
        name_of(&self.machine)
    }

    pub fn process_name(&self) -> &str {
        name_of(&self.process)
    }

    pub fn filament_name(&self) -> &str {
        name_of(&self.filament)
    }

    /// The machine's printable area as a rectangle (the part every extruder reaches) and its
    /// `bed_exclude_area` rectangles.
    pub fn bed(&self) -> Result<([f64; 4], Vec<[f64; 4]>), Error> {
        let area = self
            .machine
            .get("printable_area")
            .and_then(parse_points)
            .as_deref()
            .and_then(rect_from_points)
            .ok_or_else(|| format!("machine preset {} has no rectangular printable_area", self.machine_name()))?;
        Ok((
            shared_extruder_area(area, self.machine.get("extruder_printable_area")),
            exclusion_rects(self.machine.get("bed_exclude_area")),
        ))
    }
}

fn name_of(map: &Map<String, Value>) -> &str {
    map.get("name").and_then(Value::as_str).unwrap_or_default()
}

/// A merged preset as the Orca CLI accepts it for `--load-settings` (spike 2026-11-01): no
/// `inherits`, `from: system`, `instantiation: true`, and `compatible_printers` set to exactly
/// the machine — the CLI compares only that list with the printer and ignores
/// `compatible_printers_condition` (return code -17 "printer is not compatible with the process
/// preset" otherwise). Harmless on the filament; the machine keeps its own keys.
pub fn flatten_for_cli(mut merged: Map<String, Value>, machine: &str) -> Map<String, Value> {
    merged.remove("inherits");
    merged.insert("from".into(), json!("system"));
    merged.insert("instantiation".into(), json!("true"));
    if merged.get("type").and_then(Value::as_str) != Some("machine") {
        merged.insert("compatible_printers".into(), json!([machine]));
    }
    merged
}

/// The resolved preset `vendor/name`, flattened for a project of `machine`.
pub fn cli_preset(index: &PresetIndex, vendor: &str, name: &str, machine: &str) -> Result<Map<String, Value>, String> {
    Ok(flatten_for_cli(index.resolve(vendor, name)?, machine))
}

/// Presets for a catalog printer: its machine preset, the process `select_process` picks and
/// `Generic PLA @System` (spec «Upload scenarios»: they only affect slicing and estimates).
pub fn catalog_presets(index: &PresetIndex, vendor: &str, machine: &str) -> Result<CliPresets, String> {
    let process = index
        .select_process(vendor, machine)
        .ok_or_else(|| format!("no process preset for {machine} in {vendor}"))?;
    Ok(CliPresets {
        machine: cli_preset(index, vendor, machine, machine)?,
        process: cli_preset(index, vendor, &process, machine)?,
        filament: cli_preset(index, FILAMENT_LIBRARY, DEFAULT_FILAMENT, machine)?,
    })
}

/// Orca's generic machine (`CUSTOM_BASE_MACHINE`) with the user's bed: `printable_area`
/// `[0,0,w,d]`, `printable_height` h, no excluded area (`["0x0"]`, what Orca itself writes for
/// none), `printer_model` `Custom Printer`. Sizes are millimetres, finite and positive.
pub fn custom_presets(index: &PresetIndex, width: f64, depth: f64, height: f64) -> Result<CliPresets, String> {
    if ![width, depth, height].iter().all(|v| v.is_finite() && *v > 0.0) {
        return Err(format!("custom bed {width}x{depth}x{height} mm must be positive"));
    }
    let mut presets = catalog_presets(index, CUSTOM_VENDOR, CUSTOM_BASE_MACHINE)?;
    let (w, d) = (number(width), number(depth));
    presets.machine.insert("printable_area".into(), json!(["0x0", format!("{w}x0"), format!("{w}x{d}"), format!("0x{d}")]));
    presets.machine.insert("printable_height".into(), json!(number(height)));
    presets.machine.insert("bed_exclude_area".into(), json!(["0x0"]));
    presets.machine.insert("printer_model".into(), json!(CUSTOM_PRINTER_MODEL));
    // A bed model/texture of the generic printer would not match the user's size.
    presets.machine.remove("bed_custom_model");
    presets.machine.remove("bed_custom_texture");
    Ok(presets)
}

/// `220` for whole millimetres, `220.5` otherwise (Orca writes its numbers as strings).
fn number(value: f64) -> String {
    format!("{value}")
}

/// Writes `machine.json`, `process.json`, `filament.json` into `dir`.
pub fn write_cli_presets(presets: &CliPresets, dir: &Path) -> io::Result<[PathBuf; 3]> {
    fs::create_dir_all(dir)?;
    let write = |file: &str, preset: &Map<String, Value>| -> io::Result<PathBuf> {
        let path = dir.join(file);
        fs::write(&path, serde_json::to_vec_pretty(&Value::Object(preset.clone()))?)?;
        Ok(path)
    };
    Ok([
        write("machine.json", &presets.machine)?,
        write("process.json", &presets.process)?,
        write("filament.json", &presets.filament)?,
    ])
}

/// The spike's export command: `<orca> --load-settings "m;p" --load-filaments f --arrange 1
/// --export-3mf export.3mf --outputdir <dir> <stl>`. `--arrange 1` is required: with
/// `--arrange 0` the 2.4.2 CLI exports the object on no plate (no `model_instance` in plate 1;
/// a slice then reads an "empty" project) wherever the STL sits. Run `place_build_item`
/// afterwards: the arrange itself is not trusted (see there).
pub fn export_command(orca: &Path, presets: &[PathBuf; 3], stl: &Path, out_dir: &Path) -> Command {
    let mut command = Command::new(orca);
    command
        .arg("--load-settings")
        .arg(format!("{};{}", presets[0].display(), presets[1].display()))
        .arg("--load-filaments")
        .arg(&presets[2])
        .args(["--arrange", "1", "--export-3mf", EXPORT_NAME, "--outputdir"])
        .arg(out_dir)
        .arg(stl);
    command
}

/// Orca/Bambu write `result.json` with `return_code` (0 = OK) and `error_string`; a slicer can
/// exit 0 with a failure recorded there.
pub fn check_result_json(dir: &Path, what: &str) -> Result<(), Error> {
    let Some(result) = read_json(&dir.join("result.json")) else { return Ok(()) };
    let code = result.get("return_code").and_then(Value::as_i64).unwrap_or(0);
    if code != 0 {
        let message = result.get("error_string").and_then(Value::as_str).unwrap_or("");
        return Err(format!("{what}: slicer return_code {code}: {message}").into());
    }
    Ok(())
}

pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// The first (by name) file in `dir` whose name passes `wanted`.
pub fn find_file(dir: &Path, wanted: impl Fn(&str) -> bool) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| wanted(&n.to_string_lossy())))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// The 3MF an export wrote into `out_dir`.
pub fn exported_3mf(out_dir: &Path) -> Result<PathBuf, Error> {
    find_file(out_dir, |name| name.ends_with(".3mf")).ok_or_else(|| "the slicer export wrote no 3MF".into())
}

/// Lower-left corner for a `size` footprint: centred in `area`, or the first spot of a 10 mm
/// scan (centre-out) whose rectangle clears every exclusion rectangle; the centre if none does.
pub fn free_spot(area: [f64; 4], exclusions: &[[f64; 4]], size: (f64, f64)) -> (f64, f64) {
    let (w, d) = size;
    let centre = ((area[0] + area[2] - w) / 2.0, (area[1] + area[3] - d) / 2.0);
    let fits = |(x, y): (f64, f64)| {
        x >= area[0] && y >= area[1] && x + w <= area[2] && y + d <= area[3]
            && exclusions.iter().all(|r| x + w <= r[0] || x >= r[2] || y + d <= r[1] || y >= r[3])
    };
    let mut spots: Vec<(f64, f64)> = Vec::new();
    let steps = ((area[2] - area[0]).max(area[3] - area[1]) / 10.0).ceil() as i32;
    for i in -steps..=steps {
        for j in -steps..=steps {
            spots.push((centre.0 + f64::from(i) * 10.0, centre.1 + f64::from(j) * 10.0));
        }
    }
    spots.sort_by(|a, b| {
        let dist = |p: &(f64, f64)| (p.0 - centre.0).hypot(p.1 - centre.1);
        dist(a).total_cmp(&dist(b))
    });
    spots.into_iter().find(|&p| fits(p)).unwrap_or(centre)
}

/// Orca's arrange leaves an object it put off the bed out of plate 1 (no `<model_instance>`), and
/// the slicer then reads the moved project as empty. Adds the instance Orca writes for a lone
/// object (object 2, instance 0, identify_id 14 as in every exported probe) when it is missing.
fn ensure_plate_instance(config: &str) -> String {
    if config.contains("<model_instance>") {
        return config.to_owned();
    }
    let Some(at) = config.find("  </plate>") else { return config.to_owned() };
    let instance = "    <model_instance>\n      <metadata key=\"object_id\" value=\"2\"/>\n      <metadata key=\"instance_id\" value=\"0\"/>\n      <metadata key=\"identify_id\" value=\"14\"/>\n    </model_instance>\n";
    format!("{}{instance}{}", &config[..at], &config[at..])
}

/// Moves the exported project's single build item so the object's centre is at `centre`. Orca's
/// `--arrange 1` misreads seam-chained `bed_exclude_area` lists (Kobra 3: cube at y = -61), so the
/// position is set here from the bed geometry we parsed ourselves.
pub fn place_build_item(project: &Path, centre: (f64, f64)) -> Result<(), Error> {
    let mut archive = zip::ZipArchive::new(fs::File::open(project)?)?;
    let mut entries = Vec::with_capacity(archive.len());
    let mut moved = false;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let (name, is_dir) = (entry.name().to_owned(), entry.is_dir());
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        if name == MODEL_ENTRY {
            bytes = centre_build_item(&String::from_utf8(bytes)?, centre)?.into_bytes();
            moved = true;
        } else if name == MODEL_SETTINGS_ENTRY {
            bytes = ensure_plate_instance(&String::from_utf8(bytes)?).into_bytes();
        }
        entries.push((name, is_dir, bytes));
    }
    drop(archive);
    if !moved {
        return Err(format!("{} has no {MODEL_ENTRY}", project.display()).into());
    }
    let mut zip = zip::ZipWriter::new(fs::File::create(project)?);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, is_dir, bytes) in entries {
        if is_dir {
            zip.add_directory(name.trim_end_matches('/'), options)?;
        } else {
            zip.start_file(&name, options)?;
            zip.write_all(&bytes)?;
        }
    }
    zip.finish()?;
    Ok(())
}

/// The 12 numbers of the first `transform="…"` inside the first `<{tag} ` element, with the
/// byte range of the attribute value.
fn first_transform(xml: &str, tag: &str) -> Result<(Vec<f64>, std::ops::Range<usize>), Error> {
    let start = xml.find(&format!("<{tag} ")).ok_or_else(|| format!("exported 3MF has no <{tag}>"))?;
    let end = xml[start..].find('>').ok_or_else(|| format!("unterminated <{tag}>"))? + start;
    let key = "transform=\"";
    let at = xml[start..end].find(key).ok_or_else(|| format!("<{tag}> without transform"))? + start + key.len();
    let close = xml[at..end].find('"').ok_or("unterminated transform")? + at;
    let m: Vec<f64> = xml[at..close]
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("bad transform: {e}"))?;
    if m.len() != 12 {
        return Err(format!("transform has {} numbers, expected 12", m.len()).into());
    }
    Ok((m, at..close))
}

/// Sets the build item's x/y translation so that item + component offset (Orca keeps the mesh
/// centred on its component origin) lands on `centre`. One object, one item.
fn centre_build_item(xml: &str, (x, y): (f64, f64)) -> Result<String, Error> {
    let (component, _) = first_transform(xml, "component")?;
    let (mut item, range) = first_transform(xml, "item")?;
    item[9] = x - component[9];
    item[10] = y - component[10];
    let text: Vec<String> = item.iter().map(|v| format!("{v}")).collect();
    Ok(format!("{}{}{}", &xml[..range.start], text.join(" "), &xml[range.end..]))
}

/// `Metadata/project_settings.config` of a 3MF as JSON.
pub fn project_settings(project: &Path) -> Result<Map<String, Value>, Error> {
    let mut archive = zip::ZipArchive::new(fs::File::open(project)?)?;
    let mut text = String::new();
    archive
        .by_name(PROJECT_SETTINGS_ENTRY)
        .map_err(|_| format!("{} has no {PROJECT_SETTINGS_ENTRY}", project.display()))?
        .read_to_string(&mut text)?;
    match serde_json::from_str(&text)? {
        Value::Object(map) => Ok(map),
        _ => Err(format!("{PROJECT_SETTINGS_ENTRY} is not a JSON object").into()),
    }
}

/// The exported project carries the presets we loaded: the machine's `printer_model` and a
/// rectangular `printable_area` equal to the machine's (a silently ignored `--load-settings`
/// would leave Orca's default printer).
pub fn check_project_settings(settings: &Map<String, Value>, presets: &CliPresets) -> Result<(), Error> {
    let model = |m: &Map<String, Value>| m.get("printer_model").and_then(Value::as_str).unwrap_or_default().to_owned();
    if model(settings) != model(&presets.machine) {
        return Err(format!(
            "exported project has printer_model '{}', expected '{}'",
            model(settings),
            model(&presets.machine)
        )
        .into());
    }
    let area = |m: &Map<String, Value>| m.get("printable_area").and_then(parse_points).as_deref().and_then(rect_from_points);
    match (area(settings), area(&presets.machine)) {
        (Some(got), Some(want)) if got.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-6) => Ok(()),
        (got, want) => Err(format!("exported project has printable_area {got:?}, expected {want:?}").into()),
    }
}

/// The most useful single line of a failed tool's output: the last `error`-looking line, else
/// the last non-empty one.
pub fn failure_line(bytes: &[u8]) -> Option<String> {
    // Headless X noise the runner prints on every slicer call (and after the real error).
    const NOISE: [&str; 4] = ["xkbcomp", "XDG_RUNTIME_DIR", "_XSERVTrans", "keysym"];
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !NOISE.iter().any(|n| l.contains(n)))
        .collect();
    let last = |pred: &dyn Fn(&str) -> bool| lines.iter().rposition(|l| pred(l));
    let lower = |l: &str| l.to_ascii_lowercase();
    let index = last(&|l| l.starts_with("slicer report:"))
        .or_else(|| last(&|l| lower(l).starts_with("error")))
        // Orca CLI: "run found error, return -51, exit..." then the message on the next line.
        .map(|i| if lines[i].starts_with("run found error") && i + 1 < lines.len() { i + 1 } else { i })
        .or_else(|| {
            last(&|l| lower(l).contains("error"))
                .map(|i| if lines[i].starts_with("run found error") && i + 1 < lines.len() { i + 1 } else { i })
        })
        .or(lines.len().checked_sub(1))?;
    Some(lines[index].chars().take(300).collect())
}

/// XY extent (width, depth) of an STL, binary or ASCII.
pub fn stl_xy_size(bytes: &[u8]) -> Result<(f64, f64), Error> {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut add = |x: f64, y: f64| {
        min = [min[0].min(x), min[1].min(y)];
        max = [max[0].max(x), max[1].max(y)];
    };
    let binary_len = bytes.get(80..84).map(|c| 84 + 50 * u32::from_le_bytes(c.try_into().expect("4 bytes")) as usize);
    if binary_len == Some(bytes.len()) {
        for triangle in bytes[84..].chunks_exact(50) {
            for vertex in 0..3 {
                let at = 12 + vertex * 12;
                let f = |o: usize| f64::from(f32::from_le_bytes(triangle[at + o..at + o + 4].try_into().expect("4 bytes")));
                add(f(0), f(4));
            }
        }
    } else if bytes.trim_ascii_start().starts_with(b"solid") {
        for line in String::from_utf8_lossy(bytes).lines() {
            let mut words = line.split_whitespace();
            if words.next() == Some("vertex") {
                let xy: Vec<f64> = words.take(2).filter_map(|w| w.parse().ok()).collect();
                if let [x, y] = xy[..] {
                    add(x, y);
                }
            }
        }
    } else {
        return Err("not a binary or ASCII STL".into());
    }
    if !(min[0].is_finite() && max[0].is_finite()) {
        return Err("STL has no triangles".into());
    }
    Ok((max[0] - min[0], max[1] - min[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_fixture() -> PresetIndex {
        PresetIndex::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog/presets")).unwrap()
    }

    fn custom_fixture() -> PresetIndex {
        PresetIndex::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/orca_project/presets")).unwrap()
    }

    #[test]
    fn cli_presets_are_flat_system_presets_compatible_with_the_machine() {
        let index = catalog_fixture();
        let machine = "Acme Ring 0.4 nozzle";
        let process = cli_preset(&index, "Acme", "0.20mm Speed @Acme Ring 0.4", machine).unwrap();
        assert!(process.get("inherits").is_none());
        assert_eq!(process["from"], "system");
        assert_eq!(process["instantiation"], "true");
        assert_eq!(process["compatible_printers"], json!([machine]));
        assert_eq!(process["layer_height"], "0.2");
        let printer = cli_preset(&index, "Acme", machine, machine).unwrap();
        assert!(printer.get("inherits").is_none() && printer.get("compatible_printers").is_none());
        assert_eq!(printer["printable_height"], "250");
        let filament = cli_preset(&index, FILAMENT_LIBRARY, DEFAULT_FILAMENT, machine).unwrap();
        assert_eq!(filament["compatible_printers"], json!([machine]));
    }

    #[test]
    fn catalog_presets_pick_process_and_generic_pla() {
        let presets = catalog_presets(&catalog_fixture(), "Acme", "Acme Box 0.4 nozzle").unwrap();
        assert_eq!(presets.machine_name(), "Acme Box 0.4 nozzle");
        assert_eq!(presets.process_name(), "0.20mm Standard @Acme");
        assert_eq!(presets.filament_name(), DEFAULT_FILAMENT);
        assert_eq!(presets.bed().unwrap(), ([0.0, 0.0, 220.0, 220.0], vec![[0.0, 0.0, 18.0, 28.0]]));
        assert!(catalog_presets(&catalog_fixture(), "Acme", "Nope 0.4 nozzle").is_err());
    }

    #[test]
    fn custom_presets_put_the_users_bed_on_the_generic_machine() {
        let presets = custom_presets(&custom_fixture(), 300.0, 250.0, 220.0).unwrap();
        assert_eq!(presets.machine_name(), CUSTOM_BASE_MACHINE);
        assert_eq!(presets.machine["printable_area"], json!(["0x0", "300x0", "300x250", "0x250"]));
        assert_eq!(presets.machine["printable_height"], "220");
        assert_eq!(presets.machine["bed_exclude_area"], json!(["0x0"]));
        assert_eq!(presets.machine["printer_model"], CUSTOM_PRINTER_MODEL);
        assert_eq!(presets.machine["gcode_flavor"], "marlin");
        assert!(presets.machine.get("inherits").is_none());
        assert_eq!(presets.process_name(), "0.20mm Standard @MyMarlin");
        assert_eq!(presets.process["compatible_printers"], json!([CUSTOM_BASE_MACHINE]));
        assert_eq!(presets.filament["compatible_printers"], json!([CUSTOM_BASE_MACHINE]));
        assert_eq!(presets.bed().unwrap(), ([0.0, 0.0, 300.0, 250.0], vec![]));
        let fractional = custom_presets(&custom_fixture(), 220.5, 220.0, 250.0).unwrap();
        assert_eq!(fractional.machine["printable_area"][1], "220.5x0");
        assert!(custom_presets(&custom_fixture(), 300.0, 0.0, 220.0).is_err());
        assert!(custom_presets(&custom_fixture(), f64::NAN, 250.0, 220.0).is_err());
        // Without Orca's Custom folder there is no base machine.
        assert!(custom_presets(&catalog_fixture(), 300.0, 250.0, 220.0).is_err());
    }

    #[test]
    fn presets_are_written_and_the_export_command_has_the_spike_shape() {
        let dir = tempfile::tempdir().unwrap();
        let presets = catalog_presets(&catalog_fixture(), "Acme", "Acme Box 0.4 nozzle").unwrap();
        let paths = write_cli_presets(&presets, dir.path()).unwrap();
        let machine: Value = serde_json::from_slice(&fs::read(&paths[0]).unwrap()).unwrap();
        assert_eq!(machine["name"], "Acme Box 0.4 nozzle");
        let command = export_command(Path::new("/orca"), &paths, Path::new("/in/m.stl"), Path::new("/out"));
        let args: Vec<String> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args[0], "--load-settings");
        assert_eq!(args[1], format!("{};{}", paths[0].display(), paths[1].display()));
        assert_eq!(&args[2..], ["--load-filaments", &paths[2].display().to_string(), "--arrange", "1", "--export-3mf", EXPORT_NAME, "--outputdir", "/out", "/in/m.stl"]);
    }

    #[test]
    fn free_spot_is_centred_and_off_exclusions() {
        assert_eq!(free_spot([0.0, 0.0, 256.0, 256.0], &[], (20.0, 20.0)), (118.0, 118.0));
        let (x, y) = free_spot([0.0, 0.0, 256.0, 256.0], &[[100.0, 100.0, 156.0, 156.0]], (20.0, 20.0));
        assert!(x + 20.0 <= 100.0 || x >= 156.0 || y + 20.0 <= 100.0 || y >= 156.0, "({x}, {y})");
        assert_eq!(free_spot([25.0, 0.0, 325.0, 320.0], &[], (20.0, 20.0)), (165.0, 150.0));
        assert_eq!(free_spot([0.0, 0.0, 100.0, 100.0], &[], (40.0, 10.0)), (30.0, 45.0));
    }

    #[test]
    fn missing_plate_instance_is_added_once() {
        let empty = "<config>\n  <plate>\n    <metadata key=\"plater_id\" value=\"1\"/>\n  </plate>\n</config>\n";
        let fixed = ensure_plate_instance(empty);
        assert!(fixed.contains("<model_instance>\n      <metadata key=\"object_id\" value=\"2\"/>"), "{fixed}");
        assert!(fixed.find("<model_instance>").unwrap() < fixed.find("</plate>").unwrap());
        assert_eq!(ensure_plate_instance(&fixed), fixed);
    }

    #[test]
    fn build_item_is_centred_through_the_component_offset() {
        let xml = r#"<object><components><component objectid="1" transform="1 0 0 0 1 0 0 0 1 10 10 5"/></components></object>
 <build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 10 -61 0" printable="1"/></build>"#;
        let moved = centre_build_item(xml, (127.5, 127.5)).unwrap();
        assert!(moved.contains(r#"<item objectid="2" transform="1 0 0 0 1 0 0 0 1 117.5 117.5 0" printable="1"/>"#), "{moved}");
        assert!(moved.contains(r#"transform="1 0 0 0 1 0 0 0 1 10 10 5""#));
        assert!(centre_build_item("<build></build>", (1.0, 1.0)).is_err());
    }

    #[test]
    fn slicer_result_json_failures_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_result_json(dir.path(), "x").is_ok());
        fs::write(dir.path().join("result.json"), r#"{"return_code": -17, "error_string": "not compatible"}"#).unwrap();
        let error = check_result_json(dir.path(), "probe export").unwrap_err().to_string();
        assert!(error.contains("-17") && error.contains("not compatible"), "{error}");
        fs::write(dir.path().join("result.json"), r#"{"return_code": 0, "error_string": "Success."}"#).unwrap();
        assert!(check_result_json(dir.path(), "x").is_ok());
    }

    #[test]
    fn exported_settings_must_carry_the_loaded_machine() {
        let presets = custom_presets(&custom_fixture(), 220.0, 220.0, 250.0).unwrap();
        let good = json!({"printer_model": "Custom Printer", "printable_area": ["0x0", "220x0", "220x220", "0x220"]});
        assert!(check_project_settings(good.as_object().unwrap(), &presets).is_ok());
        let other = json!({"printer_model": "Generic Marlin Printer", "printable_area": ["0x0", "220x0", "220x220", "0x220"]});
        assert!(check_project_settings(other.as_object().unwrap(), &presets).is_err());
        let default_bed = json!({"printer_model": "Custom Printer", "printable_area": ["0x0", "250x0", "250x250", "0x250"]});
        assert!(check_project_settings(default_bed.as_object().unwrap(), &presets).is_err());
        let none = json!({"printer_model": "Custom Printer"});
        assert!(check_project_settings(none.as_object().unwrap(), &presets).is_err());
    }

    #[test]
    fn project_settings_are_read_from_the_3mf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.3mf");
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        zip.start_file(PROJECT_SETTINGS_ENTRY, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(br#"{"printer_model": "X"}"#).unwrap();
        zip.finish().unwrap();
        assert_eq!(project_settings(&path).unwrap()["printer_model"], "X");
        let empty = dir.path().join("e.3mf");
        zip::ZipWriter::new(fs::File::create(&empty).unwrap()).finish().unwrap();
        assert!(project_settings(&empty).is_err());
    }

    #[test]
    fn failure_line_prefers_the_last_error() {
        let out = b"stage 1\nerror: printer_model 'X' implies slicer 'bambu'\nbye\n";
        assert_eq!(failure_line(out).unwrap(), "error: printer_model 'X' implies slicer 'bambu'");
        assert_eq!(failure_line(b"a\n\nlast\n").unwrap(), "last");
        assert_eq!(failure_line(b""), None);
        // Runner noise after the real error is skipped; the packer's slicer report wins.
        let packer = b"error: Qidi Q2 slicing failed (exit status: 192)\nslicer report: Object conflicts were detected.\nErrors from xkbcomp are not fatal to the X server\n";
        assert_eq!(failure_line(packer).unwrap(), "slicer report: Object conflicts were detected.");
        let orca = b"run found error, return -51, exit...\nRelative extruder addressing requires resetting\nErrors from xkbcomp are not fatal to the X server\n";
        assert_eq!(failure_line(orca).unwrap(), "Relative extruder addressing requires resetting");
    }

    #[test]
    fn stl_size_from_binary_and_ascii() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.stl");
        crate::embedded_mesh::write_binary_stl(&path, &[[(1.0, 2.0, 0.0), (21.0, 2.0, 0.0), (1.0, 12.0, 5.0)]]).unwrap();
        assert_eq!(stl_xy_size(&fs::read(&path).unwrap()).unwrap(), (20.0, 10.0));
        let ascii = b"solid t\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 30 0 0\n   vertex 0 5.5 0\n  endloop\n endfacet\nendsolid t\n";
        assert_eq!(stl_xy_size(ascii).unwrap(), (30.0, 5.5));
        assert!(stl_xy_size(b"garbage").is_err());
        assert!(stl_xy_size(b"solid empty\nendsolid empty\n").is_err());
    }
}

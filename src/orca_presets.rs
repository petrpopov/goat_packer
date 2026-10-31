//! OrcaSlicer system presets → facts for our printer profiles. Reads the JSON preset tree of a
//! pinned OrcaSlicer checkout (`resources/profiles/<Vendor>/{machine,process,filament}/*.json`);
//! the installed app ships binary caches instead. Only numbers leave this module — the presets
//! are AGPL and are never copied (spec 2026-11-01-printer-catalog-and-stl-design, «Licensing»).
use std::{collections::BTreeMap, fs, io, path::Path};

use serde_json::{Map, Value, json};

/// Orca's shared filament vendor: `inherits` of a vendor filament falls back here.
pub const FILAMENT_LIBRARY: &str = "OrcaFilamentLibrary";
/// Default filament for catalog probes and STL builds (spec «Upload scenarios»).
pub const DEFAULT_FILAMENT: &str = "Generic PLA @System";

/// Presets keyed by (vendor folder, name). Names are NOT unique across vendors: ~130 repeat at
/// Orca 2.4.2 (`fdm_machine_common`, `fdm_filament_pla`, …) with different contents, and a
/// name-only map silently picked another vendor's base (CLI spike 2026-11-01: a wrong 3.9 kB
/// Creality machine). A BTreeMap keeps iteration (and the any-vendor fallback) deterministic.
#[derive(Default)]
pub struct PresetIndex {
    presets: BTreeMap<(String, String), Map<String, Value>>,
}

pub struct MachineEntry<'a> {
    pub vendor: String,
    pub name: String,
    pub raw: &'a Map<String, Value>,
}

impl PresetIndex {
    /// Every `*.json` under `<root>/<Vendor>/…` with a `name`; the vendor is the top folder.
    pub fn load(root: &Path) -> io::Result<Self> {
        let mut index = Self::default();
        for vendor_dir in fs::read_dir(root)? {
            let vendor_dir = vendor_dir?;
            if !vendor_dir.file_type()?.is_dir() {
                continue;
            }
            let vendor = vendor_dir.file_name().to_string_lossy().into_owned();
            let mut stack = vec![vendor_dir.path()];
            while let Some(dir) = stack.pop() {
                for entry in fs::read_dir(&dir)? {
                    let path = entry?.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().is_some_and(|e| e == "json") {
                        if let Ok(value) = serde_json::from_slice::<Value>(&fs::read(&path)?) {
                            index.insert(&vendor, value);
                        }
                    }
                }
            }
        }
        Ok(index)
    }

    pub fn insert(&mut self, vendor: &str, value: Value) {
        if let Value::Object(map) = value {
            if let Some(name) = map.get("name").and_then(Value::as_str) {
                self.presets.insert((vendor.to_owned(), name.to_owned()), map);
            }
        }
    }

    /// The raw preset (no `inherits` merge) of this vendor with this exact name, any type
    /// (e.g. `machine_model`).
    pub fn get(&self, vendor: &str, name: &str) -> Option<&Map<String, Value>> {
        self.presets.get(&(vendor.to_owned(), name.to_owned()))
    }

    /// Where a reference to `name` made from `vendor` lands: the same vendor first, then
    /// Orca's filament library, then any vendor (alphabetical) as a last resort.
    fn locate(&self, vendor: &str, name: &str) -> Option<(&str, &Map<String, Value>)> {
        for v in [vendor, FILAMENT_LIBRARY] {
            if let Some(((v, _), map)) = self.presets.get_key_value(&(v.to_owned(), name.to_owned())) {
                return Some((v.as_str(), map));
            }
        }
        self.presets.iter().find(|((_, n), _)| n == name).map(|((v, _), map)| (v.as_str(), map))
    }

    fn of_type<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = (&'a str, &'a str, &'a Map<String, Value>)> + 'a {
        self.presets
            .iter()
            .filter(move |(_, m)| m.get("type").and_then(Value::as_str) == Some(kind))
            .map(|((v, n), m)| (v.as_str(), n.as_str(), m))
    }

    /// Instantiable machine presets (what Orca shows in its printer list), sorted by vendor, name.
    pub fn machines(&self) -> impl Iterator<Item = MachineEntry<'_>> {
        self.of_type("machine")
            .filter(|(_, _, m)| m.get("instantiation").and_then(Value::as_str) == Some("true"))
            .map(|(vendor, name, raw)| MachineEntry { vendor: vendor.to_owned(), name: name.to_owned(), raw })
    }

    /// The preset with its `inherits` chain merged (child keys win). Each parent is looked up
    /// from the vendor of the preset naming it (see `locate`). Errors on a missing preset or
    /// parent, or a cycle.
    pub fn resolve(&self, vendor: &str, name: &str) -> Result<Map<String, Value>, String> {
        let first = self.get(vendor, name).ok_or_else(|| format!("preset not found: {vendor}/{name}"))?;
        let mut chain: Vec<(&str, &Map<String, Value>)> = Vec::new();
        let mut current = (vendor, first);
        loop {
            let (v, map) = current;
            if chain.iter().any(|(sv, sm)| *sv == v && std::ptr::eq(*sm, map)) {
                return Err(format!("inherits cycle at {v}/{}", map.get("name").and_then(Value::as_str).unwrap_or_default()));
            }
            chain.push(current);
            let Some(parent) = map.get("inherits").and_then(Value::as_str).filter(|s| !s.is_empty()) else { break };
            current = self.locate(v, parent).ok_or_else(|| format!("preset not found: {parent} (inherited in {v})"))?;
        }
        let mut merged = Map::new();
        for (_, map) in chain.into_iter().rev() {
            for (k, v) in map {
                merged.insert(k.clone(), v.clone());
            }
        }
        Ok(merged)
    }

    /// The process preset for a catalog probe / STL build of `machine` (vendor-scoped):
    /// 1. an instantiable process of the vendor whose `compatible_printers` lists the machine
    ///    (BBL style), best `process_rank`, then by name;
    /// 2. else a 0.20 mm process whose `@<suffix>` names the machine (Prusa, Creality: they
    ///    match by `compatible_printers_condition`, which the CLI ignores), `Standard` first;
    /// 3. else the vendor's base process (`fdm_process_common`, else the first root process).
    /// Returns the process name (it lives in `vendor`).
    pub fn select_process(&self, vendor: &str, machine: &str) -> Option<String> {
        let processes: Vec<(&str, &Map<String, Value>)> =
            self.of_type("process").filter(|(v, _, _)| *v == vendor).map(|(_, n, m)| (n, m)).collect();
        // Instantiable processes, resolved once, with their rank (layer height from the merged
        // `layer_height`, not the name: Volumic names read `Compatible speed - 0.20mm`).
        let mut candidates: Vec<((u8, u8), &str, Map<String, Value>)> = processes
            .iter()
            .filter(|(_, m)| m.get("instantiation").and_then(Value::as_str) == Some("true"))
            .filter_map(|(n, _)| {
                let resolved = self.resolve(vendor, n).ok()?;
                let layer = resolved.get("layer_height").and_then(first_number).or_else(|| layer_height(n));
                Some((process_rank(n, layer), *n, resolved))
            })
            .collect();
        candidates.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        let listed = candidates.iter().find(|(_, _, r)| {
            r.get("compatible_printers")
                .and_then(Value::as_array)
                .is_some_and(|list| list.iter().any(|p| p.as_str() == Some(machine)))
        });
        if let Some((_, name, _)) = listed {
            return Some((*name).to_owned());
        }
        let keys = machine_keys(vendor, machine);
        let named = candidates.iter().find(|(rank, n, _)| {
            rank.0 == 0 && n.split_once('@').is_some_and(|(_, suffix)| keys.contains(&normalize(suffix)))
        });
        if let Some((_, name, _)) = named {
            return Some((*name).to_owned());
        }
        let roots = || processes.iter().filter(|(_, m)| m.get("inherits").and_then(Value::as_str).is_none_or(str::is_empty));
        roots().find(|(n, _)| *n == "fdm_process_common").or_else(|| roots().next()).map(|(n, _)| (*n).to_owned())
    }

    /// The resolved preset ready for `--load-settings`/`--load-filaments` (see `flatten_for_cli`).
    pub fn cli_preset(&self, vendor: &str, name: &str, machine: &str) -> Result<Map<String, Value>, String> {
        Ok(flatten_for_cli(self.resolve(vendor, name)?, machine))
    }
}

/// Orca numbers: `"0.2"`, `["0.2"]` or a JSON number.
fn first_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Array(a) => a.first().and_then(first_number),
        _ => None,
    }
}

/// The leading layer height of a process name: `0.20mm Standard @X` and Snapmaker's
/// `0.20 Standard @Snapmaker (0.4 nozzle)` → 0.2.
fn layer_height(name: &str) -> Option<f64> {
    let head: String = name.trim().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    head.parse().ok()
}

/// Lower is better. First key: 0.20 mm layers before everything else. Second: `Standard`, then
/// everyday variants (Prusa has no Standard: `SPEED`/`STRUCTURAL`), then other ones, then
/// material- or toolhead-specific ones (`SOLUBLE FULL`, `TPU`, `MMU`). Ties go by name.
fn process_rank(name: &str, layer_height: Option<f64>) -> (u8, u8) {
    let layer = u8::from(layer_height.is_none_or(|h| (h - 0.2).abs() > 1e-9));
    let lower = name.to_ascii_lowercase();
    let variant = lower.split('@').next().unwrap_or("");
    let words: Vec<&str> = variant.split_whitespace().skip(1).collect();
    let has = |list: &[&str]| list.iter().any(|w| words.iter().any(|v| v.starts_with(w)));
    let kind = if has(&["soluble", "tpu", "pet", "mmu", "mm", "support", "interface"]) {
        3
    } else if has(&["standard"]) {
        0
    } else if has(&["speed", "structural", "quality", "optimal", "balanced", "fine"]) {
        1
    } else {
        2
    };
    (layer, kind)
}

/// Lowercase ASCII alphanumerics only: `Ender-3 V3 SE` and `Ender3V3SE` compare equal.
fn normalize(text: &str) -> String {
    text.chars().filter(char::is_ascii_alphanumeric).flat_map(|c| c.to_lowercase()).collect()
}

/// The ways a process `@suffix` can name a machine preset: its name without ` nozzle`, also
/// without the first word or the vendor folder name (`Prusa MK4S 0.4 nozzle` → `mk4s04`).
fn machine_keys(vendor: &str, machine: &str) -> Vec<String> {
    let stem = machine.trim().strip_suffix(" nozzle").unwrap_or(machine.trim());
    let mut keys = vec![normalize(stem)];
    if let Some((_, rest)) = stem.split_once(' ') {
        keys.push(normalize(rest));
    }
    if let Some(rest) = stem.strip_prefix(vendor) {
        keys.push(normalize(rest));
    }
    keys
}

/// A merged preset as the Orca CLI accepts it for `--load-settings` (spike 2026-11-01): no
/// `inherits`, `from: system`, `instantiation: true`, and `compatible_printers` set to exactly
/// the machine — the CLI compares only that list with the printer and ignores
/// `compatible_printers_condition` (return code -17 "printer is not compatible with the process
/// preset" otherwise). Harmless on the machine and the filament.
pub fn flatten_for_cli(mut merged: Map<String, Value>, machine: &str) -> Map<String, Value> {
    merged.remove("inherits");
    merged.insert("from".into(), json!("system"));
    merged.insert("instantiation".into(), json!("true"));
    if merged.get("type").and_then(Value::as_str) != Some("machine") {
        merged.insert("compatible_printers".into(), json!([machine]));
    }
    merged
}

/// Orca point lists: `["0x0","256x0",…]` or one string `"0x0,220x0,…"`.
pub fn parse_points(value: &Value) -> Option<Vec<(f64, f64)>> {
    let items: Vec<String> = match value {
        Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
        Value::String(s) => s.split(',').map(str::to_owned).collect(),
        _ => return None,
    };
    items
        .iter()
        .map(|p| {
            let (x, y) = p.trim().split_once('x')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect()
}

/// `[x0,y0,x1,y1]` when the points are exactly an axis-aligned rectangle; deltas/polygons → None.
pub fn rect_from_points(points: &[(f64, f64)]) -> Option<[f64; 4]> {
    let mut xs: Vec<f64> = points.iter().map(|p| p.0).collect();
    let mut ys: Vec<f64> = points.iter().map(|p| p.1).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    (points.len() == 4 && xs.len() == 2 && ys.len() == 2 && xs[1] > xs[0] && ys[1] > ys[0]).then(|| [xs[0], ys[0], xs[1], ys[1]])
}

/// `bed_exclude_area` → rectangles. One polygon per preset in Orca; a non-rectangular polygon is
/// kept as its bounding rectangle (conservative), a degenerate one (`"0x0"`) is dropped.
pub fn exclusion_rects(value: Option<&Value>) -> Vec<[f64; 4]> {
    let Some(points) = value.and_then(parse_points) else { return Vec::new() };
    if points.len() < 3 {
        return Vec::new();
    }
    let min_x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let max_x = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let min_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_y = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    if max_x - min_x < 1e-6 || max_y - min_y < 1e-6 {
        return Vec::new();
    }
    vec![[min_x, min_y, max_x, max_y]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture() -> PresetIndex {
        PresetIndex::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog/presets")).unwrap()
    }

    #[test]
    fn child_overrides_parent_and_keeps_inherited_keys() {
        let box_preset = fixture().resolve("Acme", "Acme Box 0.4 nozzle").unwrap();
        assert_eq!(box_preset.get("printer_model").and_then(|v| v.as_str()), Some("Acme Box"));
        assert_eq!(box_preset.get("printable_height").and_then(|v| v.as_str()), Some("250"));
        assert_eq!(rect_from_points(&parse_points(&box_preset["printable_area"]).unwrap()), Some([0.0, 0.0, 220.0, 220.0]));
    }

    #[test]
    fn vendor_comes_from_the_folder_and_only_instantiable_machines_are_listed() {
        let index = fixture();
        let machines: Vec<(String, String)> = index.machines().map(|m| (m.vendor, m.name)).collect();
        let expected = [("Acme", "Acme Box 0.4 nozzle"), ("Acme", "Acme Box 0.6 nozzle"), ("Acme", "Acme Ring 0.4 nozzle")].map(|(v, n)| (v.to_owned(), n.to_owned()));
        assert_eq!(machines, expected);
    }

    #[test]
    fn comma_string_points_and_degenerate_exclusions() {
        let ring = fixture().resolve("Acme", "Acme Ring 0.4 nozzle").unwrap();
        assert_eq!(rect_from_points(&parse_points(&ring["printable_area"]).unwrap()), Some([0.0, 0.0, 300.0, 300.0]));
        assert_eq!(exclusion_rects(ring.get("bed_exclude_area")), Vec::<[f64; 4]>::new());
    }

    #[test]
    fn exclusion_polygon_becomes_its_bounding_rect() {
        let tri = serde_json::json!(["0x0", "30x0", "0x20"]);
        assert_eq!(exclusion_rects(Some(&tri)), vec![[0.0, 0.0, 30.0, 20.0]]);
    }

    #[test]
    fn non_rectangular_area_is_rejected() {
        let circle: Vec<(f64, f64)> = (0..32).map(|i| { let a = i as f64 / 32.0 * std::f64::consts::TAU; (100.0 * a.cos(), 100.0 * a.sin()) }).collect();
        assert_eq!(rect_from_points(&circle), None);
    }

    #[test]
    fn inherits_cycle_is_an_error() {
        let mut index = PresetIndex::default();
        index.insert("Acme", serde_json::json!({"name": "a", "inherits": "b", "type": "machine"}));
        index.insert("Acme", serde_json::json!({"name": "b", "inherits": "a", "type": "machine"}));
        assert!(index.resolve("Acme", "a").is_err());
    }

    #[test]
    fn inherits_resolve_inside_the_vendor_first() {
        // Acme and Zeta both define `fdm_machine_common` with different values; Zeta sorts
        // last, so a global name lookup would hand it Acme's.
        let index = fixture();
        let zeta = index.resolve("Zeta", "Zeta Cube 0.4 nozzle").unwrap();
        assert_eq!(zeta["gcode_flavor"], "marlin");
        let acme = index.resolve("Acme", "Acme Box 0.4 nozzle").unwrap();
        assert_eq!(acme["gcode_flavor"], "klipper");
    }

    #[test]
    fn filament_parents_fall_back_to_the_library_before_other_vendors() {
        let index = fixture();
        // Acme has its own fdm_filament_pla (999); Zeta has none → OrcaFilamentLibrary (220),
        // not Acme's.
        assert_eq!(index.resolve("Acme", "Acme PLA").unwrap()["nozzle_temperature"], json!(["999"]));
        assert_eq!(index.resolve("Zeta", "Zeta PLA").unwrap()["nozzle_temperature"], json!(["220"]));
        let generic = index.resolve(FILAMENT_LIBRARY, DEFAULT_FILAMENT).unwrap();
        assert_eq!(generic["filament_type"], json!(["PLA"]));
    }

    #[test]
    fn any_vendor_is_the_last_resort() {
        let mut index = PresetIndex::default();
        index.insert("Other", json!({"name": "base", "type": "machine", "k": "v"}));
        index.insert("Acme", json!({"name": "leaf", "inherits": "base", "type": "machine"}));
        assert_eq!(index.resolve("Acme", "leaf").unwrap()["k"], "v");
        assert!(index.resolve("Acme", "missing").is_err());
    }

    #[test]
    fn process_listing_the_machine_wins_and_prefers_0_20mm_standard() {
        let index = fixture();
        // Both `0.20mm Standard @Acme` and `0.12mm Fine @Acme` list the Box; the Ring
        // processes don't.
        assert_eq!(index.select_process("Acme", "Acme Box 0.4 nozzle").as_deref(), Some("0.20mm Standard @Acme"));
    }

    #[test]
    fn process_named_after_the_machine_and_nozzle_when_none_lists_it() {
        let index = fixture();
        // Ring processes match by condition only (empty list); the 0.6 one is another nozzle.
        assert_eq!(index.select_process("Acme", "Acme Ring 0.4 nozzle").as_deref(), Some("0.20mm Speed @Acme Ring 0.4"));
    }

    #[test]
    fn vendor_base_process_is_the_fallback() {
        let index = fixture();
        assert_eq!(index.select_process("Zeta", "Zeta Cube 0.4 nozzle").as_deref(), Some("fdm_process_common"));
        assert_eq!(index.select_process("Nobody", "X 0.4 nozzle"), None);
    }

    #[test]
    fn process_rank_prefers_everyday_profiles() {
        assert_eq!(process_rank("0.20mm Standard @BBL X1C", layer_height("0.20mm Standard @BBL X1C")), (0, 0));
        assert_eq!(process_rank("0.20 Standard @Snapmaker (0.4 nozzle)", layer_height("0.20 Standard @Snapmaker (0.4 nozzle)")), (0, 0));
        assert_eq!(process_rank("0.20mm SPEED @MK4S 0.4", layer_height("0.20mm SPEED @MK4S 0.4")), (0, 1));
        assert_eq!(process_rank("0.20mm Strength @X", layer_height("0.20mm Strength @X")), (0, 2));
        assert_eq!(process_rank("0.20mm SOLUBLE FULL @MK4S 0.4", layer_height("0.20mm SOLUBLE FULL @MK4S 0.4")), (0, 3));
        assert_eq!(process_rank("0.20mm PET-CF 0.4 Nozzle Standard @X", layer_height("0.20mm PET-CF 0.4 Nozzle Standard @X")), (0, 3));
        assert_eq!(process_rank("0.24mm Standard @X 0.8 nozzle", layer_height("0.24mm Standard @X 0.8 nozzle")), (1, 0));
        assert!(process_rank("0.08 Extra Fine @Snapmaker (0.4 nozzle)", layer_height("0.08 Extra Fine @Snapmaker (0.4 nozzle)")) > process_rank("0.20 Standard @Snapmaker (0.4 nozzle)", layer_height("0.20 Standard @Snapmaker (0.4 nozzle)")));
        assert_eq!(process_rank("fdm_process_common", None), (1, 2));
        assert_eq!(process_rank("Compatible speed - 0.20mm", Some(0.2)), (0, 1));
        assert!(process_rank("0.20mm SPEED @MK4S 0.4", layer_height("0.20mm SPEED @MK4S 0.4")) < process_rank("0.20mm SOLUBLE FULL @MK4S 0.4", layer_height("0.20mm SOLUBLE FULL @MK4S 0.4")));
    }

    #[test]
    fn machine_keys_match_orca_suffix_spellings() {
        let keys = machine_keys("Creality", "Creality Ender-3 V3 SE 0.4 nozzle");
        assert!(keys.contains(&normalize("Creality Ender3V3SE 0.4")));
        let keys = machine_keys("Prusa", "Prusa MK4S HF0.4 nozzle");
        assert!(keys.contains(&normalize("MK4S HF0.4")));
        assert!(!keys.contains(&normalize("MK4S 0.4")));
    }

    #[test]
    fn cli_presets_are_flat_system_presets_compatible_with_the_machine() {
        let index = fixture();
        let machine = "Acme Ring 0.4 nozzle";
        let process = index.cli_preset("Acme", "0.20mm Speed @Acme Ring 0.4", machine).unwrap();
        assert!(process.get("inherits").is_none());
        assert_eq!(process["from"], "system");
        assert_eq!(process["instantiation"], "true");
        assert_eq!(process["compatible_printers"], json!([machine]));
        assert_eq!(process["layer_height"], "0.2");
        let printer = index.cli_preset("Acme", machine, machine).unwrap();
        assert!(printer.get("inherits").is_none() && printer.get("compatible_printers").is_none());
        assert_eq!(printer["printable_height"], "250");
        let filament = index.cli_preset(FILAMENT_LIBRARY, DEFAULT_FILAMENT, machine).unwrap();
        assert_eq!(filament["compatible_printers"], json!([machine]));
    }

    #[test]
    fn empty_bed_exclude_area_list_has_no_rectangles() {
        // Orca writes an empty exclusion as `["0x0"]` in exported 3MFs (spike 2026-11-01).
        assert!(exclusion_rects(Some(&json!(["0x0"]))).is_empty());
        assert!(exclusion_rects(Some(&json!([]))).is_empty());
        assert!(exclusion_rects(None).is_empty());
    }
}

//! OrcaSlicer system presets → facts for our printer profiles. Reads the JSON preset tree of a
//! pinned OrcaSlicer checkout (`resources/profiles/<Vendor>/{machine,process,filament}/*.json`);
//! the installed app ships binary caches instead. Only numbers leave this module — the presets
//! are AGPL and are never copied (spec 2026-11-01-printer-catalog-and-stl-design, «Licensing»).
use std::{collections::HashMap, fs, io, path::Path};

use serde_json::{Map, Value};

#[derive(Default)]
pub struct PresetIndex {
    by_name: HashMap<String, (String, Map<String, Value>)>,
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
                self.by_name.insert(name.to_owned(), (vendor.to_owned(), map));
            }
        }
    }

    /// The raw preset (no `inherits` merge) with this exact name, any type (e.g. `machine_model`).
    pub fn get(&self, name: &str) -> Option<(&str, &Map<String, Value>)> {
        self.by_name.get(name).map(|(vendor, map)| (vendor.as_str(), map))
    }

    /// Instantiable machine presets (what Orca shows in its printer list), sorted by vendor, name.
    pub fn machines(&self) -> impl Iterator<Item = MachineEntry<'_>> {
        let mut list: Vec<_> = self
            .by_name
            .iter()
            .filter(|(_, (_, m))| m.get("type").and_then(Value::as_str) == Some("machine") && m.get("instantiation").and_then(Value::as_str) == Some("true"))
            .map(|(name, (vendor, raw))| MachineEntry { vendor: vendor.clone(), name: name.clone(), raw })
            .collect();
        list.sort_by(|a, b| (&a.vendor, &a.name).cmp(&(&b.vendor, &b.name)));
        list.into_iter()
    }

    /// The preset with its `inherits` chain merged (child keys win). Errors on a missing parent
    /// or a cycle.
    pub fn resolve(&self, name: &str) -> Result<Map<String, Value>, String> {
        let mut chain = Vec::new();
        let mut current = Some(name.to_owned());
        while let Some(n) = current {
            if chain.iter().any(|(seen, _): &(String, &Map<String, Value>)| *seen == n) {
                return Err(format!("inherits cycle at {n}"));
            }
            let (_, map) = self.by_name.get(&n).ok_or_else(|| format!("preset not found: {n}"))?;
            current = map.get("inherits").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
            chain.push((n, map));
        }
        let mut merged = Map::new();
        for (_, map) in chain.into_iter().rev() {
            for (k, v) in map {
                merged.insert(k.clone(), v.clone());
            }
        }
        Ok(merged)
    }
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
        let box_preset = fixture().resolve("Acme Box 0.4 nozzle").unwrap();
        assert_eq!(box_preset.get("printer_model").and_then(|v| v.as_str()), Some("Acme Box"));
        assert_eq!(box_preset.get("printable_height").and_then(|v| v.as_str()), Some("250"));
        assert_eq!(rect_from_points(&parse_points(&box_preset["printable_area"]).unwrap()), Some([0.0, 0.0, 220.0, 220.0]));
    }

    #[test]
    fn vendor_comes_from_the_folder_and_only_instantiable_machines_are_listed() {
        let index = fixture();
        let machines: Vec<(String, String)> = index.machines().map(|m| (m.vendor, m.name)).collect();
        let expected = [("Acme", "Acme Box 0.4 nozzle"), ("Acme", "Acme Ring 0.4 nozzle")].map(|(v, n)| (v.to_owned(), n.to_owned()));
        assert_eq!(machines, expected);
    }

    #[test]
    fn comma_string_points_and_degenerate_exclusions() {
        let ring = fixture().resolve("Acme Ring 0.4 nozzle").unwrap();
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
        assert!(index.resolve("a").is_err());
    }
}

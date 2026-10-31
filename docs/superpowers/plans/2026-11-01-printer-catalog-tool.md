# Printer catalog tool (OrcaSlicer import + slicing qualification) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-11-01-printer-catalog-and-stl-design.md` (read «Licensing», «Catalog profile fields», «Qualification»).

**Goal:** A developer tool `printer_catalog` that turns OrcaSlicer's JSON machine presets (pinned OrcaSlicer checkout) into our `profiles/*.json` for rectangular-bed printers, and qualifies each one by slicing in the runner image: build a probe 3MF with Orca CLI, detect the start-G-code purge line from our own probe G-code, run `colored_3mf_packer` + `verify_outputs` on a dense plate. End users never run it.

**Architecture:** Pure, unit-tested library modules: `orca_presets` (index + `inherits` merge + Orca point-list parsing), `purge_detect` (extrusions before the first layer → axis-aligned purge keepouts), `bed_outline` (bed STL top face → our own simplified polygon aligned to the printable area). A binary `src/bin/printer_catalog.rs` with `extract` (pure, testable on a fake preset tree) and `qualify` (calls the slicer and our binaries; not unit-tested — repo rule: tests never call a slicer). A shell wrapper runs `qualify` inside the runner image. Tower behaviour is not qualified: the packer already measures the tower per job from the probe slice of the user's own project.

**Tech Stack:** Rust 2024 (`serde_json`, `geo 0.33`, existing STL reading), OrcaSlicer 2.4.2 JSON presets, OrcaSlicer CLI (runner image), bash, Docker.

**Licensing (binding):** never copy Orca files (JSON, G-code, STL/SVG) into the repo or the release; only numbers and our own traced polygon go into `profiles/*.json`, each with a `source` block.

**Repo rules (CLAUDE.md):** `cargo test --release`; tests self-contained (temp data or `tests/fixtures/`), never call a slicer or the network; profiles are the only source of printer geometry; `SLICER_KEEPOUT_SAFETY_MM` is empirical — do not lower it; every slicer call goes through `slicer_cli::slice_command` (`--arrange 0`).

---

## File structure

| File | Responsibility |
|---|---|
| `src/orca_presets.rs` (new) + `pub mod` in `src/lib.rs` | load preset JSONs, resolve `inherits`, parse `printable_area` / `bed_exclude_area` |
| `src/purge_detect.rs` (new) | purge segments before the first layer → `[x0,y0,x1,y1,width]` |
| `src/bed_outline.rs` (new) | bed STL → simplified outline aligned to the printable area |
| `src/bin/printer_catalog.rs` (new) | `extract` and `qualify` subcommands |
| `tests/fixtures/catalog/` (new) | tiny hand-written preset tree and G-code snippets for tests (our own content) |
| `deploy/catalog-qualify.sh` (new) | run `qualify` for a list of drafts inside the runner image |
| `docs/catalog/README.md`, `docs/catalog/report-<date>.md` (new) | how to run; qualification results |
| `profiles/*.json` (generated) | qualified catalog profiles |
| `CLAUDE.md` | catalog invariants |

---

### Task 1: Spike — Orca CLI builds a probe 3MF from an STL in the runner image

No production code. Outcome is written to the spec as an appendix; later tasks rely on it.

- [ ] **Step 1: Build/locate the runner image** — see `deploy/README.md` and `deploy/build-images.sh` (images `goat-packer-*:<tag>`; slicer AppImages unpacked to `/opt/slicers/orca/AppRun`, `/opt/slicers/bambu/AppRun`). Run a shell: `docker run --rm -it --entrypoint bash -v "$PWD:/repo" -v /Users/petrpopov/Developer/projects/main/OrcaSlicer:/orca:ro <runner-image>`.

- [ ] **Step 2: Resolve one preset by hand** (Prusa MK4S 0.4 or Creality Ender-3 V3 SE): merge its `inherits` chain into one machine JSON with `"from": "system"`, `"instantiation": "true"`; same for a compatible «0.20mm Standard» process and «Generic PLA @System» filament. Write them to `/tmp/spike/{machine,process,filament}.json`.

- [ ] **Step 3: Export a probe 3MF**

```bash
/opt/slicers/orca/AppRun --load-settings "/tmp/spike/machine.json;/tmp/spike/process.json" \
  --load-filaments /tmp/spike/filament.json --arrange 1 --export-3mf /tmp/spike/probe.3mf \
  --outputdir /tmp/spike /repo/tests/fixtures/square.stl
```
Expected: `/tmp/spike/probe.3mf` exists and contains `Metadata/project_settings.config` with the machine's `printer_model` and `printable_area`. If export fails, try `--allow-newer-file`, `--datadir /tmp/spike/data`, or `--load-defaultfila`; record what works.

- [ ] **Step 4: Slice it the way the packer does** — `/opt/slicers/orca/AppRun --arrange 0 --slice 1 --outputdir /tmp/spike/out /tmp/spike/probe.3mf` (this is `slicer_cli::slice_command`). Expected: a `.gcode` (or `.gcode.3mf` containing `Metadata/plate_1.gcode`).

- [ ] **Step 5: Run our packer on it** — `/repo/target/release/colored_3mf_packer --help` for flags; run it on `probe.3mf` with a hand-made profile JSON for that printer (copy `profiles/snapmaker_u1.json`, change id/name/area/slicer_path). Expected: a packed 3MF and `verify_outputs` OK.

- [ ] **Step 6: Record** in the spec, section «Appendix: CLI spike», the exact working commands, required flags, the first-layer marker string in the probe G-code (`;LAYER_CHANGE` / `; CHANGE_LAYER` / other), and failures. Commit:

```bash
git add docs/superpowers/specs/2026-11-01-printer-catalog-and-stl-design.md
git commit -m "Spec: Orca CLI spike results for STL probes"
```

If Step 3 or 5 cannot be made to work, STOP and report: the plan's Task 6 depends on it.

---

### Task 2: `orca_presets` — index, inherits, point lists (TDD)

**Files:** Create `src/orca_presets.rs`; add `pub mod orca_presets;` to `src/lib.rs`; fixtures under `tests/fixtures/catalog/presets/Acme/machine/`.

- [ ] **Step 1: Fixtures (our own content)**

`tests/fixtures/catalog/presets/Acme/machine/fdm_acme_common.json`:
```json
{ "type": "machine", "name": "fdm_acme_common", "from": "system", "instantiation": "false",
  "printable_area": ["0x0", "220x0", "220x220", "0x220"], "printable_height": "250", "nozzle_diameter": ["0.4"] }
```
`tests/fixtures/catalog/presets/Acme/machine/Acme Box 0.4 nozzle.json`:
```json
{ "type": "machine", "name": "Acme Box 0.4 nozzle", "inherits": "fdm_acme_common", "from": "system",
  "instantiation": "true", "printer_model": "Acme Box", "bed_exclude_area": ["0x0", "18x0", "18x28", "0x28"] }
```
`tests/fixtures/catalog/presets/Acme/machine/Acme Ring 0.4 nozzle.json` (string form, as some Creality presets use):
```json
{ "type": "machine", "name": "Acme Ring 0.4 nozzle", "inherits": "fdm_acme_common", "from": "system",
  "instantiation": "true", "printer_model": "Acme Ring", "printable_area": "0x0,300x0,300x300,0x300", "bed_exclude_area": "0x0" }
```

- [ ] **Step 2: Failing tests** (in `src/orca_presets.rs`, `#[cfg(test)] mod tests`):

```rust
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
        let machines: Vec<_> = index.machines().map(|m| (m.vendor.as_str(), m.name.as_str())).collect();
        assert_eq!(machines, vec![("Acme", "Acme Box 0.4 nozzle"), ("Acme", "Acme Ring 0.4 nozzle")]);
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
```

Run `cargo test --release orca_presets` → FAIL (module missing).

- [ ] **Step 3: Implement `src/orca_presets.rs`**

```rust
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
```

- [ ] **Step 4: Run** — `cargo test --release orca_presets` → PASS (6).
- [ ] **Step 5: Commit** — `git add src/orca_presets.rs src/lib.rs tests/fixtures/catalog && git commit -m "orca_presets: resolve Orca machine presets and their bed geometry"`

---

### Task 3: `purge_detect` — purge line from probe G-code (TDD)

**Files:** Create `src/purge_detect.rs`; `pub mod purge_detect;` in `src/lib.rs`.

Rules: only moves **before the first layer marker** (`;LAYER_CHANGE`, `; CHANGE_LAYER`, `;LAYER:0` — extend with what Task 1 recorded); track `G90/G91`, `M82/M83`, `G92 E`; an extruding move is a G0/G1 with positive E delta and XY motion; width from volume: `w = E · π·(1.75/2)² / (len · z)`, clamped to 0.3–3 mm, `z` = current Z (default 0.2); keep segments that intersect the printable area (clipped); diagonal segments become their bounding strip; result `[x0, y0, x1, y1, width]` with axis-aligned centre lines as the packer expects (`axis_aligned_line_keepout`). The packer adds its own safety.

- [ ] **Step 1: Failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const P1S_LIKE: &str = "G90\nM83\nG1 X18.0 Y1.0 Z0.8 F18000\nG1 Z0.2\nG0 X240 E15 F6033\nG0 Y11 E0.700\nG0 X239.5\nG0 E0.2\nG0 Y1.5 E0.700\nG0 X18 E15\n;LAYER_CHANGE\n;Z:0.2\nG1 X100 Y100 E5\n";

    #[test]
    fn segments_before_the_first_layer_are_the_purge() {
        let lines = purge_lines(P1S_LIKE, [0.0, 0.0, 256.0, 256.0]);
        let rounded: Vec<[f64; 4]> = lines.iter().map(|l| [l[0], l[1], l[2], l[3]].map(|v| (v * 10.0).round() / 10.0)).collect();
        assert_eq!(rounded, vec![[18.0, 1.0, 240.0, 1.0], [240.0, 1.0, 240.0, 11.0], [239.5, 11.0, 239.5, 1.5], [239.5, 1.5, 18.0, 1.5]]);
        assert!(lines.iter().all(|l| l[4] >= 0.3 && l[4] <= 3.0));
    }

    #[test]
    fn off_bed_purge_is_ignored_and_relative_moves_are_tracked() {
        // Prusa-style: purge at Y-4 (off the printable area), relative XY moves via G91.
        let gcode = "G90\nM83\nG0 X0 Y-4 Z0.2\nG91\nG1 X15 E7\nG1 X10 E4\nG90\n; CHANGE_LAYER\n";
        assert!(purge_lines(gcode, [0.0, 0.0, 250.0, 210.0]).is_empty());
    }

    #[test]
    fn absolute_extrusion_and_g92_reset() {
        let gcode = "G90\nM82\nG92 E0\nG1 X10 Y5 Z0.2\nG1 X60 Y5 E2.5\nG92 E0\nG1 X60 Y15 E0.5\n;LAYER:0\n";
        let lines = purge_lines(gcode, [0.0, 0.0, 200.0, 200.0]);
        assert_eq!(lines.len(), 2);
        assert_eq!([lines[1][0], lines[1][1], lines[1][2], lines[1][3]], [60.0, 5.0, 60.0, 15.0]);
    }

    #[test]
    fn diagonal_segment_becomes_its_bounding_strip() {
        let gcode = "G90\nM83\nG1 X10 Y10 Z0.3\nG1 X20 Y12 E1\n;LAYER_CHANGE\n";
        let line = purge_lines(gcode, [0.0, 0.0, 200.0, 200.0])[0];
        // Wider of the two spans is the centre line; the narrow span becomes extra width.
        assert_eq!([line[0], line[1], line[2], line[3]], [10.0, 11.0, 20.0, 11.0]);
        assert!(line[4] >= 2.0);
    }
}
```

Run `cargo test --release purge_detect` → FAIL.

- [ ] **Step 2: Implement `src/purge_detect.rs`**

```rust
//! Start-G-code purge («prime/load line») from a probe slice the catalog tool produced itself:
//! extruding moves before the first layer marker, as the axis-aligned `[x0,y0,x1,y1,width]`
//! strips `PrinterProfile::purge_lines` expects. Off-bed purges (Prusa, Creality) are dropped —
//! they never collide with placements.

const LAYER_MARKERS: [&str; 3] = [";LAYER_CHANGE", "; CHANGE_LAYER", ";LAYER:0"];
const FILAMENT_AREA_MM2: f64 = std::f64::consts::PI * 0.875 * 0.875; // 1.75 mm filament

fn word(line: &str, letter: char) -> Option<f64> {
    line.split_whitespace().find_map(|w| w.strip_prefix(letter)?.parse().ok())
}

pub fn purge_lines(gcode: &str, printable: [f64; 4]) -> Vec<[f64; 5]> {
    let (mut x, mut y, mut z, mut e) = (0.0_f64, 0.0_f64, 0.2_f64, 0.0_f64);
    let (mut absolute_xyz, mut absolute_e) = (true, true);
    let mut out = Vec::new();
    for raw in gcode.lines() {
        if LAYER_MARKERS.iter().any(|m| raw.starts_with(m)) {
            break;
        }
        let line = raw.split(';').next().unwrap_or("").trim();
        let cmd = line.split_whitespace().next().unwrap_or("");
        match cmd {
            "G90" => absolute_xyz = true,
            "G91" => absolute_xyz = false,
            "M82" => absolute_e = true,
            "M83" => absolute_e = false,
            "G92" => {
                if let Some(v) = word(line, 'E') {
                    e = v;
                }
            }
            "G0" | "G1" => {
                let step = |cur: f64, v: Option<f64>| match v { Some(v) if absolute_xyz => v, Some(v) => cur + v, None => cur };
                let (nx, ny) = (step(x, word(line, 'X')), step(y, word(line, 'Y')));
                z = step(z, word(line, 'Z'));
                let de = match word(line, 'E') {
                    Some(v) if absolute_e => { let d = v - e; e = v; d }
                    Some(v) => v,
                    None => 0.0,
                };
                let len = ((nx - x).powi(2) + (ny - y).powi(2)).sqrt();
                if de > 0.0 && len > 0.5 {
                    if let Some(strip) = strip(x, y, nx, ny, de, len, z.max(0.05), printable) {
                        out.push(strip);
                    }
                }
                x = nx;
                y = ny;
            }
            _ => {}
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn strip(x0: f64, y0: f64, x1: f64, y1: f64, de: f64, len: f64, z: f64, printable: [f64; 4]) -> Option<[f64; 5]> {
    let [ax0, ay0, ax1, ay1] = printable;
    // Fully outside the printable area → cannot collide with a placement.
    if x0.max(x1) < ax0 || x0.min(x1) > ax1 || y0.max(y1) < ay0 || y0.min(y1) > ay1 {
        return None;
    }
    let width = (de * FILAMENT_AREA_MM2 / (len * z)).clamp(0.3, 3.0);
    let (cx0, cx1, cy0, cy1) = (x0.clamp(ax0, ax1), x1.clamp(ax0, ax1), y0.clamp(ay0, ay1), y1.clamp(ay0, ay1));
    let (dx, dy) = ((cx1 - cx0).abs(), (cy1 - cy0).abs());
    if dx < 1e-6 || dy < 1e-6 {
        return Some([cx0, cy0, cx1, cy1, width]);
    }
    // Diagonal: centre line along the longer span, the shorter span widens the strip.
    if dx >= dy {
        let ym = (cy0 + cy1) / 2.0;
        Some([cx0.min(cx1), ym, cx0.max(cx1), ym, width + dy])
    } else {
        let xm = (cx0 + cx1) / 2.0;
        Some([xm, cy0.min(cy1), xm, cy0.max(cy1), width + dx])
    }
}
```

- [ ] **Step 3: Run** — `cargo test --release purge_detect` → PASS. If the first test's P1S-like widths fall outside 0.3–3.0 because `Z0.8` precedes `Z0.2`, keep the clamp; do not special-case.
- [ ] **Step 4: Commit** — `git add src/purge_detect.rs src/lib.rs && git commit -m "purge_detect: start-G-code purge strips from probe G-code"`

---

### Task 4: `bed_outline` — trace a bed STL into our polygon (TDD)

**Files:** Create `src/bed_outline.rs`; `pub mod bed_outline;` in `src/lib.rs`.

Algorithm (validated on the P1S/A2L bed models during design): take triangles whose three vertices are at the max Z (the plate's top face), keep edges used by exactly one such triangle, chain them into loops, take the loop with the largest area, simplify with `geo::Simplify` (ε = 0.5 mm), round to 0.1 mm. Alignment: if the outline's bbox contains the printable area, keep coordinates; otherwise translate so its bbox centre equals the printable area centre (some vendors model the bed centred at the origin). Output ≤ 60 points, closed ring without the duplicate last point.

- [ ] **Step 1: Failing tests** — generate the STL in the test (two-triangle top face + a box), no files:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Binary STL of an axis-aligned slab [x0,x1]×[y0,y1]×[-1,0] (12 triangles).
    fn slab(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<u8> {
        let p = |x: f32, y: f32, z: f32| [x, y, z];
        let (a, b, c, d) = (p(x0, y0, 0.0), p(x1, y0, 0.0), p(x1, y1, 0.0), p(x0, y1, 0.0));
        let (e, f, g, h) = (p(x0, y0, -1.0), p(x1, y0, -1.0), p(x1, y1, -1.0), p(x0, y1, -1.0));
        let tris = [[a, b, c], [a, c, d], [e, g, f], [e, h, g], [a, e, f], [a, f, b], [b, f, g], [b, g, c], [c, g, h], [c, h, d], [d, h, e], [d, e, a]];
        let mut out = vec![0u8; 80];
        out.extend((tris.len() as u32).to_le_bytes());
        for t in tris {
            out.extend([0u8; 12]);
            for v in t { for c in v { out.extend(c.to_le_bytes()); } }
            out.extend([0u8; 2]);
        }
        out
    }

    #[test]
    fn slab_top_face_is_the_outline() {
        let outline = outline_from_stl(&slab(-2.0, -5.0, 258.0, 260.0), [0.0, 0.0, 256.0, 256.0]).unwrap();
        let xs: Vec<f64> = outline.iter().map(|p| p[0]).collect();
        let ys: Vec<f64> = outline.iter().map(|p| p[1]).collect();
        assert_eq!(outline.len(), 4);
        assert_eq!(xs.iter().cloned().fold(f64::INFINITY, f64::min), -2.0);
        assert_eq!(ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max), 260.0);
    }

    #[test]
    fn centred_bed_model_is_moved_onto_the_printable_area() {
        let outline = outline_from_stl(&slab(-140.0, -146.5, 140.0, 146.5), [0.5, 1.0, 270.5, 271.0]).unwrap();
        let cx = (outline.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min) + outline.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max)) / 2.0;
        assert!((cx - 135.5).abs() < 0.11);
    }

    #[test]
    fn garbage_is_none() {
        assert!(outline_from_stl(b"not an stl", [0.0, 0.0, 100.0, 100.0]).is_none());
    }
}
```

Run `cargo test --release bed_outline` → FAIL.

- [ ] **Step 2: Implement `src/bed_outline.rs`**

```rust
//! Our own simplified plate outline traced from a slicer bed model (top face boundary). The
//! model file is read at catalog-build time and never stored or shipped (AGPL); only the
//! resulting polygon goes into `profiles/*.json` as `plate_outline`.
use std::collections::HashMap;

use geo::{Area, Coord, LineString, Polygon, Simplify};

type Key = (i64, i64);

fn triangles(stl: &[u8]) -> Option<Vec<[[f64; 3]; 3]>> {
    if stl.len() >= 84 {
        let n = u32::from_le_bytes(stl[80..84].try_into().ok()?) as usize;
        if 84 + n * 50 == stl.len() {
            let f = |o: usize| f32::from_le_bytes(stl[o..o + 4].try_into().unwrap()) as f64;
            return Some((0..n).map(|i| { let b = 84 + i * 50 + 12; [0, 1, 2].map(|v| [f(b + v * 12), f(b + v * 12 + 4), f(b + v * 12 + 8)]) }).collect());
        }
    }
    let text = std::str::from_utf8(stl).ok()?;
    let verts: Vec<[f64; 3]> = text
        .lines()
        .filter_map(|l| {
            let mut it = l.trim().strip_prefix("vertex")?.split_whitespace().map(|s| s.parse::<f64>().ok());
            Some([it.next()??, it.next()??, it.next()??])
        })
        .collect();
    (!verts.is_empty() && verts.len() % 3 == 0).then(|| verts.chunks(3).map(|c| [c[0], c[1], c[2]]).collect())
}

pub fn outline_from_stl(stl: &[u8], printable: [f64; 4]) -> Option<Vec<[f64; 2]>> {
    let tris = triangles(stl)?;
    let z_top = tris.iter().flatten().map(|v| v[2]).fold(f64::NEG_INFINITY, f64::max);
    let key = |v: &[f64; 3]| -> Key { ((v[0] * 100.0).round() as i64, (v[1] * 100.0).round() as i64) };
    let mut edges: HashMap<(Key, Key), u32> = HashMap::new();
    for t in tris.iter().filter(|t| t.iter().all(|v| (v[2] - z_top).abs() < 1e-3)) {
        for i in 0..3 {
            let (a, b) = (key(&t[i]), key(&t[(i + 1) % 3]));
            *edges.entry(if a < b { (a, b) } else { (b, a) }).or_default() += 1;
        }
    }
    let mut adjacency: HashMap<Key, Vec<Key>> = HashMap::new();
    for ((a, b), count) in edges {
        if count == 1 {
            adjacency.entry(a).or_default().push(b);
            adjacency.entry(b).or_default().push(a);
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut best: Option<(f64, Vec<Key>)> = None;
    for &start in adjacency.keys() {
        if seen.contains(&start) {
            continue;
        }
        let mut ring = vec![start];
        seen.insert(start);
        let mut current = start;
        while let Some(&next) = adjacency.get(&current).and_then(|n| n.iter().find(|k| !seen.contains(*k))) {
            seen.insert(next);
            ring.push(next);
            current = next;
        }
        if ring.len() < 3 {
            continue;
        }
        let poly = Polygon::new(LineString::from(ring.iter().map(|k| Coord { x: k.0 as f64 / 100.0, y: k.1 as f64 / 100.0 }).collect::<Vec<_>>()), vec![]);
        let area = poly.unsigned_area();
        if best.as_ref().is_none_or(|(a, _)| area > *a) {
            best = Some((area, ring));
        }
    }
    let (_, ring) = best?;
    let mut line = LineString::from(ring.iter().map(|k| Coord { x: k.0 as f64 / 100.0, y: k.1 as f64 / 100.0 }).collect::<Vec<_>>());
    line.close();
    let simple = line.simplify(&0.5);
    let mut points: Vec<[f64; 2]> = simple.0.iter().map(|c| [c.x, c.y]).collect();
    points.pop(); // drop the closing duplicate
    let (min_x, max_x) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p[0]), b.max(p[0])));
    let (min_y, max_y) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p[1]), b.max(p[1])));
    let [ax0, ay0, ax1, ay1] = printable;
    let contains = min_x <= ax0 + 1e-6 && min_y <= ay0 + 1e-6 && max_x >= ax1 - 1e-6 && max_y >= ay1 - 1e-6;
    let (dx, dy) = if contains { (0.0, 0.0) } else { ((ax0 + ax1) / 2.0 - (min_x + max_x) / 2.0, (ay0 + ay1) / 2.0 - (min_y + max_y) / 2.0) };
    let round = |v: f64| (v * 10.0).round() / 10.0;
    (points.len() >= 3 && points.len() <= 60).then(|| points.iter().map(|p| [round(p[0] + dx), round(p[1] + dy)]).collect())
}
```

If `line.simplify(&0.5)` does not compile with this `geo` version, call `line.simplify(0.5)`; if `Option::is_none_or` is unavailable, use `match`.

- [ ] **Step 3: Run** — `cargo test --release bed_outline` → PASS.
- [ ] **Step 4: Commit** — `git add src/bed_outline.rs src/lib.rs && git commit -m "bed_outline: trace bed models into our own plate outline"`

---

### Task 5: `printer_catalog extract` — draft profiles (TDD on the fixture tree)

**Files:** Create `src/bin/printer_catalog.rs`; tests in it (`#[cfg(test)]`) using `tests/fixtures/catalog/presets`.

Draft profile = existing `PrinterProfile` keys + `vendor`, `printer_model`, `orca_machine`, `printable_height`, `source`. Defaults: `slicer_kind: "orca"`, `slicer_path: "/opt/slicers/orca/AppRun"`, `blank_start_gcode_for_cli: false`, `no_check_for_final_tower_probe: false`, `edge_margin_mm: 0.0`, `purge_lines: []` (filled by `qualify`), `tower_marker_style: "wipe_tower"`, `tower_line_width_mm: 0.5`, `source: {"orca_tag": <arg>, "imported": <date>, "qualified": false}`. `plate_outline` from `bed_outline` when the machine model JSON names a `bed_model` STL that exists in the vendor folder; otherwise omitted (the webapp then draws the printable rectangle). Skip: non-rectangular areas (report `skipped: non-rectangular bed`), existing `profiles/*.json` without `source` or with `source.manual == true` (never overwrite hand-made profiles: P1S, A2L, U1). Id = lowercase slug of the machine preset name (`acme_box_0_4_nozzle`).

- [ ] **Step 1: Failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_builds_drafts_for_rectangular_machines() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog/presets");
        let index = goat_packer::orca_presets::PresetIndex::load(&root).unwrap();
        let drafts = drafts(&index, &root, "v2.4.2", "2026-11-01", &[]);
        let ids: Vec<_> = drafts.iter().map(|d| d["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["acme_box_0_4_nozzle", "acme_ring_0_4_nozzle"]);
        let first = &drafts[0];
        assert_eq!(first["vendor"], "Acme");
        assert_eq!(first["printer_model"], "Acme Box");
        assert_eq!(first["printable_area"], serde_json::json!([0.0, 0.0, 220.0, 220.0]));
        assert_eq!(first["exclusion_rectangles"], serde_json::json!([[0.0, 0.0, 18.0, 28.0]]));
        assert_eq!(first["source"]["qualified"], false);
    }

    #[test]
    fn hand_made_profiles_are_never_overwritten() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog/presets");
        let index = goat_packer::orca_presets::PresetIndex::load(&root).unwrap();
        let drafts = drafts(&index, &root, "v2.4.2", "2026-11-01", &["acme_box_0_4_nozzle".to_owned()]);
        assert_eq!(drafts.len(), 1);
    }
}
```

(Use the crate's actual lib name from `Cargo.toml` instead of `goat_packer` if it differs.)

- [ ] **Step 2: Implement** `fn drafts(index, vendors_root, orca_tag, date, protected_ids: &[String]) -> Vec<serde_json::Value>` and `fn main()` with the repo's hand-rolled arg parsing (`while let Some(option)` shape, see other bins):
  - `printer_catalog extract --orca <checkout>/resources/profiles --tag v2.4.2 [--vendor Name]... --out <dir>`: loads the index, computes `protected_ids` = ids of `profiles/*.json` lacking `source` or with `source.manual`, writes one `<id>.json` per draft into `--out` (pretty JSON, keys in a stable order) and prints a summary `N drafts, M skipped (reason counts)`.
  - Field mapping: `printable_area` → `rect_from_points(parse_points(..))`; `exclusion_rectangles` → `exclusion_rects`; `printable_height` → number; `orca_machine` → preset name; `printer_name` → `printer_model` value (fallback preset name); `vendor` → folder.
- [ ] **Step 3: Run** — `cargo test --release --bin printer_catalog` → PASS.
- [ ] **Step 4: Commit** — `git add src/bin/printer_catalog.rs && git commit -m "printer_catalog: extract draft profiles from Orca presets"`

---

### Task 6: `printer_catalog qualify` + runner wrapper

**Files:** Modify `src/bin/printer_catalog.rs`; create `deploy/catalog-qualify.sh`, `docs/catalog/README.md`.

`qualify --draft <id.json> --orca <checkout>/resources/profiles --slicer /opt/slicers/orca/AppRun --packer <path>/colored_3mf_packer --verify <path>/verify_outputs --work <dir> --out profiles/` — uses the commands recorded in Task 1's spec appendix:

1. Resolve machine (`orca_machine`), a process (prefer name containing `0.20mm Standard` whose `compatible_printers` lists the machine; else the first compatible; else Orca's base process) and filament `Generic PLA @System`; write the three merged JSONs into `<work>` with `"from": "system"` and `"instantiation": "true"` (Task 1 tells if more keys are required).
2. Export `probe.3mf` from `tests/fixtures/square.stl` (Task 1 command).
3. Slice with `slicer_cli::slice_command` (reuse the library function, `--arrange 0`); read the G-code (plain or inside `.gcode.3mf`).
4. `purge_detect::purge_lines(gcode, printable_area)` → `purge_lines`.
5. Run `colored_3mf_packer` on `probe.3mf` with the draft (now with purge lines) as `--profile`, default clearance, into `<work>/packed.3mf`; then `verify_outputs`. If the final slice fails with the known CLI issues, retry once with `no_check_for_final_tower_probe: true`, then once with `blank_start_gcode_for_cli: true`; record which flag made it pass.
6. On success: write `profiles/<id>.json` with `source.qualified: true`, `source.qualified_on: <date>`, `source.notes` (flags used, purge count). On failure: keep the draft out of `profiles/`, print the reason. Always append a CSV/markdown row to `<work>/report.md` (vendor, model, result, reason, seconds).

Unit-test only the pure helpers you add (process selection from a fixture index: add `tests/fixtures/catalog/presets/Acme/process/0.20mm Standard @Acme.json` with `"compatible_printers": ["Acme Box 0.4 nozzle"]` and a second process not listing it; assert the right one is chosen).

`deploy/catalog-qualify.sh`: runs `docker run --rm -v "$PWD:/repo" -v "$ORCA_CHECKOUT:/orca:ro" <runner-image> /repo/target/x86_64-unknown-linux-gnu/release/printer_catalog qualify …` for every draft in a directory, N at a time (`xargs -P`), and collects reports. Follow the style of `deploy/build-images.sh` (bash, `set -euo pipefail`, Russian comments).

`docs/catalog/README.md`: when to run (Orca version bump), commands (`extract`, `catalog-qualify.sh`), licensing rule, how hand-made profiles are protected.

- [ ] **Step 1:** failing test for process selection → implement → PASS.
- [ ] **Step 2:** implement `qualify` and the wrapper; `cargo build --release && cargo test --release --all` → PASS.
- [ ] **Step 3: Commit** — `git add src/bin/printer_catalog.rs deploy/catalog-qualify.sh docs/catalog tests/fixtures/catalog && git commit -m "printer_catalog: qualify drafts by slicing in the runner image"`

---

### Task 7: First batch and report

- [ ] **Step 1:** `printer_catalog extract --orca /Users/petrpopov/Developer/projects/main/OrcaSlicer/resources/profiles --tag v2.4.2 --vendor BBL --vendor Prusa --vendor Creality --vendor Elegoo --vendor Anycubic --vendor Qidi --vendor Sovol --vendor Flashforge --vendor Snapmaker --vendor Voron --out /tmp/catalog-drafts`.
- [ ] **Step 2:** `deploy/catalog-qualify.sh /tmp/catalog-drafts` (on a machine with the runner image).
- [ ] **Step 3:** Copy `report.md` to `docs/catalog/report-<date>.md`; review failures by reason; do not hand-edit generated profiles (fix the tool and re-run).
- [ ] **Step 4: Commit** — `git add profiles docs/catalog && git commit -m "Catalog: first batch of qualified printer profiles (Orca 2.4.2)"`
- [ ] **Step 5:** Repeat for the remaining vendors in batches; commit each batch.

---

### Task 8: CLAUDE.md

- [ ] Add under «Printer profiles / slicer quirks»: «Catalog: `printer_catalog extract|qualify` builds `profiles/*.json` from OrcaSlicer presets (numbers only, AGPL files never copied; `plate_outline` is our own trace). A profile with `source.qualified` was sliced end-to-end in the runner image; hand-made profiles (no `source` or `source.manual`) are never overwritten. Purge lines come from `purge_detect` on our probe G-code.» Commit.

---

## Self-review
- Spec «Catalog profile fields» → Tasks 2, 4, 5 (+ purge in 3, qualification flags in 6); «Qualification» → Tasks 1, 6, 7; «Licensing» → module docs + Tasks 4, 5, 8.
- Tower qualification intentionally omitted: the packer measures the tower per job from the user's own probe slice (`parse_prime_tower_bounds`), so a catalog value would only be a fallback.
- Web/backend parts of the spec (STL upload, combobox, «нет в списке», unknown-3MF fallback, `vendor` in the API) are a separate plan: `2026-11-01-stl-upload-and-printer-picker.md`.

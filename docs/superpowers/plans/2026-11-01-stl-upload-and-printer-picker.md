# STL uploads, printer picker and «нет в списке» — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-11-01-printer-catalog-and-stl-design.md` — sections «Upload scenarios», «Web UI», and «Appendix: CLI spike» (working Orca CLI commands; read it first).

**Prerequisites:** branch `printer-catalog` with plan `2026-11-01-printer-catalog-tool.md` Tasks 2–6 merged (vendor-scoped `orca_presets`, flatten + process selection used by `qualify`), and the webapp redesign (v0.2.0).

**Goal:** A user uploads a 3MF **or an STL**, picks the printer in a searchable combobox grouped by manufacturer (or chooses «Моего принтера нет в списке» and types the bed size), sees that bed in the scene immediately, and packs. STL → 3MF conversion runs on the server inside the job, with the presets of the slicer installed in the runner image (`/opt/slicer/resources/profiles`, plain JSON) — nothing from Orca is copied into the repo.

**Architecture:**
- Rust: move the flatten/select logic used by `printer_catalog qualify` into a library module `orca_project` and add a binary `source_project` (STL + machine preset name or custom bed → `input.3mf` via the Orca CLI export command from the spike). The job runner calls it before `colored_3mf_packer` when the input is an STL.
- Catalog: one profile per printer model (`printer_model`), preferring the 0.4 mm nozzle preset; geometry is identical across nozzles and 3MF detection matches `printer_model` → no ambiguity.
- Backend: profiles API gains `vendor`; uploads accept `.stl`; jobs carry `profileId` chosen by the user or `customBed`; for unknown-printer 3MF the detect endpoint returns the bed read from the project; the worker writes an ad-hoc `profile.json` for custom beds.
- Frontend: `PrinterPicker` combobox (pure grouping/filtering in `utils/printers.ts`, tested), custom-bed fields, scene from the picked/custom profile before upload, STL accepted by the file drop.

**Tech Stack:** Rust 2024; Kotlin/Spring; React/TS; OrcaSlicer CLI 2.4.2 (runner image).

---

## File structure

| File | Responsibility |
|---|---|
| `src/orca_project.rs` (new, moved from `printer_catalog`) | flatten machine/process/filament presets for the CLI; pick process/filament; custom-bed machine |
| `src/bin/source_project.rs` (new) | STL → `input.3mf` (catalog machine or custom bed) |
| `src/bin/printer_catalog.rs` | use `orca_project`; one draft per `printer_model` (0.4 nozzle preferred) |
| `deploy/build-images.sh` / Dockerfiles | ship `source_project` in runner images |
| `webapp/backend/.../ProfileService.kt`, `JobDtos.kt` | `vendor` in `ProfileDto`; detection result with project bed |
| `webapp/backend/.../server/Storage.kt`, local upload controller | accept `.stl` |
| `webapp/backend/.../server/Queue.kt` `JobRequest`, local `PackParams` | `customBed`; STL inputs |
| `webapp/backend/.../job/JobRunner.kt`, `server/QueueWorker.kt`, `server/NativeLauncher.kt` | conversion step; ad-hoc profile for custom beds |
| `webapp/frontend/src/utils/printers.ts` + `tests/printers.test.ts` | group/sort/filter profiles |
| `webapp/frontend/src/components/PrinterPicker.tsx` | combobox |
| `webapp/frontend/src/pages/PackPage.tsx`, `components/FileDrop.tsx` | STL, picker, custom bed, scene before upload |

---

### Task 1: Catalog — one profile per printer model

**Files:** `src/bin/printer_catalog.rs` (+ tests).

- [ ] **Step 1: Failing test** (fixtures: add `tests/fixtures/catalog/presets/Acme/machine/Acme Box 0.6 nozzle.json` inheriting `Acme Box 0.4 nozzle` with `"nozzle_diameter": ["0.6"]`, `"instantiation": "true"`):

```rust
    #[test]
    fn one_draft_per_printer_model_prefers_the_0_4_nozzle() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog/presets");
        let index = goat_packer::orca_presets::PresetIndex::load(&root).unwrap();
        let drafts = drafts(&index, &root, "v2.4.2", "2026-11-01", &[]);
        let boxes: Vec<_> = drafts.iter().filter(|d| d["printer_model"] == "Acme Box").collect();
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0]["orca_machine"], "Acme Box 0.4 nozzle");
        assert_eq!(boxes[0]["id"], "acme_box");
    }
```
- [ ] **Step 2:** implement: group machines by `(vendor, printer_model)`; pick the preset whose `nozzle_diameter[0] == 0.4`, else the smallest nozzle; id = slug of `printer_model` (e.g. `prusa_mk4s`); keep `orca_machine` = chosen preset name. Update the earlier extract tests' expected ids accordingly.
- [ ] **Step 3:** `cargo test --release --bin printer_catalog` → PASS. Commit «printer_catalog: one profile per printer model».

---

### Task 2: `orca_project` library module (move + custom bed)

**Files:** create `src/orca_project.rs` (`pub mod` in `src/lib.rs`); modify `src/bin/printer_catalog.rs` to call it.

- [ ] **Step 1:** Move from `printer_catalog.rs` into `orca_project` (no behaviour change, tests move with the code): flattening a preset for the CLI (merge chain, drop `inherits`, `"from": "system"`, `"instantiation": "true"`), process selection with explicit `compatible_printers = [machine]`, filament `Generic PLA @System`. Public API:

```rust
pub struct CliPresets { pub machine: serde_json::Map<String, Value>, pub process: serde_json::Map<String, Value>, pub filament: serde_json::Map<String, Value> }
pub fn catalog_presets(index: &PresetIndex, vendor: &str, machine: &str) -> Result<CliPresets, String>;
/// Orca's generic machine with the user's bed: printable_area [0,0,w,d], printable_height h,
/// bed_exclude_area ["0x0"], printer_model "Custom Printer".
pub fn custom_presets(index: &PresetIndex, width: f64, depth: f64, height: f64) -> Result<CliPresets, String>;
pub fn write_cli_presets(presets: &CliPresets, dir: &Path) -> io::Result<[PathBuf; 3]>;
/// The spike's export command: `<orca> --load-settings "m;p" --load-filaments f --arrange 1 --export-3mf <out> --outputdir <dir> <stl>`.
pub fn export_command(orca: &Path, presets: &[PathBuf; 3], stl: &Path, out_3mf: &Path, work: &Path) -> std::process::Command;
```
- [ ] **Step 2: Test for `custom_presets`** on the fixture tree: add our own `tests/fixtures/catalog/presets/Custom/machine/fdm_machine_common.json` (minimal keys) and `Custom/process/fdm_process_common.json`; assert the merged machine has `printable_area == ["0x0","300x0","300x250","0x250"]`, `printable_height == "220"`, and the process lists the custom machine name in `compatible_printers`. Locate the real Orca generic base on the image first (`ls /opt/slicer/resources/profiles/Custom/machine` via `ssh main docker run --rm --network none --entrypoint ls goat-packer-runner-orca:v0.2.0 …`) and use the same preset names in the fixture.
- [ ] **Step 3:** `cargo test --release --all` → PASS. Commit «orca_project: shared CLI preset flattening, custom-bed machine».

---

### Task 3: `source_project` binary + runner images

**Files:** `src/bin/source_project.rs`; `deploy/build-images.sh` (and the runner Dockerfile that copies binaries — find where `colored_3mf_packer` is copied and add `source_project` next to it).

CLI (hand-rolled args, `while let Some(option)` shape):
```
source_project --stl model.stl --presets /opt/slicer/resources/profiles --orca /opt/slicers/orca/AppRun
               (--vendor V --machine "Prusa MK4S 0.4 nozzle" | --custom-bed W,D,H) --output input.3mf --work-dir work
```
Exit non-zero with a Russian-free English error (the backend translates via `CliErrorTranslator`); add an entry to the translator for «STL could not be converted» → «Не удалось подготовить проект из STL: …».

- [ ] **Step 1:** implement with `orca_project`; after export, check `input.3mf` exists and its `Metadata/project_settings.config` has `printable_area` (sanity).
- [ ] **Step 2:** unit-test only argument validation (e.g. `--custom-bed 300,250` without height → error) — no slicer in tests.
- [ ] **Step 3:** add to runner images; `sh deploy/build-images.sh test-local` dry run if possible, else review the diff carefully.
- [ ] **Step 4:** manual check on `main` exactly like the spike (temp dir, `--network none`, cleanup): convert the 20 mm cube for `Prusa MK4S 0.4 nozzle` and for `--custom-bed 220,220,250`, then run `colored_3mf_packer` on each result.
- [ ] **Step 5:** commit «source_project: STL → slicer project on the runner».

---

### Task 4: Backend — vendor, STL uploads, custom bed, unknown-3MF bed

**Files:** `ProfileService.kt`, `JobDtos.kt` (`ProfileDto.vendor`, detection DTO), `server/Storage.kt` (accept `.stl`), local upload/detect controller (find where `.3mf` is validated: `grep -rn '\.3mf' webapp/backend/src/main`), `server/Queue.kt` (`JobRequest.customBed`), local `PackParams`, `job/JobRunner.kt`, `server/QueueWorker.kt`, `server/NativeLauncher.kt` (copy `input.stl` when present), tests.

- [ ] **Step 1: `vendor`.** `ProfileDto.vendor: String` = JSON `vendor`, else the first word(s) of `printer_name` for hand-made profiles («Bambu Lab», «Snapmaker»): rule = `printer_name` minus its last token. Test in `ProfilesIntegrationTest` (`p1s.vendor == "Bambu Lab"`).
- [ ] **Step 2: STL accepted** by uploads (server) and local multipart; extension check `.3mf` or `.stl`, size limits unchanged. Detect endpoint for `.stl` returns `{profileId: null, printerModel: null, kind: "stl"}` without parsing. Tests: upload `.stl` accepted, `.obj` rejected with the existing Russian message updated to «Нужен файл .3mf или .stl».
- [ ] **Step 3: Unknown 3MF bed.** Detection DTO gains `projectBed: {printableArea:[…], exclusionRectangles:[…], printableHeight}` when `profileId == null` and the 3MF's `project_settings.config` has `printable_area` (parse with the same point rules as `orca_presets`; port the 30-line parser to Kotlin or expose via the Rust detector if one exists — check `ProjectInspector`/`projects.inspect`). Test with a fixture 3MF generated in the test (zip with a minimal `project_settings.config`).
- [ ] **Step 4: Job params.** `JobRequest.customBed: CustomBed? = null` (`width, depth, height: Double, margin: Double = 5.0`), validated (50–1000 mm, margin 0–30). Local `PackParams` the same (multipart fields `customWidth`, `customDepth`, `customHeight`, `customMargin`). Exactly one of `profileId` / `customBed` must be set for STL; for 3MF either detection's `profileId`, a user-picked `profileId`, or `customBed`.
- [ ] **Step 5: Ad-hoc profile.** When `customBed` (or unknown-3MF `projectBed` confirmed by the user) is used, the worker writes `profile.json` from a template: `id "custom"`, `printer_name "Custom Printer"`, `slicer_kind "orca"`, `slicer_path` runner orca path, `printable_area [0,0,w,d]`, `edge_margin_mm margin`, `exclusion_rectangles` from projectBed or `[]`, `purge_lines []`, `tower_marker_style "wipe_tower"`, `tower_line_width_mm 0.5`, `blank_start_gcode_for_cli false`, `no_check_for_final_tower_probe true`. Unit-test the template builder.
- [ ] **Step 6: Conversion step.** If the job input is `.stl`, `JobRunner` (local) / worker (server) runs `source_project` first (catalog: `--vendor`, `--machine` from the profile's `vendor`/`orca_machine`; custom: `--custom-bed`), stage message «Готовлю проект из STL…» (add a tracker rule for its stderr line or a stage event — follow `ProgressTracker` patterns; percent stays in the probe band start). Local mode needs `GOAT_SLICER_PRESETS` (path to an Orca `resources/profiles` JSON tree) and the Orca binary path; if unset, STL jobs fail fast with «STL в локальном режиме требует GOAT_SLICER_PRESETS». Hand-made profiles (P1S, A2L, U1) have no `orca_machine`: add `"orca_machine"` + `"vendor"` to those three JSONs (`Bambu Lab P1S 0.4 nozzle`, `Bambu Lab A2L 0.4 nozzle` — check it exists in the image's presets; `Snapmaker U1 (0.4 nozzle)`).
- [ ] **Step 7:** `TESTCONTAINERS_RYUK_DISABLED=true ./gradlew test` → PASS. Commit(s).

---

### Task 5: Frontend — grouping (TDD) and PrinterPicker

**Files:** `src/utils/printers.ts`, `tests/printers.test.ts`, `src/components/PrinterPicker.tsx`, CSS in `src/styles/workspace.css`.

- [ ] **Step 1: Failing tests**

```ts
import { describe, expect, test } from 'bun:test';
import { filterPrinters, groupPrinters } from '../src/utils/printers';

const p = (id: string, name: string, vendor: string) => ({ id, name, vendor });
const list = [p('b', 'Prusa MK4S', 'Prusa'), p('a', 'Bambu Lab P1S', 'Bambu Lab'), p('c', 'Prusa CORE One', 'Prusa'), p('d', 'Ender-3 V3 SE', 'Creality')];

describe('printers', () => {
  test('vendors A→Z, models A→Z inside', () => {
    expect(groupPrinters(list).map((g) => [g.vendor, g.items.map((i) => i.name)])).toEqual([
      ['Bambu Lab', ['Bambu Lab P1S']],
      ['Creality', ['Ender-3 V3 SE']],
      ['Prusa', ['Prusa CORE One', 'Prusa MK4S']],
    ]);
  });
  test('search matches vendor or model, case/space-insensitive, keeps grouping', () => {
    expect(groupPrinters(filterPrinters(list, 'prusa mk')).flatMap((g) => g.items.map((i) => i.id))).toEqual(['b']);
    expect(groupPrinters(filterPrinters(list, 'creal')).map((g) => g.vendor)).toEqual(['Creality']);
    expect(filterPrinters(list, '').length).toBe(4);
  });
});
```
- [ ] **Step 2:** implement with `localeCompare(…, 'ru', { sensitivity: 'base' })`; search = every whitespace-separated token appears in `vendor + ' ' + name` (case-insensitive).
- [ ] **Step 3: `PrinterPicker`** — an input with a listbox popup (pattern: ozonware `frontend/src/components/Combobox.tsx` grouping/keyboard; same author): `role="combobox"`, `aria-expanded`, `aria-controls`, `aria-activedescendant`; ArrowUp/Down move across items (skipping vendor headers), Enter selects, Escape closes; vendor headers sticky in the list; last fixed option «Моего принтера нет в списке» (value `custom`). Props: `profiles`, `value: string | 'custom' | null`, `onChange`, `locked?: boolean` (detected from 3MF → read-only display with the printer name and «определён по файлу»).
- [ ] **Step 4:** typecheck/lint/tests/build pass. Commit.

---

### Task 6: Frontend — STL, picker, custom bed, scene before upload

**Files:** `components/FileDrop.tsx` (accept `.3mf,.stl`; copy «Выберите .3mf или .stl»), `pages/PackPage.tsx`, `api/types.ts` (`vendor`, detection `projectBed`/`kind`), `api/services.ts` (send `profileId`/custom fields), `scene/bed.ts` (accept a synthetic profile).

- [ ] **Step 1:** Printer state in PackPage: `printerChoice: { kind: 'detected', id } | { kind: 'picked', id } | { kind: 'custom', width, depth, height, margin } | null`.
  - 3MF detected → `detected` (picker locked).
  - 3MF unknown with `projectBed` → `custom` prefilled from the project bed (fields editable), note «Принтер не в каталоге — размеры взяты из проекта».
  - STL → picker enabled, required.
  - «Моего принтера нет в списке» → fields Ширина / Глубина / Высота, мм and «Отступ от края, мм» (default 5, hint «линия прочистки неизвестна — держим запас»).
- [ ] **Step 2:** The scene renders as soon as a printer is chosen (before any file): build a synthetic `PrinterProfile` for custom beds (`printableArea [0,0,w,d]`, `edgeMarginMm margin`, `exclusionRectangles` from projectBed or `[]`, `plateOutline null`, `vendor 'Custom'`). Add a `bed.test.ts` case for the synthetic profile.
- [ ] **Step 3:** Submit: send `profileId` (detected/picked) or custom fields; `canSubmit` requires a file and a printer choice; STL + custom requires all three dimensions valid (50–1000).
- [ ] **Step 4:** Verify: typecheck/lint/tests/build; local E2E with backend in local mode (`GOAT_SLICER_PRESETS=/Users/petrpopov/Developer/projects/main/OrcaSlicer/resources/profiles`, Orca app path): STL + «Prusa MK4S» packs; STL + custom 220×220×250 packs; a 3MF from P1S still auto-detects and locks the picker.
- [ ] **Step 5:** Commit.

---

## Self-review
- Spec «Upload scenarios» rows 1–4 → Tasks 4, 6 (+3 for STL conversion); «Web UI» combobox grouped by vendor A→Z, search inside the field, «нет в списке» → Tasks 5, 6; licensing: presets read at runtime from the slicer installed in the runner image, nothing copied.
- Detection ambiguity across nozzle variants removed by Task 1.
- Schedule page keeps 3MF detection only (its input is an already-packed plate); STL there is out of scope.

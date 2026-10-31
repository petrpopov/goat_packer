# Printer catalog from OrcaSlicer, STL uploads and «нет в списке» — design

Date: 2026-11-01. Status: agreed with the user in chat; implementation plans follow.

## Goal

Any consumer FDM printer owner can use the service with no local scripts and no slicer installed: upload a 3MF **or an STL**, pick their printer from a searchable list grouped by manufacturer (or «Моего принтера нет в списке» and type the bed size), and pack. The catalog covers the rectangular-bed printers that OrcaSlicer ships (≈940 of 1001 instantiable machines at tag 2.4.2); deltas/circular beds are out of scope.

## Licensing

OrcaSlicer profiles and bed assets are AGPL-3.0. We take **numbers** (printable area, excluded areas, height, nozzle, model name) — facts, not expression — and write them in our own `profiles/*.json` schema. Bed outlines: traced from the slicer's bed model/texture into **our own simplified polygon** (10–40 points); the STL/SVG files themselves are never copied into the repo or shipped. Purge lines and tower behaviour come from G-code **our** probe slice produced, not from copying start G-code. Each imported profile records its provenance (`source`: Orca tag, date, qualification result).

## Who runs what

- **End user:** browser only. Picks/gets a printer, uploads, packs. Slicing happens on our server (runner image already ships pinned BambuStudio and OrcaSlicer AppImages).
- **Us (developers/CI):** a catalog tool in this repo, run when we bump the pinned Orca version. It reads Orca's JSON profiles from a pinned OrcaSlicer checkout (the installed macOS app ships binary `.opc` caches; the JSON lives in the source tree), extracts facts, and **qualifies** each printer by slicing in the runner image. Output: committed `profiles/*.json`.

## Upload scenarios

| Upload | Printer | Settings used to slice |
|---|---|---|
| 3MF, known `printer_model` | auto-detected (today) | the 3MF's own `project_settings.config` |
| 3MF, unknown `printer_model` | geometry read from the 3MF (`printable_area`, `bed_exclude_area`, `printable_height`); ask the user only if missing | the 3MF's own settings |
| STL | user picks from the catalog combobox | server builds a 3MF via Orca CLI: `--load-settings "<machine>;<process>" --load-filaments "<filament>" --export-3mf` with the printer's resolved Orca presets |
| STL or 3MF, «Моего принтера нет в списке» | user types width × depth × height (mm), optional edge margin (default 5 mm — unknown purge line) | Orca's generic machine base («Custom Printer», `fdm_machine_common`) with `printable_area`/`printable_height` overridden |

Defaults for STL / unknown printers: filament **Generic PLA** (OrcaFilamentLibrary `Generic PLA @System`); process **0.20mm Standard** compatible with the printer's nozzle, else Orca's base process. They only affect slicing/time estimates; packing is geometry. An STL is single-colour → no prime tower.

## Catalog profile fields

Existing `PrinterProfile` keys stay; imported profiles fill:
- `id` (slug of Orca machine name), `printer_name`, `printer_model` (to match 3MF detection), new `vendor` (manufacturer, for grouping), new `orca_machine` (resolved preset name to load for STL builds), `slicer_kind: "orca"`, `slicer_path` (runner path);
- `printable_area` (rectangle; also accept Orca's comma-string format), `exclusion_rectangles` (from `bed_exclude_area`, rectangles only; polygons → bounding rectangle), `printable_height`, `edge_margin_mm` (0 unless qualification needs more);
- `purge_lines` — detected from the probe slice: extruding moves before the first layer marker, inside the printable area, as axis-aligned `[x0,y0,x1,y1,width]` (diagonal segments → their bounding strip); off-bed purges ignored;
- `tower_line_width_mm` from `parse_prime_tower_bounds` on a two-colour probe; `no_check_for_final_tower_probe` / `blank_start_gcode_for_cli` set by the qualification run when the plain run fails and the workaround passes;
- `plate_outline` traced (see Licensing);
- absent: `filament_change_seconds`, `nozzle_clean_gcode`, `heat_zones` (hardware-measured) → those options are disabled for the printer, exactly like today for A2L/U1;
- new `source`: `{ "orca_tag": "v2.4.2", "imported": "2026-11-…", "qualified": true|false, "notes": [...] }`.

The three hand-made profiles (P1S, A2L, U1) stay authoritative; the importer never overwrites a profile without `source` (or with `source.manual: true`).

## Qualification (per printer, in the runner image)

1. Resolve the Orca machine preset (merge the `inherits` chain), pick process + filament as above.
2. Build `probe.3mf` from a two-colour cube with Orca CLI `--export-3mf`; slice it with our `slicer_cli::slice_command` (`--arrange 0`).
3. Detect purge lines and tower from the G-code.
4. Run `colored_3mf_packer` on `probe.3mf` with the generated profile (dense plate) and `verify_outputs`; the final slice must succeed without «gcode path conflicts».
5. Write the profile with `source.qualified` and the report row (vendor, model, result, reason).

Unqualified printers are not shown in the catalog (or shown disabled with the reason) — decided per report.

## Web UI

- **Printer combobox** (pack, schedule): search inside the field, options grouped by `vendor`, vendors A→Z, models A→Z inside a vendor; last option «Моего принтера нет в списке» opens W×D×H (+ margin) inputs. For a 3MF with a detected printer the combobox shows it pre-selected and locked (today's behaviour), with «Сменить» only when detection failed. Pattern reference: ozonware `frontend/src/components/Combobox.tsx` (same author; grouping already implemented there).
- The bed scene draws the selected or custom printer immediately, before upload.
- `/api/profiles` returns `vendor`; custom bed is sent with the job (`customBed: {width, depth, height, margin}`).

## Out of scope

Delta/circular beds; PrusaSlicer-format 3MF projects (only Orca/Bambu-family 3MF); hardware-only values (AMS change time, nozzle-clean macros) for imported printers.

## Rollout order

1. Spike: Orca CLI in the runner image slices an STL with `--load-settings` and exports a 3MF that our packer accepts (never done in this repo yet).
2. Catalog tool: resolve presets, extract facts, trace outlines, purge/tower detectors (pure, unit-tested on fixtures), qualification runner; first batch: Bambu, Prusa, Creality, Elegoo, Anycubic, Qidi, Sovol, Flashforge, Snapmaker, Voron; then the full catalog.
3. Backend: STL → 3MF build step, custom-bed jobs, unknown-3MF geometry fallback, `vendor` in the API.
4. Web: printer combobox, «нет в списке» inputs, STL upload.

## Appendix: CLI spike (2026-11-01)

Runner images `goat-packer-runner-orca:v0.2.0` (OrcaSlicer 2.4.2, same tag as the preset checkout) and `goat-packer-runner-bambu:v0.2.0` on `main`, `docker run --rm --network none --cpus 2 --memory 4g --user 0`. Probe mesh: a 20×20×10 mm binary STL cube generated on the fly (`tests/fixtures/square.stl` is 184 bytes, too small to be useful). Printers: Prusa MK4S 0.4 nozzle (process `0.20mm SPEED @MK4S 0.4`), Creality Ender-3 V3 SE 0.4 nozzle (`0.20mm Standard @Creality Ender3V3SE 0.4`), Bambu Lab P1S 0.4 nozzle (`0.20mm Standard @BBL X1C`); filament `Generic PLA @System` (OrcaFilamentLibrary) for all.

### Presets for `--load-settings`

- Each preset is flattened by hand: merge the `inherits` chain (child keys win), drop `inherits`, set `"from": "system"`, `"instantiation": "true"`. No `version` key was needed; neither `--datadir` nor `--allow-newer-file` nor `--load-defaultfila`.
- **`inherits` must resolve inside the preset's own vendor folder** (then `OrcaFilamentLibrary`): 130 names repeat across vendors at 2.4.2 (`fdm_machine_common`, `fdm_filament_pla`, …). A global name map picks an arbitrary vendor's base (first attempt produced a wrong 3.9 kB Creality machine). `PresetIndex` currently keys by name only — it must become vendor-scoped.
- **The process's `compatible_printers` must list the machine's name explicitly.** The CLI's check (`OrcaSlicer.cpp` ~2575) compares only that list with the printer's system name and ignores `compatible_printers_condition`. Prusa processes use the condition with an empty list → `result.json`: `"return_code": -17`, `"error_string": "The selected printer is not compatible with the process preset in the 3mf."` (log: `process not compatible with printer.`). Fix: overwrite `compatible_printers = ["<machine name>"]` in the flattened process (and the filament, harmless).

### Export (works for all three)

```bash
/opt/slicers/orca/AppRun --load-settings "machine.json;process.json" --load-filaments filament.json \
  --arrange 1 --export-3mf probe.3mf --outputdir /spike/<p> /spike/cube.stl
```

Exit 0, `result.json` `"Success."`, ~10–13 kB 3MF (`3D/3dmodel.model`, `3D/Objects/cube.stl_1.model`, `Metadata/project_settings.config`, `model_settings.config`, `slice_info.config`, `filament_sequence.json`). `project_settings.config` has `printer_model` (`Prusa MK4S`, `Creality Ender-3 V3 SE`, `Bambu Lab P1S`), `printable_area` (`["0x0","250x0","250x210","0x210"]` etc.), `printer_settings_id`/`print_settings_id`/`filament_settings_id` = the preset names, `from: "project"`. `--arrange 1` centres the object (build item translation 115,95 / 100,100 / 118,118). The 3MF's `Application` metadata is `BambuStudio-02.06.00.51` (Orca fork). Notes: `bed_exclude_area` is written as `["0x0"]` when empty (a degenerate one-point "polygon" → must yield no rectangle); `enable_prime_tower: 1`, `wipe_tower_x/y = 15/220` even on the 210 mm-deep MK4S bed — irrelevant for a single-colour probe, but the two-colour tower probe must set a sane `wipe_tower_x/y`. Stderr noise (xkbcomp keysym warnings, `XDG_RUNTIME_DIR is invalid`) is harmless.

### Slice (the `slicer_cli::slice_command` shape)

```bash
/opt/slicers/orca/AppRun --arrange 0 --slice 1 --outputdir out probe.3mf
```

Exit 0; output is plain `out/plate_1.gcode` (+ `out/result.json`), no `.gcode.3mf`. First-layer markers: MK4S and Ender-3 V3 SE `;LAYER_CHANGE` then `;Z:0.2` (50 per 10 mm cube); P1S (BBL preset) `; CHANGE_LAYER` + `; Z_HEIGHT: 0.2`, no `;LAYER_CHANGE`. All present in `purge_detect::LAYER_MARKERS`; no start G-code contains a marker before its purge.

### Purge detection (`purge_detect::purge_lines` on the fetched G-code)

| Printer | Start-G-code purge | Detected strips |
|---|---|---|
| Prusa MK4S | `G0 X0 Y-4` then `X15…X45` at Y-4 (off-bed) | none (correct: outside the printable area) |
| Ender-3 V3 SE | two lines at X-2.1 / X-2.4, Y20–145 (off-bed) | none (correct) |
| P1S (Orca preset) | on-bed | `[18,1,240,1,0.81]`, `[240,1,240,11,0.84]`, `[239.5,11,239.5,1.5,0.89]`, `[239.5,1.5,18,1.5,0.81]` |

The P1S strips match the hand-made `profiles/p1s.json` coordinates exactly; widths come out 0.81–0.89 (from E/len/z) instead of the hand-made 0.5, i.e. conservative.

### Packer + verifier on `probe.3mf`

Profile = the `printer_catalog extract` draft as is (extra keys `vendor`, `source`, `orca_machine`, `printable_height`, `nozzle_diameter` are accepted), `slicer_path=/opt/slicers/orca/AppRun`. `colored_3mf_packer --source probe.3mf --profile profile.json --output packed.3mf --work-dir work`, then `verify_outputs work/embedded_source.stl work/placements.json packed.3mf --minimum-gap 0.7 [--keepout …] [--purge-line …]`.

- **MK4S:** 110 cubes, final Orca slice OK (110 `M486 A` objects, support check clean), 17 s. Note `no known slicer mapping for printer_model 'Prusa MK4S'; trusting profile slicer_kind 'orca'`.
- **Ender-3 V3 SE:** 100 cubes, final slice OK (100 objects per layer), 16 s; same note.
- **`verify_outputs` failed on both with `AssertionError: placement 0 violates 0.000 mm printable-area margin`** — float noise, not a layout error: the cube's flat side sits exactly on the bed edge (x = 0 at 270°, `cos` ≈ −1.8e−16). With `--edge-margin 0.5` on both tools the report shows `minimum_edge_clearance_mm: 0.4999999999999982` and still fails at 0.5, passes at 0.49; at zero margin `--edge-margin -0.001` passes (`minimum_exact_gap_mm: 1.0`, CRC/transform/metadata OK). `verify_outputs`' `covers` check needs an epsilon (or qualification passes a −0.001 margin) before it can gate qualification.
- **P1S via Orca:** packer refuses: `error: printer_model 'Bambu Lab P1S' implies slicer 'bambu', but profile 'bambu_lab_p1s_0_4_nozzle' is configured for slicer 'orca'` (guard in `colored_3mf_packer.rs` ~1688). Imported BBL printers either get `slicer_kind: "bambu"` or the guard needs an exception — decide in Task 5/6. With `slicer_kind: "bambu"` + `/opt/slicers/bambu/AppRun` (runner-bambu image) the same Orca-exported `probe.3mf` packs 141 cubes, final BambuStudio slice OK, verify OK (purge gap 3.06 mm, keepout clear); `support check skipped: the G-code carries no object labels`.

Not covered: the two-colour probe for tower detection (single-colour cube only).

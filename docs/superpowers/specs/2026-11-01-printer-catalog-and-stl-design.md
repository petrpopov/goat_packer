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

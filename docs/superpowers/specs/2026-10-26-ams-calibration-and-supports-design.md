# AMS change-time calibration test; support overlap control — design

Date: 2026-10-26
Status: roadmap sub-projects 5 and 6; details decided autonomously (see "Decisions").

## 5. AMS calibration test

### What the user gets

Page "Калибровка AMS" on the portal:

1. Choose printer profile and variant: **обычный** (10 changes) or **перфекционист** (30).
2. Upload any of their own project 3MFs for that printer (the one they pack is fine). The test
   inherits its printer, process, filaments and AMS type, so the measured correction is relative
   to exactly the settings the packer will later see.
3. Job output to download: `calib_A.3mf`, `calib_B.3mf`, `instructions.md`, `calibration.json`.
4. After printing: form with four clock times (A start, A finish, B start, B finish; `HH:MM`,
   a finish earlier than its start means after midnight). Result: "полная длительность одной
   смены: N с", plus "использовать как коррекцию" which stores N per profile in browser
   localStorage and pre-fills the pack/schedule `changeSeconds` field.

Profiles without `filament_change_seconds` (tool changers, U1) show "калибровка не нужна".

### Test model

- Cube 20 × 20 mm, generated mesh (12 triangles), centred on the usable bed (away from purge
  lines / exclusions), extruder 1.
- `calib_A.3mf`: the cube in filament 1, height H, no changes.
- `calib_B.3mf`: the same cube; `Metadata/custom_gcode_per_layer.xml` with `ToolChange`
  entries alternating filament 2 / filament 1 at band boundaries. Exactly `changes` entries.
- Bands are whole layers of the template's `layer_height` (first layer `initial_layer_print_height`):
  normal 10 changes × 4 layers, perfectionist 30 changes × 2 layers; H = first layer + bands
  × band thickness, cube top = last band top. Changes are placed at layer tops.
- Template must have ≥ 2 filaments, else error "для теста нужен проект минимум с двумя
  филаментами".
- `project_settings.config` copied from the template (incl. `printer_model`, AMS type, prime
  tower settings); `wipe_tower_x/y` kept as is but clamped so the tower does not overlap the
  cube (move the cube instead if needed: cube at bed centre, tower at template position; if
  they collide shift the cube by tower width + 10 mm along X).
- After generating, the binary slices both files with `slicer_cli::slice_command` (production
  slicer) and writes `calibration.json`: `{ variant, changes_expected, changes_sliced,
  estimate_a_s, estimate_b_s, slicer_change_s }`. `changes_sliced != changes_expected` is an
  error (guards the XML format against slicer versions).

### Computation

```
extra = ((fact_B − est_B) − (fact_A − est_A)) / changes
real_change_s = extra + slicer_change_s
```

`extra` outside 0..200 s → warning "похоже, время записано неверно", value still shown.

### Instructions (instructions.md, Russian, plain words)

PLA; load filament 1 and 2 into the slots/AMS unit you normally print from (gen-1 AMS: the far
unit is slower — test the one you use); cold printer before each print (15 min after the
previous); same calibration toggles for A and B; write down the time you pressed «Печать» and
the time of the «Печать завершена» notification; enter the four times on the page. Explain
why two prints (the control print cancels heating and calibration) and the accuracy
(normal ≈ ±12 s, perfectionist ≈ ±4 s per change).

### Components

- `src/ams_calibration.rs` (lib): cube mesh, band heights, `custom_gcode_per_layer.xml`,
  3MF assembly from a template; pure, unit-tested.
- `src/bin/ams_calibration.rs`: `--template <3mf> --profile <json> --variant normal|perfectionist
  --out-dir <dir>`; generates, slices, writes json + instructions.
- Webapp: job kind "calibration" in local (`JobRunner`) and server (`Queue`/`QueueWorker`)
  modes following the pack job pattern; page + calculator in the frontend (calculator is pure
  frontend code with unit tests).

## 6. Supports

Today: the probe's first-layer patch carries the support base; support zones may overlap each
other without limit, bodies never touch anything foreign; orientation locks when supports reach
past the silhouette.

- New option `--support-overlap-mm X` (frontend "Перекрытие поддержек, мм"):
  empty = unlimited (today), `0` = supports of different objects must not overlap,
  `X > 0` = they may overlap by at most X mm. Implemented as a third no-fit pair in the packing
  search: support cores (full footprint mask eroded by X/2 — i.e. dilated by
  `clearance − X/2`, raster erosion when negative) must not overlap. Bodies keep the existing
  rule untouched. `validate_result` gets the same check (independently re-rasterized).
- Report `supports_detected: bool` (patch reaches past the silhouette) and the frontend shows
  "обнаружены поддержки: поворот моделей заблокирован".

## Decisions

- Upload-based template instead of shipping printer templates in the repo: no 50 KB settings
  blobs to maintain, and the correction matches the user's own AMS type and process.
- Two separate files instead of two plates in one 3MF: no multi-plate assembly, and "file A /
  file B" is easier to follow.
- Calculator in the frontend with localStorage: no server persistence or accounts needed.

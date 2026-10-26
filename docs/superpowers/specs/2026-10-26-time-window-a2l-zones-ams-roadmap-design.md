# Time window, A2L heat zones, AMS change-time correction — roadmap

Date: 2026-10-26
Status: agreed decomposition; each sub-project gets its own spec → plan → implementation.

## Problem

A max-density A2L plate of multi-colour goats takes more than two days. Production wants a
plate that fits a window (12 h, 24 h), packed from the bed centre (A2L heats only the
first-layer bbox, Bambu Studio itself recommends filling the centre) with the prime tower next
to the objects. Slicer estimates for AMS printers are systematically low, so a time window
without a filament-change correction yields a "12 h" plate that really prints ~15 h.

## Facts this design rests on

### Orca CLI re-arranges objects without `--arrange 0`

Orca help: `--arrange option  Arrange options: 0-disable, 1-enable, others-auto`. Without the
flag Orca re-arranges the plate while slicing: the probe slice moves the goat to the bed
centre and a dense U1 plate silently slices 25 of 38 objects (koza-mini session, 2026-10-11/12).
koza-mini worked around it with a wrapper `orca_wrap.sh` (`exec OrcaSlicer --arrange 0 "$@"`)
as `slicer_path`; goat_packer itself still calls the slicer with only `--slice 1 --outputdir`
(`src/bin/colored_3mf_packer.rs` `slice_project`, `src/bin/daily_scheduler.rs`). Until fixed,
the final-slice verification on U1 proves nothing.

Related trap (settings, not placement): Bambu Studio / Orca reset on open any project value
not listed in `different_settings_to_system` (layer height fell back to 0.20, support gaps and
accelerations were lost). Every new edit of `project_settings.config` must be checked against it.

### A2L bed heat zones

System profile `Bambu Lab A2L 0.4 nozzle.json`, key `bed_heat_soak_area`
(`["102x93.4","228x93.4","228x226.6","102x226.6","60x49","270x49","270x271","60x271"]`):

| Zone   | X         | Y           | Size            |
|--------|-----------|-------------|-----------------|
| inner  | 102–228   | 93.4–226.6  | 126 × 133.2 mm  |
| middle | 60–270    | 49–271      | 210 × 222 mm    |
| full   | 0–330     | 0–320       | whole bed (assumed, not listed) |

The start G-code heat-soaks the first-layer bbox (`G29.30 X{first_layer_print_min[0]}
Y{first_layer_print_min[1]} I{first_layer_print_size[0]} J{first_layer_print_size[1]}`), so
the goal is a compact centred first-layer bbox of objects **and tower**. The purge line is not
part of that bbox (and can be drawn elsewhere anyway).

### AMS filament-change cost (koza-mini `memory/print-time-corrections.md`)

- Bambu printer profiles carry a per-AMS-type load/unload time: `default_ams_type` 2 = AMS Lite
  (11+12 s), 3 = AMS 2 Pro (18+19 s), 1 = AMS gen 1 (23+21 s). A2L defaults to Lite; a gen-1
  AMS is physically attached, which rewinds filament far back.
- Measured real extra over the estimate, per change: A2L +55.6 s (22 goats, 160 changes),
  later +72 s (30 goats); P1S +34 s; U1 (tool changer) no per-change term, +10–25 min prep.
- Per-change error grows with the interval between changes (+47 s short, up to +90 s long);
  one constant has roughly ±15 % error.
- The change count per plate (~160) does not depend on the object count: changes and flushes
  happen per layer, not per object. They belong to the fixed term of the time model.

## Decisions

- The time window is a **packer mode** (one plate fitting N hours), not a scheduler feature.
  The time model and AMS correction are shared with the scheduler so there is one estimate.
- Centre-out packing, heat zones and tower relocation apply only with the window enabled.
  Without a window the packer behaves exactly as today (max count, corner sweep).
- The AMS calibration test ships from the portal in two variants: **normal** (10 changes) and
  **perfectionist** (30 changes).

## Sub-projects (in order)

### 1. `--arrange 0` fix

Pass `--arrange 0` on every slicer CLI call for `slicer_kind = orca`; verify whether Bambu
Studio CLI accepts it and pass it there too if so. Leave a why-comment (the 25-of-38 repro).

### 2. Time model and AMS correction (shared module)

`time ≈ fixed + per_object × N`, corrected as
`corrected = estimate + changes × (real_change_s − slicer_change_s) + prep`, then a safety
margin (configurable %, default 5–10 %).

- `changes` comes from the slice output (Bambu `result.json` / `filament_change_times`, Orca
  G-code header).
- `slicer_change_s` is what the project's own slicer settings assume (load + unload for its
  `default_ams_type`), read from the project, so a user who changed the AMS type is neither
  double-corrected nor under-corrected.
- `real_change_s` (full real duration of one change) defaults from the printer profile JSON
  (new optional field; absent = no correction, e.g. U1) and can be overridden from the frontend.
- The webapp checkbox "коррекция времени на смену прутка" enables it.
- The scheduler's `prep_min` moves into the same model.

### 3. Time window in the packer

Off by default; frontend parameter "уложиться в N часов".

- Probe slice (1 object, already done) + one control slice give the line; pick N from it with
  the correction and margin; pack with the existing `max_objects` cap; the final slice (already
  done for the tower check) verifies; if over the window, drop trailing objects (packing order
  is the prefix order) and recheck by interpolation. No slice per added object.
- Fixed term alone exceeding the window is an explicit error ("не укладывается даже 1 объект").
- Assumption: identical clones (same height and colouring). Mixed objects break the affine model.

### 4. Centre-out packing, A2L zones, tower near objects

- Heat zones go into the printer profile JSON (new optional field), never into code.
- Placement order runs centre-out, so every prefix of N objects is compact around the centre;
  the chosen zone is the smallest one containing the prefix plus tower.
- The tower is moved next to the objects (in the centre when only the centre is filled) by
  writing `wipe_tower_x/y`. This is an explicit exception to "`project_settings.config` is
  copied byte-for-byte" and must be checked against the `different_settings_to_system` reset.
  The tower size is still measured by the probe slice; the repack-once rule stays.
- Centre-out may fit slightly fewer objects than the corner sweep on a full bed, hence it is
  window-only.
- Scheduler prefixes (`subset_3mf` keeps the first N build items) become centred too.

### 5. AMS calibration test and portal page

Model: 20 × 20 mm cube, 0.2 mm layers, 15 % infill, no supports, prime tower and flush on as in
real prints. One 3MF, two plates:

- **A**: the cube in one colour, no changes (control).
- **B**: the same cube, colour switches by height via `custom_gcode_per_layer.xml` `ToolChange`
  entries (no mesh painting).

| Variant       | Changes | Bands         | Accuracy per change |
|---------------|---------|---------------|---------------------|
| normal        | 10      | 10 × 1.2 mm   | ≈ ±12 s             |
| perfectionist | 30      | 30 × 0.6 mm   | ≈ ±4 s              |

Accuracy assumes four minute-resolution timestamps (±2 min on the difference) and ±1 min
heating spread.

- A Rust binary generates the project 3MF (no slicer; testable with self-contained tests).
  The server slices it once per profile and variant with the production slicer and caches the
  `.gcode.3mf`, plus the A/B estimates, change count and `slicer_change_s`.
- The portal page "Калибровка AMS" offers printer, variant, downloads (`.gcode.3mf` and
  instructions as text), a form for the four times, and the result "полная длительность смены
  N с" with a button to save it as the correction. For U1 it says calibration is not needed.
- User procedure: PLA, the AMS slots and unit normally used (gen-1 far unit is slower); cold
  printer before each print (15 min pause); print A, write down "pressed Print" and "Print
  finished" times; print B with the same calibration toggles, same two times.
- Computation: `extra = ((fact_B − est_B) − (fact_A − est_A)) / changes`;
  `real_change_s = extra + slicer_change_s`. Outside 0–200 s extra is flagged as a likely
  measurement error.
- Known bias: short intervals between changes give a lower bound (see measured +47 s vs +90 s);
  the window's safety margin covers it. A later refinement can learn from real prints.

### 6. Supports

Already mostly in place: the probe's first-layer patch carries the support base, the two-zone
rule lets supports overlap supports but never a body, and orientation is locked when the patch
exceeds the silhouette. Remaining: a frontend parameter for the support overlap tolerance and
an indication that supports were detected.

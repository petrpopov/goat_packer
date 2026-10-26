# CLAUDE.md

2D irregular-nesting packer for FDM printing: packs the real flat-bottom silhouette of a mesh (not its bbox) and rebuilds a slicer 3MF project from the layout. Second tool benchmarks slice times and schedules a 24h multi-printer run. Rust 2024, plain `std` + rayon/serde/zip/quick-xml/geo/tiny-skia; webapp in `webapp/` (Kotlin + React, см. `webapp/README.md`).

## Commands

```bash
cargo build --release
cargo test --release --all     # release is required: packing tests are slow in debug
cargo test --release colored_pack   # by name substring
```

CI: Woodpecker `ci.ezheg.xyz` (`.woodpecker/`): `check` on every push, `release` on tag `v*` builds `goat-packer-*:<tag>` into Docker on `main` via `deploy/build-images.sh` (the only description of the image build; no registry).

Tests must stay self-contained: generated temp data or `tests/fixtures/` only; no sibling-dir paths, symlinks, env-gated no-op tests, shared `/tmp` names. The packer and tests never call a slicer or network; the production colored-3MF pipeline does (slicer path comes from the printer profile).

## Packing core (`src/lib.rs`, `src/packing_search.rs`)

- Search is deterministic; `seed`/`candidates`/`stall` are compatibility fields, not random controls. More effort extends a prefix, never discards an earlier winner. Rayon only parallelizes independent angle families.
- `validate_result` must stay independent of the search's no-fit maps (it re-rasterizes). `verify_outputs` is a further independent check (`geo`, exact geometry) sharing no packing code: use it as second opinion on any layout change.
- Heuristic at finite resolution, not an optimality proof. Tradeoffs: `docs/packing-search.md`.

## Printer profiles / slicer quirks (`src/colored_packer.rs`)

- `PrinterProfile` (`profiles/*.json`) is the only source of printer geometry; never hardcode it. New printer = new JSON, not a code branch.
- `SLICER_KEEPOUT_SAFETY_MM = 4.0` is empirical (false "gcode path conflicts" on dense plates). Don't lower it from a single-object test.
- `slicer_kind` is inferred from `printer_model`, not 3MF `Application` metadata (Orca forks still say BambuStudio).
- No prime tower (`parse_prime_tower_bounds` -> `None`) is legitimate, not an error.
- Every slicer call goes through `slicer_cli::slice_command`, which passes `--arrange 0`: without it Orca re-arranges the plate on slice (25 of 38 sliced on U1). BambuStudio takes the same flag; don't narrow it to `slicer_kind = orca`.

## `colored_3mf_packer` (`src/bin/colored_3mf_packer.rs`) invariants

- Collision geometry comes from the mesh embedded in the source 3MF (`embedded_mesh`), never a separate STL (a drifted STL silently invalidated collisions). Its bbox center must be compensated (rotation-aware) in every build-item transform in `assemble`; otherwise clones with a source translation fall off the bed.
- `assemble` uses the default mesh mode, never `reuse_source_object_instances` (triggers "gcode path conflicts"). `project_settings.config` is copied byte-for-byte (single exception: a centre-out time-window plate shifts `wipe_tower_x/y[0]` after assembly, `plate_centre::relocate_wipe_tower_in_3mf`); `paint_color` lives in the mesh, so coloring survives.
- `evaluate_final_tower`: a tower appearing/disappearing only in the final slice is a hard error; only a grown tower earns one repack.
- Centre-out (`--time-window-hours`, opt out `--no-centre`; `src/plate_centre.rs`): placements are ordered by distance from the first `heat_zones` centre and the window keeps a prefix; the tower keepout moves there by the same shift as the setting (no re-probe), and the first final slice must show the tower within 5 mm of the target or it errors. No window = no reorder, no relocation.
- `prepare_cli_copy` resizes `flush_volumes_matrix` to `filament_colour.len()^2` and optionally blanks `machine_start_gcode`; Orca CLI rejects mismatches.
- `--nozzle-clean` (`src/nozzle_clean.rs`): wipe body is profile-owned (`nozzle_clean_gcode`); absent field = unsupported, explicit error (A2L has no hardware-validated sequence). Details: `docs/superpowers/specs/2026-09-02-nozzle-clean-injection-design.md`.

## Scheduler (`src/scheduler.rs`, `src/bin/daily_scheduler.rs`)

- Source must be single-plate (`plate_count_3mf` guard); build-item order is assumed to be one dense plate.
- xmltree drops namespace prefixes; `emit_core_xml` restores `p:UUID`/`p:path`/`xml:lang` by string replacement. `assemble.rs` avoids this by quick-xml round-trip, so keep new 3MF edits there.
- Multi-plate 3MF (`build_schedule_plates_3mf`), both locked by `tests/scheduler_plate_grid.rs`: plate grid is `cols = ceil(sqrt(n))` (not round), row-major, slot offset `bed_size * 1.2`; `wipe_tower_x/y` entries are used verbatim per plate, so every plate gets the SAME entry as the source (shifting it puts the tower off-bed, omitting it auto-places it onto objects).
- Benchmark `adaptive` (default) slices few sizes and interpolates (`origin: "interpolated"`); `--reuse-benchmarks` re-reads a prior `--output-report`.

## AMS calibration (`src/ams_calibration.rs`, `src/bin/ams_calibration.rs`)

- Generates two cube projects from the user's own 3MF (settings byte-for-byte, `Application` metadata copied: the slicers only load `project_settings.config` from a 3MF they recognise).
- A per-layer `ToolChange` applies to the layer whose `print_z == top_z` (that layer is already in the new filament), so entries carry the first layer of each band; an entry at the first layer's own z recolours it without a change (Bambu reported one change short). The binary errors when `changes_sliced != changes_expected`.

## Conventions

- Comments explain why something must not change (empirical slicer quirks); preserve and extend them.
- Binaries hand-roll arg parsing (no clap); follow the `while let Some(option)` shape.
- Design docs/plans: `docs/superpowers/{specs,plans}/`, dated.
- `build_schedule_workbook.mjs` needs `@oai/artifact-tool`.

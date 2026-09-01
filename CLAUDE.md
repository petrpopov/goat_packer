# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A 2D irregular-nesting packer for FDM 3D printing. It packs the **actual flat-bottom silhouette**
of a mesh (outer loop + inner holes), not its bounding box, then reassembles a real slicer 3MF
project from the resulting layout. A second tool benchmarks slice times per batch size and
optimizes a 24-hour multi-printer production schedule.

Rust edition 2024. No async, no framework — plain `std` +
`rayon`/`serde`/`zip`/`xmltree`/`quick-xml`/`geo`/`tiny-skia`/`sha1`.

## Commands

```bash
cargo build --release
cargo test --release --all            # release matters: packing tests are slow in debug
cargo test --release --test packing   # one integration test file
cargo test --release colored_pack     # by name substring
```

The `.mjs` workbook builder needs `@oai/artifact-tool`.

### External test fixtures

Several tests read `../Goat 3/**` and `../Goat 4/**` (STLs and several 3MFs) relative to the
repo root — those directories are **not** part of this repo and are currently absent. Affected:
`tests/footprint.rs`, `tests/quality.rs`, `tests/scheduler_plates.rs`, `tests/scheduler_subset.rs`,
`tests/assemble_fixtures.rs`, and two `#[cfg(test)]` cases inside `src/bin/colored_3mf_packer.rs`.
Everything else is self-contained (synthetic STLs via `tempfile`). Expect those to fail without
the asset directory; don't "fix" them by rewriting paths.

The end-to-end binaries additionally shell out to real slicer CLIs
(`/Applications/BambuStudio.app/...`, `/Applications/OrcaSlicer.app/...`) — paths come from the
printer profile, not from code.

## Architecture

### Layer 1 — geometry & packing (`src/lib.rs`)

The whole packing core lives in one file, in pipeline order:

1. `extract_footprint` — reads a **binary** STL, keeps only triangles whose three vertices sit at
   min-Z (within tolerance), finds odd-count edges (the boundary), and walks them into closed
   loops. Coordinates are snapped to a 1e-5 mm integer grid (`Key`) so vertex welding is exact.
   The result is centered on the mesh's XY bbox center. Loops are sorted by descending |area|, so
   `loops[0]` is the outer contour and the rest are holes.
2. `rasterize_footprint` — rotates the loops, point-in-polygon-samples them onto a pixel grid, and
   produces two masks: `physical` (exact silhouette) and `clearance` (dilated by `clearance/2`,
   round structuring element). Collision always uses `clearance`; bed-fit checks use
   `physical_bounds`.
3. `pack_footprint` — pre-rasterizes one `RasterMask` per rotation step, then runs `attempts`
   independent randomized restarts in parallel via rayon. Even attempts use `drop_attempt`
   (gravity-drop + contact scoring), odd use `greedy_attempt` (anchor-relative sampling). Restarts
   are reduced with `better_layout`: **object count is the only real objective; bbox area is a
   tiebreak.** `candidates_per_step` positions are sampled per placed object *within* one restart,
   so total cost scales with attempts × candidates, not their ratio. `stall_attempts` stops the
   search once that many consecutive restarts bring no improvement (the CLI's `--stall`, default
   `attempts/4`; `0` disables). Progress is printed to stderr.
4. `validate_result` — an independent re-check of the winning layout (re-rasterizes from scratch,
   pairwise mask overlap, bed bounds, exclusions). Every caller runs this; treat it as the
   contract, not as an optimization.

Collision is bit-parallel: `BitGrid` stores the occupied plate as `u64` words and shifts a mask's
rows into place, so a placement test is a handful of word ANDs. Grids are allocated with a `margin`
border so masks with negative `min_x/min_y` can be tested without bounds gymnastics.

The search is heuristic (irregular nesting is NP-hard). More `--attempts` can improve the result;
the seed makes any given command reproducible.

### Layer 2 — printer profiles & prime tower (`src/colored_packer.rs`)

`PrinterProfile` (JSON in `profiles/`) is the single source of truth for bed size, edge margin,
bed exclusion rectangles, purge lines, slicer path, and slicer kind. Both binaries load it — never
hardcode printer geometry.

`SLICER_KEEPOUT_SAFETY_MM = 4.0` is an empirically measured pad applied to *every* keepout. The
long comment above it records why (BambuStudio/Orca CLI raise false "gcode path conflicts" on dense
plates well beyond the geometric margin). Don't lower it based on an isolated single-object test —
that's exactly the measurement that understated it before.

`parse_prime_tower_bounds` walks G-code between `WIPE_TOWER_START`/`WIPE_TOWER_END` markers. It
returns `None` when there is no tower — a legitimate case (single-material project), not an error.

`slicer_kind` is inferred from `printer_model`, never from the 3MF's `Application` metadata:
OrcaSlicer forks (including Snapmaker's) still emit `Application: BambuStudio-x.y.z`.

### Layer 3 — `colored_3mf_packer` (`src/bin/colored_3mf_packer.rs`)

The one packing entry point. Flow:

```
validate printer match → extract embedded mesh to STL → probe-slice (1 object) to find the tower
→ pack around fixed keepouts + tower keepout → the assemble module builds the plate
→ final slice → evaluate_final_tower → (at most one repack) → write OUTPUT.verification.json
```

Two invariants encoded here as load-bearing decisions:

- Collision geometry comes from the mesh **embedded in the source 3MF** (`embedded_mesh`),
  never from a separately supplied STL. A drifted standalone STL silently invalidates every
  collision check while keeping the same bbox — this caused a real near-zero-clearance touch.
  The component's own transform is baked into that STL, and `assemble` compensates the baked
  mesh's bbox center (rotation-aware) in every build-item transform: a placement's (x, y) is where
  that center lands on the bed. A source component carrying its own translation (Goat 3:
  (-35.38, +40.12)) otherwise displaces every clone by a rotation-dependent ~54 mm and objects
  fall off the heated bed ("partly inside, can not be sliced").
- `assemble` runs with the default mesh mode, never `reuse_source_object_instances`. The
  shared-object/N-items mode reproducibly triggers BambuStudio's "gcode path conflicts" on dense
  plates; N distinct `<object>` resources sharing one external mesh component slices cleanly and
  is equally paint-safe.

`evaluate_final_tower` is the tower state machine: a tower that appears only in the final slice, or
disappears only in the final slice, is a hard error (packing reserved the wrong space). Only a
grown tower earns a repack, and only once.

`prepare_cli_copy` applies CLI-only workarounds to `Metadata/project_settings.config`: blanking
`machine_start_gcode` when the profile asks, and resizing `flush_volumes_matrix` to
`filament_colour.len()²` (stale-size artifact of projects trimmed down from multi-color templates;
Orca's CLI rejects the mismatch outright even though the values are never read).

### Layer 4 — scheduler (`src/scheduler.rs`, `src/bin/daily_scheduler.rs`)

`subset_3mf` produces a 3MF with only the first N build items, pruning `3dmodel.model`,
`model_settings.config`, `cut_information.xml`, and `plate_1.json` consistently. Note
`emit_core_xml`: xmltree drops the namespace prefixes 3MF requires, so `p:UUID`/`p:path`/`xml:lang`
are restored by string replacement after serialization.

The source must be **single-plate** — `plate_count_3mf` guards this, because build-item order is
assumed to be one dense plate and a multi-plate file would silently undercount batches.

`optimize_schedule` is a DP over end-second: day jobs (10:00→23:00, each followed by a turnaround)
maximize objects, then exactly one overnight job starts at 23:00 and must finish by 10:00 next day.
Ties break toward fewer jobs, then earlier completion.

Outputs land in `--output-dir`: `planner_report.json`, `benchmarks.csv`, `schedule.csv`,
`sensitivity.csv`. `--reuse-benchmarks` re-reads `planner_report.json` to skip re-slicing when only
turnaround/prep assumptions changed.

### Layer 5 — 3MF assembly & verification

- `src/embedded_mesh.rs` — derives the collision STL from the source 3MF: parses the core model
  with quick-xml, resolves the component's `p:path` and transform, and bakes that transform into
  the vertices (f64 math, LE f32 packing). Also exposes the mesh's XY bbox center — the exact
  point every build-item translation must compensate.
- `src/assemble.rs` — rebuilds the packed 3MF from the source template via a quick-xml event
  round-trip: untouched XML bytes round-trip verbatim, so attribute order, quoting, and the
  prefixed attributes xmltree cannot represent (`p:UUID`, `p:path`) all survive. Four mutually
  exclusive mesh strategies: default (N objects → one shared component), `duplicate_mesh_per_object`,
  `reuse_source_object_instances`, `inline_colored_mesh_objects`. Object/instance identities are
  deterministic uuid5s (sha1-based, stable across runs), floats print through the same
  `_number`-compatible formatting as the original artifacts, and extracted footprint rings go
  through `canonical_ring` (fixed start vertex + direction) so re-runs are byte-identical.
  `Metadata/project_settings.config` is copied byte-for-byte; paint (`paint_color`) attributes
  live in the untouched mesh part, so every mode preserves coloring; preview PNGs render in
  `src/plate_preview.rs` (tiny-skia).
- `src/bin/verify_outputs.rs` — independent exact-geometry check (`geo`, not the rasterized
  masks): minimum gap, edge margin, purge-line gap, keepouts, per-item transforms, ZIP CRC, and
  object/instance cardinality; same CLI as the old Python verifier
  (`verify_outputs <stl> <placements> <project> [--minimum-gap F] [--edge-margin F] ...`).
  Use it as a second opinion on any layout change; it shares no packing code with the Rust packer.
- `build_schedule_workbook.mjs` — turns `planner_report.json` into a formatted `.xlsx`.

## Conventions

- Comments in this codebase explain *why a thing must not change* (empirical slicer quirks,
  reproduced failures). Preserve and extend them rather than trimming; several encode bugs that
  cost real print time to find.
- New printers are added as a JSON profile in `profiles/`, not as code branches.
- Binaries hand-roll argument parsing (no clap); follow the existing `while let Some(option)` shape.
- Design docs and implementation plans live in `docs/superpowers/{specs,plans}/`, dated.

# Eliminate Python — Implementation Plan

Spec: `docs/superpowers/specs/2026-09-01-eliminate-python-rust-port-design.md` (read it first;
it holds the contracts, crate choices, uuid5 vectors, and acceptance criteria).

Execution model: tasks are grouped into waves. Tasks inside a wave touch disjoint files and run
as parallel subagents; waves run sequentially. Task 0 and Task 6 are done by the coordinator.

Global constraints for every task:

- Never touch `src/scheduler.rs`, `src/bin/daily_scheduler.rs`, or the user's uncommitted
  scheduler WIP anywhere (unstaged edits + untracked files). `Cargo.toml` already carries all
  new dependencies — do not edit it.
- Do not commit; the coordinator commits after integration.
- Do not invoke the real slicer CLIs; end-to-end slicing is Task 6.
- Comments explain *why a thing must not change* (empirical slicer quirks, reproduced
  failures) — port the existing explanatory comments from the Python, don't trim them.
- Rust edition 2024, std + existing crate style (hand-rolled arg parsing, `AppResult`-style
  error types, snake_case, no new frameworks).

## Wave 0 — bootstrap (coordinator, done before dispatch)

- [x] Spec + plan docs written.
- [x] `Cargo.toml`: add `quick-xml`, `tiny-skia`, `geo`, `sha1` (preserving the user's
      uncommitted `tempfile` move).
- [x] Empty module skeletons `src/embedded_mesh.rs`, `src/plate_preview.rs`, `src/assemble.rs`
      registered in `src/lib.rs`; `cargo check --release` green.

## Wave 1 — three parallel tasks (disjoint files)

### Task 1 — `embedded_mesh` module (ports `extract_embedded_stl.py`)

Files: `src/embedded_mesh.rs` (fill in), `src/bin/colored_3mf_packer.rs` (replace the
`derive_embedded_stl` subprocess with a direct call).

- Public API: parse a source 3MF's `3D/3dmodel.model` with quick-xml, resolve the component's
  `p:path` and its 12-number transform; expose
  `pub fn parse_transform(&str) -> Result<[[f64; 3]; 4]>` (4×3 row-major, last row translation,
  row-vector convention `p' = p·M`), `pub fn apply_matrix(&[[f64; 3]; 4], (f64,f64,f64)) ->
  (f64,f64,f64)`, `pub fn embedded_mesh_triangles(&Path) -> io::Result<Vec<[(f64,f64,f64); 3]>>`
  (component transform baked, f64), `pub fn stl_xy_center(&Path) -> io::Result<(f64, f64)>` (XY
  bbox center of the STL mesh), and `pub fn write_binary_stl(&Path, &[( [(f64,f64,f64); 3] )])`
  (80 zero bytes, LE u32 count, per triangle 12 zero bytes + 9 LE f32 + u16 zero).
- The f64→f32 packing order must match the Python exactly (acceptance 4 depends on it).
- Bin integration: `derive_embedded_stl` becomes an in-process call; update the error message
  prefix ("failed to derive collision STL from embedded mesh: …").
- Tests (in-module `#[cfg(test)]`): the synthetic-3MF fixture from
  `tests/test_build_3mf.py::synthetic_source_3mf` (component transform `(10, 20, -0.5)`, flat
  square) — assert baked corners, STL byte layout (header/count/record size), and that
  `extract_footprint` round-trips the written STL.
- Verify: `cargo test --release embedded_mesh`, `cargo test --release --bin colored_3mf_packer`,
  `cargo build --release`.

### Task 2 — `plate_preview` module (ports `_draw_preview` from `build_3mf.py`)

Files: `src/plate_preview.rs` (fill in). Consumed later by Task 4; no bin changes here.

- API suggestion: `pub struct PreviewInput<'a> { pub polygons: &'a [Vec<Point>], pub printable:
  &'a [(f64, f64)], pub exclusions: &'a [Vec<(f64, f64)>], pub purge_lines: &'a [(f64, f64,
  f64, f64, f64)], pub keepouts: &'a [(f64, f64, f64, f64)], pub edge_margin: f64, pub pick:
  bool }` and `pub fn render(input: &PreviewInput, size: u32) -> Vec<u8>` returning PNG bytes.
- Port the Python drawing exactly (layout/colors/geometry — see `_draw_preview`): background
  245/247/250, bed fill 224/228/233 + outline 45/50/58, green safe-area outline when
  `edge_margin > 0`, red purge lines with width `max(2, round(w·scale))`, exclusion fill
  242/112/112 α230 (when ≥3 points), keepout rects 238/140/76 α170 + outline 185/82/26,
  footprints 111/169/220 (pick mode: `((37·(i+1))%254+1, (83·(i+1))%254+1, (149·(i+1))%254+1)`)
  with outline 22/91/148; screen transform `x' = margin + (x−min)·scale`,
  `y' = size − margin − (y−min)·scale`, `scale` and 3% margin per the Python.
- tiny-skia: one path per footprint containing exterior ring then hole rings, filled
  `FillRule::EvenOdd` (holes over the object fill; no separate hole redraw needed); strokes via
  `Stroke { width }`; premultiplied RGBA `Paint`.
- Tests: render 512 and 128 → assert PNG signature, IHDR width/height, and that pixel bytes
  are non-constant (decode header by hand — no image crate).
- Verify: `cargo test --release plate_preview`.

### Task 3 — `verify_outputs` binary (ports `verify_outputs.py`)

Files: `src/bin/verify_outputs.rs` (new; auto-discovered bin target).

- CLI identical to the Python: positional `stl placements project`; `--minimum-gap 0.5`,
  `--edge-margin 0`, repeatable `--purge-line x0,y0,x1,y1,w`, `--purge-gap 0.5`, repeatable
  `--keepout min_x,min_y,max_x,max_y`, `--report path`. JSON report to stdout (and file), same
  field names as the Python output.
- Geometry via `geo`: build the footprint `Polygon` from `extract_footprint` loops (exterior =
  loops[0], interiors = the rest), per-placement rotate/translate; printable containment =
  inset rectangle `covers` (use `Relate`/`intersects` + explicit point-in-poly for boundary
  semantics); pairwise min gap via `Euclidean.distance`; purge keepouts as rectangles
  (`axis_aligned_line_keepout` semantics: segment bounds expanded by width/2); exclusion /
  keepout overlap area via `BooleanOps::intersection(...).unsigned_area()` with the 1e-7
  threshold.
- Per-item checks from the 3MF itself: parse `3D/3dmodel.model` (quick-xml) for items +
  component transform (reuse `embedded_mesh::{parse_transform, apply_matrix}`); assert count
  parity (resources/items/config objects/instances ∈ {1, N}), angle within 1e-6 of
  `atan2(m1, m0)` vs expected, translation equals `placement − R·c_stl` within 1e-6, composed
  mesh→component→item Z spans `[0, height]`; ZIP CRC via reading every entry.
- Tests (bin `#[cfg(test)]`): a synthetic 3MF built in a temp dir (zip crate) with a
  component-transformed square + two placements — the good case passes; a mutated transform
  fails with the expected message.
- Verify: `cargo test --release --bin verify_outputs`.

## Wave 2 — after Wave 1 lands

### Task 4 — `assemble` module + packer integration (ports `build_3mf.py`)

Files: `src/assemble.rs` (fill in), `src/bin/colored_3mf_packer.rs` (replace the
`build_colored_project` subprocess with a direct call; keep writing
`<work_dir>/placements.json` for debugging).

- API: `pub struct AssembleOptions { pub placements: Vec<PackedPlacement>, pub stl_path:
  PathBuf, pub edge_margin: f64, pub purge_lines: Vec<[f64; 5]>, pub keepouts: Vec<Rect>, pub
  duplicate_mesh_per_object: bool, pub reuse_source_object_instances: bool, pub
  inline_colored_mesh_objects: bool }` and `pub fn assemble_project(template: &Path, output:
  &Path, options: &AssembleOptions) -> Result<(), String>`.
- quick-xml event round-trip for `3D/3dmodel.model` (span-clone `<object>`s, rewrite
  resources/build, insert items with `transform = _transform(placement, z_offset,
  c_stl)` and `printable="1"`); z offset = source build item `transform[11]`
  (`_source_z_offset`); footprint center `c_stl` from `embedded_mesh::stl_xy_center`; preview
  polygons from `extract_footprint` loops rotated/translated per placement.
- Port `_rebuild_model_settings` (N objects `id=index+2`, plate `model_instance`s with
  `identify_id = 1000+10i`, `assemble_item`s with identity rotation + z translation),
  `_rebuild_cut_information` (`id=index+1`) and `plate_1.json` (`bbox_all`, `bbox_objects` ids
  `1002+10i`, hardcoded `layer_height` 0.2 and source-stl name, `first_layer_time` 0) — each
  only when present in the template.
- All four mesh strategies per spec §Contracts 5; `_number` float formatting and `_stable_uuid`
  (sha1-based uuid5, spec vectors as tests); duplicate mode appends relationships to
  `3D/_rels/3dmodel.model.rels`; inline mode bakes the component transform into inlined
  vertices; `Metadata/project_settings.config` copied verbatim.
- ZIP write: Deflate level 6, template entry order, new entries appended.
- Previews via `plate_preview::render` for the five PNG entries.
- Tests: Rust port of
  `tests/test_build_3mf.py::test_item_transform_compensates_component_translation` (synthetic
  template + baked STL via `embedded_mesh` + two placements: assert exact item transforms,
  world-space bbox centers equal placements via composed matrices, and `verify_outputs`-level
  checks); uuid5 vector test; `_number` formatting test; per-strategy resource/item cardinality
  tests on the synthetic template.
- Verify: `cargo test --release assemble`, `cargo test --release --bin colored_3mf_packer`,
  `cargo build --release`.

## Wave 3 — after Wave 2 lands

### Task 5 — test translation, docs, Python removal

Files: `tests/assemble_fixtures.rs` (new), `CLAUDE.md`, `README.md`, `README.ru.md`, deletions.

- Port the fixture-dependent Python tests (`../Goat 3/p1s/goat3_fast_max.3mf`,
  `../Goat 3/snap/goat3_snap.3mf`, `../Goat 4/...`) into `tests/assemble_fixtures.rs` following
  the repo convention for external fixtures (they compile now, run when the assets return):
  paint-attribute preservation, mesh byte-equality, per-strategy cardinality, z-offset
  preservation, metadata presence rules.
- Delete `build_3mf.py`, `extract_embedded_stl.py`, `verify_outputs.py`,
  `tests/test_build_3mf.py`, `__pycache__/`.
- Docs: CLAUDE.md — commands section drops the `python3 tests/test_build_3mf.py` lines and the
  trimesh/shapely/Pillow note; Layer 5 rewritten to describe `embedded_mesh`/`assemble`/
  `plate_preview`/`verify_outputs` (keep the empirical-quirk comments). READMEs — build/run
  sections note the toolchain is pure Rust (no Python); flag tables unchanged.
- Verify: `grep -rn "python3" src/ README.md README.ru.md CLAUDE.md` clean (historical docs in
  `docs/superpowers/` keep their references); `cargo build --release` and fixture-free
  `cargo test --release` green.

## Wave 4 — integration (coordinator)

### Task 6 — end-to-end, determinism, commit

- [x] `cargo build --release`; full self-contained `cargo test --release` green (all failures are
      the documented absent-`../Goat 3`/`../Goat 4` fixture tests, incl. the new
      `tests/assemble_fixtures.rs`).
- [x] E2E on the real Goat 3 P1S source with `--attempts 2107 --stall 0 --seed 20270831`:
      20 objects, probe + final BambuStudio slices succeed, tower stable, paint mentions
      preserved; new `verify_outputs` binary passes on the output with the p1s purge lines and
      tower keepout (min exact gap 0.738 mm, purge gap 4.45 mm).
- [x] Determinism: run twice, outputs byte-identical. The packed layout equals the Python
      pipeline's layout for the same seed/STL (placements.json diff empty).
- [x] Surgical commit (exclude the user's scheduler WIP hunks in shared files) + push.
- [x] Mark this plan's checkboxes and record results.

Two real-world integration bugs were found by the e2e slicer runs and fixed with regression
tests (both in `src/assemble.rs`): the plate's `<model_instance>` children were emitted after
`</plate>` with a duplicated closing tag (off-by-one in `rebuild_plate_events`' interior-copy
end), and `cut_information.xml` lost its root `</objects>` (tail slice started at the exclusive
span end instead of the End event).

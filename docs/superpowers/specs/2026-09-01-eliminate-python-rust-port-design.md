# Eliminate Python — Pure-Rust 3MF Toolchain

## Context

The packing pipeline currently shells out to Python twice per run
(`colored_3mf_packer.rs` → `extract_embedded_stl.py`, `build_3mf.py`), the exact-geometry
verifier (`verify_outputs.py`) is Python-only, and the assembly test suite
(`tests/test_build_3mf.py`) is Python. That couples every deployment to a `python3` runtime with
`trimesh`, `shapely`, and `Pillow` installed, and splits one pipeline across two languages.

Two research passes (2026-09-01) confirmed the port is feasible with exact-or-equivalent
numerics. This document is the contract for that port.

## Scope

Ported to Rust:

| Python artifact | Rust replacement |
| --- | --- |
| `extract_embedded_stl.py` | `src/embedded_mesh.rs` (lib module) |
| `build_3mf.py` | `src/assemble.rs` (lib module) + direct calls from `colored_3mf_packer` |
| `verify_outputs.py` | `src/bin/verify_outputs.rs` (standalone binary, same CLI) |
| `tests/test_build_3mf.py` | Rust integration/unit tests; fixture-dependent cases become `tests/assemble_fixtures.rs` following the existing `../Goat 3` / `../Goat 4` convention |

Out of scope: the slicer CLIs themselves (BambuStudio/Orca are external binaries, not Python)
and `build_schedule_workbook.mjs` (Node, internal `@oai/artifact-tool` dependency, manual-only;
can move to `rust_xlsxwriter` later).

## Contracts that must survive (load-bearing, several empirically earned)

1. **`Metadata/project_settings.config` is carried through byte-for-byte.** Never parsed or
   re-serialized by the assembler.
2. **The shared/duplicated mesh part is byte-identical** to the source's; `paint_color`
   attributes live there and must never be touched.
3. **Component-transform semantics** (fixed 2026-09-01, commit fc1a9dd): the collision STL is
   the embedded mesh with the component's own transform baked in; every build-item translation
   is `placement − R(angle)·c_stl` where `c_stl` is the baked STL's XY bbox center; the source
   item's z translation (`transform[11]`) is reused as the item z offset.
4. **Deterministic identities**: `uuid5(NAMESPACE_URL,
   "goat4-contour-packing/{kind}/{index}")` for `inline-resource`/`resource`/`component`/
   `build-item`/`part-{n}` kinds; object-id conventions — resources/build items use `index+2`,
   `cut_information.xml` uses `index+1`, `identify_id = 1000 + 10·i`, plate bbox ids
   `1002 + 10·i`; assemble_item z in `model_settings.config` uses the same z offset.
5. **Mesh strategies** (mutually exclusive): default (N objects, one shared component),
   `duplicate-mesh-per-object` (`*_instance_N.model` entries + appended
   `3D/_rels/3dmodel.model.rels` Relationships), `reuse-source-object-instances` (one object, N
   items; skips model_settings/cut/plate_1.json rewrites), `inline-colored-mesh-objects`
   (component transform baked into inlined vertices, no component). The packer binary uses the
   default mode only — the others exist for tests and manual recovery paths.
6. **Rewrite-only-if-present** for `cut_information.xml` and `plate_1.json` (Snapmaker projects
   lack them; never invent Bambu-only metadata).
7. **Preview PNGs** (`plate_1.png`, `plate_no_light_1.png`, `top_1.png`, `pick_1.png` at 512 px,
   `plate_1_small.png` at 128 px): same sizes/format and visual layout; pixel-exact PIL parity
   is explicitly NOT required (cosmetic, BambuStudio regenerates its own view).
8. **STL bytes are deterministic** and must remain f32-identical to the Python output:
   XML vertex parsed as f64 → component transform applied in f64 → packed as LE f32.
9. **The exact verifier's semantics**: min pairwise gap (min over all boundary segment pairs),
   printable-area containment with edge-margin inset, exclusion/keepout overlap (area > 1e-7
   threshold), purge-line gap against a keepout rectangle (axis-aligned segment expanded by
   width/2 — the shapely `buffer(cap_style=2, join_style=2)` equivalent), per-item angle /
   translation / on-bed-Z checks through the full mesh→component→item matrix chain, ZIP CRC,
   metadata cardinality. Tolerances stay at 1e-6 mm.

## Crate decisions (from research, 2026-09-01)

- **XML: `quick-xml`** (not `xmltree`). `xmltree` structurally cannot represent prefixed
  attributes (`p:UUID`, `p:path`, `xml:lang`) — it drops them at parse — which is why
  `scheduler.rs::emit_core_xml` restores prefixes by string replacement after serialization.
  quick-xml events keep raw bytes: untouched regions round-trip verbatim (attribute order,
  quoting, unknown namespaces), and new attributes carry literal prefixes. The assembler uses
  the event-span pattern: parse once into owned events, deep-clone `<object>` spans by cloning
  event slices, edit attributes with `clear_attributes` + `push_attribute`, insert `<item>`s
  before `</build>`. Do not refactor `scheduler.rs` in this port (it has unrelated WIP); the
  xmltree-removal there is a follow-up.
- **Geometry: `geo` 0.33** (replaces shapely). `Euclidean.distance(&Polygon, &Polygon)` is
  semantically identical to shapely's `distance` (min over boundary segments, holes handled, 0
  on intersect/touch); evaluation order differs so expect ~1e-12 relative differences —
  irrelevant at 1e-6 tolerances. Use the new `line_measures::{Distance, Euclidean}` API (the
  old `EuclideanDistance` trait is deprecated). Area via `unsigned_area()`, containment via
  `Relate`/`intersects`, overlap area via `BooleanOps::intersection`. Do NOT use geo's
  `buffer` (join/arc discretization): purge-line keepouts are plain rectangles, already
  available as `axis_aligned_line_keepout`.
- **Preview raster: `tiny-skia` 0.12** (replaces PIL). One `PathBuilder` path per footprint
  containing exterior + hole rings, filled with `FillRule::EvenOdd`; `Stroke { width }` for
  outlines and purge lines; RGBA `Pixmap`; `encode_png`. No `image`/`imageproc` needed.
- **`uuid5` via `sha1`**: SHA-1 of `NAMESPACE_URL` bytes (6ba7b811-9dad-11d1-80b4-00c04fd430c8)
  ‖ name, version nibble 5, variant bits `10`, lowercase hyphenated hex. Known vectors to
  assert (generated with CPython's `uuid.uuid5`, name = `goat4-contour-packing/{kind}/{index}`):
  - `resource/0` → `9f3bce4a-ccbe-5fb1-94d4-8283e284f863`
  - `resource/1` → `4d85a473-7e39-59be-8e91-25a8029d62d0`
  - `build-item/0` → `25c59a57-7f8d-5ec0-8e2a-9635308923ab`
  - `build-item/1` → `a5cc3fc8-946f-58c7-8783-29d59c72a9fe`
  - `component/0` → `214e7b7d-2a5e-5286-86a3-a67481ba1263`
  - `inline-resource/0` → `b241d998-6fcf-55ec-9fd9-c1d902c24d61`
  - `part-0/0` (name `part-{part_index}/{object_index}`) → `1354723b-e007-51c9-b19a-e01549a67497`
- **ZIP**: keep the existing `zip` crate; write with Deflate, compression level 6 to match the
  Python artifact's shape (level is cosmetic, order is not: preserve template entry order,
  append new entries last).
- **STL**: std-only (`to_le_bytes`); no crate.

## Numerical fidelity rules

- All transform math in f64, exactly as the Python does (same operation order where it feeds
  persisted values: `x − (cx·cos − cy·sin)`, `y − (cx·sin + cy·cos)`).
- Number formatting in generated XML/JSON mirrors `_number`: `|v| < 1e-10 → "0"`;
  near-integers (within 1e-10) print as integers; otherwise 10 decimals with trailing zeros
  and dot stripped.
- The footprint center used for compensation is the STL mesh's XY bbox center, computed from
  the same STL the packer collided against (`extract_footprint` already returns loops centered
  on exactly that point, but the center itself must be recovered from the STL vertices —
  `embedded_mesh` exposes it).
- `plate_1.json` uses compact JSON (serde_json `to_string`), same field set as the Python,
  including the cosmetic hardcoded `layer_height: 0.2` and name field.

## Acceptance criteria

1. `grep -rn "python3" src/` returns nothing; `build_3mf.py`, `extract_embedded_stl.py`,
   `verify_outputs.py`, `tests/test_build_3mf.py`, and `__pycache__/` are deleted.
2. `cargo build --release` and all fixture-free tests (`cargo test --release` excluding the
   `../Goat */`-dependent files) pass.
3. Rust equivalents of the two self-contained Python tests (component-transform baking,
   item-transform compensation with world-space center assertion) pass.
4. End-to-end on the real Goat 3 P1S source (`goat3_single_pla.3mf`) with
   `--attempts 2107 --stall 0 --seed 20270831`: packs 20 objects, probe and final BambuStudio
   slices succeed, tower stable, `paint_color` mentions preserved — i.e. identical outcome to
   the Python pipeline (determinism: the packed STL bytes and placement search are unchanged).
5. The new `verify_outputs` binary, run with the same arguments the Python verifier accepted on
   that output, passes all checks (min gap ≥ 0.5 with the raster-tolerance contract, purge gap,
   keepouts, transforms, cardinality).
6. Running the packer twice with identical arguments produces byte-identical output 3MFs.
7. The user's uncommitted scheduler WIP (`src/scheduler.rs`, `src/bin/daily_scheduler.rs`,
   `Cargo.toml` tempfile move, scheduler doc hunks) is untouched by every task; Cargo.toml only
   gains the four new dependencies.

## Risks

- quick-xml has no DOM; the event-span helpers (~100–200 lines) are the main new machinery.
  Mitigated by the synthetic fixture tests asserting exact output structure.
- geo vs GEOS produce non-bit-identical distances (1e-12 rel). Accepted: tolerances are 1e-6.
- PNG output differs from PIL pixel-wise (antialiasing). Accepted: previews are cosmetic.
- Fixture-dependent tests cannot run until `../Goat 3`/`../Goat 4` return; they follow the
  existing repo convention for such tests and must merely compile.

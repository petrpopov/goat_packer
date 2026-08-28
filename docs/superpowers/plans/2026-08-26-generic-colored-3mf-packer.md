# Generic Colored 3MF Packer Implementation Plan

> **For agentic workers:** Use subagent-driven-development or executing-plans only when delegation or a separate execution session adds value. For small work, implement directly. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generalize the contour packer so it can multiply a painted source 3MF safely around real prime-tower/brim geometry using an external printer profile, then produce packed Goat 3 projects for P1S and U1.

**Architecture:** Rust owns profiles, prime-tower G-code probing, packing constraints, and final verification; the existing 3MF assembly logic becomes source-object cloning that does not touch the colored mesh XML. Each run is profile-specific and preserves the source project settings. A probe/final slice feedback loop reserves the real prime tower rather than guessing its unknown length.

**Tech Stack:** Rust 1.97, existing `zip`/`xmltree`/`serde_json`, BambuStudio and OrcaSlicer CLIs, existing Python/Shapely verifier where suitable.

---

## Chunk 1: Generalized constraints and tower discovery

### Task 1: Define and validate printer profiles

**Files:**
- Create: `goat_packer/profiles/p1s.json`
- Create: `goat_packer/profiles/snapmaker_u1.json`
- Modify: `goat_packer/src/scheduler.rs`
- Create: `goat_packer/tests/colored_profile.rs`

- [ ] Write failing profile parsing tests for printer name, printable bounds, fixed keepouts, edge margin, purge lines, slicer kind/path.
- [ ] Implement profile structs and expand fixed geometry to raster keepouts.
- [ ] Run focused profile tests.

### Task 2: Probe prime-tower bounds from sliced G-code

**Files:**
- Create: `goat_packer/src/colored_packer.rs`
- Create: `goat_packer/tests/prime_tower.rs`

- [ ] Write failing fixtures for `WIPE_TOWER_*` marker blocks, brim blocks, absolute G90 and relative G91 coordinates, and extrusion-only filtering.
- [ ] Implement tower centerline bbox parsing and conservative expansion.
- [ ] Slice Goat 3 source projects and compare parsed P1S/U1 bboxes to expected tower/brim locations.

## Chunk 2: Colored cloning, packing and Goat 3 execution

### Task 3: Clone colored source object safely

**Files:**
- Modify: `goat_packer/src/scheduler.rs`
- Modify: `goat_packer/src/colored_packer.rs`
- Create: `goat_packer/tests/colored_clone.rs`

- [ ] Write failing test proving copies reuse the source mesh part unchanged and retain all `paint_color` triangle attributes.
- [ ] Implement generic root-resource/model-settings duplication from one source object.
- [ ] Assert project-settings byte identity and referential integrity.

### Task 4: Add iterative colored packing CLI

**Files:**
- Create: `goat_packer/src/bin/colored_3mf_packer.rs`
- Modify: `goat_packer/README.md`
- Create: `goat_packer/tests/colored_pack_loop.rs`

- [ ] Write failing loop test in which an updated tower bbox triggers one re-pack.
- [ ] Implement `--source-3mf`, `--stl`, `--profile`, `--output`, `--clearance`, and search controls.
- [ ] Implement slice-probe → pack → assemble → slice-final → verify, with a one-repack safety loop.
- [ ] Run all tests and strict clippy.

### Task 5: Produce Goat 3 deliverables

**Files:**
- Create: `Goat 3/p1s/goat3_p1s_colored_contour_packed.3mf`
- Create: `Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.3mf`
- Create: `Goat 3/*/*_verification.json`

- [ ] Run the generic tool for P1S and U1 source projects.
- [ ] Verify final tower/brim, purge line, edge margin, model gap, paint attributes, settings identity, archive XML, and CLI load/slice.
- [ ] Render previews and report counts/clearances.

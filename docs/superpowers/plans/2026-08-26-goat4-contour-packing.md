# Goat 4 Contour Packing Implementation Plan

> **For agentic workers:** Use subagent-driven-development or executing-plans only when delegation or a separate execution session adds value. For small work, implement directly. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and run a fast contour-aware nesting tool that creates one maximally populated Goat 4 plate for Bambu Lab P1S and one for Snapmaker U1 while preserving each supplied 3MF template's printer/process settings.

**Architecture:** A Rust binary reads the binary STL, extracts the flat-bottom boundary from z-min triangles, centers the footprint exactly like the embedded 3MF mesh, rasterizes many rotations, and performs parallel randomized contact-greedy/ruin-and-recreate nesting against printer-specific bed masks. It writes placements as JSON. A small Python assembler clones the template's shared mesh object, injects the optimized transforms and per-object metadata, updates plate metadata/previews, and repackages a standards-compliant 3MF without altering the embedded printer/process configuration.

**Tech Stack:** Rust 1.97 (`rayon`, `rand`, `serde`, `serde_json`), Python 3.10 plus Shapely/Trimesh/Pillow for independent geometric QA and previews, BambuStudio CLI for P1S validation, Snapmaker Orca CLI for U1 validation.

---

## Chunk 1: Geometry and packer

### Task 1: Scaffold and test STL footprint extraction

**Files:**
- Create: `goat_packer/Cargo.toml`
- Create: `goat_packer/src/lib.rs`
- Create: `goat_packer/src/main.rs`
- Create: `goat_packer/tests/footprint.rs`

- [ ] **Step 1: Write failing tests** for binary-STL bounds, flat-bottom boundary extraction, loop closure, and centering.
- [ ] **Step 2: Run `cargo test footprint -- --nocapture`** and confirm the missing APIs fail compilation.
- [ ] **Step 3: Implement the binary STL parser and quantized boundary-edge stitching, preserving outer and hole loops under an even-odd fill rule.**
- [ ] **Step 4: Run the focused test and confirm it passes.**
- [ ] **Step 5: Add a real-file test asserting Goat 4 dimensions, its outer/keyring-hole topology, and polygon area equal to the union area of all z-min triangles.**

### Task 2: Rasterize rotated, clearance-expanded masks

**Files:**
- Modify: `goat_packer/src/lib.rs`
- Create: `goat_packer/tests/raster.rs`

- [ ] **Step 1: Write failing tests** for polygon fill, rotation, Euclidean dilation, translated-bed containment, and exclusion-zone rejection.
- [ ] **Step 2: Run `cargo test raster -- --nocapture`** and confirm expected failure.
- [ ] **Step 3: Implement scanline rasterization and compact row-bitset masks.**
- [ ] **Step 4: Implement separate physical and clearance masks** at 0.25 mm raster resolution with conservative 0.35 mm half-clearance dilation (0.7 mm requested gap), so inter-model spacing is protected without unnecessarily shrinking usable bed edges.
- [ ] **Step 5: Run all Rust tests.**

### Task 3: Implement parallel contour-aware nesting

**Files:**
- Modify: `goat_packer/src/lib.rs`
- Modify: `goat_packer/src/main.rs`
- Create: `goat_packer/tests/packing.rs`

- [ ] **Step 1: Write failing synthetic packing tests** proving concave silhouettes can interlock even when bounding boxes overlap, shapes do not collide, and bed exclusions remain empty.
- [ ] **Step 2: Run the packing test and confirm expected failure.**
- [ ] **Step 3: Implement count-first contact-scored randomized greedy insertion** over 2° rotations and 0.25 mm translations with bit-parallel collision checks.
- [ ] **Step 4: Implement deterministic parallel restarts and ruin-and-recreate improvement** with a fixed time/iteration budget, convergence log, and JSON reporting; accept only layouts with at least 17 P1S models and retain the highest count before secondary contact/compactness scoring.
- [ ] **Step 5: Run `cargo test` and `cargo build --release`.**

## Chunk 2: 3MF assembly and independent verification

### Task 4: Assemble placements into printer templates

**Files:**
- Create: `goat_packer/build_3mf.py`
- Create: `goat_packer/tests/test_build_3mf.py`

- [ ] **Step 1: Write failing archive/XML tests** for object count, unique IDs/UUIDs, transforms, one-plate membership, retained printer model, and retained exclusion configuration.
- [ ] **Step 2: Run the Python tests and confirm expected failure.**
- [ ] **Step 3: Implement template cloning and ZIP repackaging** using the original shared mesh part and original project settings; synchronize resources/build items, relationships, object configs, and plate instances while preserving printer/extruder metadata. Update `cut_information.xml` and `plate_1.json` only when present in the source template (Bambu); do not invent them for Snapmaker.
- [ ] **Step 4: Generate every referenced preview** (`plate_1`, `plate_1_small`, `plate_no_light_1`, `top_1`, `pick_1`) from the actual placement geometry and embed them under the existing metadata paths.
- [ ] **Step 5: Run Python and Rust test suites.**

### Task 5: Optimize both printers and validate deliverables

**Files:**
- Create: `Goat 4/goat4_p1s_contour_packed.3mf`
- Create: `Goat 4/goat4_snapmaker_u1_contour_packed.3mf`
- Create: `Goat 4/goat4_p1s_placements.json`
- Create: `Goat 4/goat4_snapmaker_u1_placements.json`
- Create: `Goat 4/goat4_p1s_contour_packed_preview.png`
- Create: `Goat 4/goat4_snapmaker_u1_contour_packed_preview.png`

- [ ] **Step 1: Parse printable and exclusion polygons directly from each template, then run release optimization for P1S** on its 256×256 mm area with the 18×28 mm lower-left exclusion and 0.7 mm target clearance.
- [ ] **Step 2: Run release optimization for U1** on its actual `(0.5,1)`–`(270.5,271)` printable polygon with no exclusion area and the same clearance/search convergence requirements; require its final count to be at least the final P1S count.
- [ ] **Step 3: Independently validate in Shapely** that every transformed physical footprint is inside the template polygon, avoids exclusions, and every pair has at least 0.7 mm clearance within 0.05 mm tolerance; automatically increase dilation/refine and rerun if this fails.
- [ ] **Step 4: Validate each 3MF archive** (ZIP CRC, XML parsing, all metadata references, settings identity, object count, transform-to-JSON equality), and independently compose component/build transforms against embedded mesh vertices to verify matrix convention, XY center, rotation, and Z=3.7.
- [ ] **Step 5: Load and slice P1S with the installed BambuStudio CLI and U1 with the installed Snapmaker Orca CLI** and require exit code 0.
- [ ] **Step 6: Inspect both generated previews** and report final counts, minimum measured gaps, search settings, and absolute output paths.

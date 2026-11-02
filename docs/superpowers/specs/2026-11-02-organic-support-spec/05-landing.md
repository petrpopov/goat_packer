# Organic support spec set, part 5: the first-layer printed body (landing)

2026-11-02. Behaviour spec for the `landing` kind of `../2026-11-02-slicer-free-packer-design.md`
(cited as **D**; D §4.2, §6, Q4). It fixes how far the printed first layer of a normal object reaches
relative to the mesh, from the calibration corpus (`/tmp/goat_calib`, cited as **M**). Parts 01-04 cover
the supports and the tower.

**Licence rule** (D §1). This part describes behaviour in our own words: a rule, its constants and the
measurements behind them. Citations name upstream files only so that a reviewer can check a behaviour.

**Citation prefixes**, as in parts 01-04: **B** = BambuStudio 02.08.02.61, **O** = OrcaSlicer 2.4.2,
**R** = OrcaSlicer 2.5.0-rc, **S** = Snapmaker Orca 2.4.0. Paths are under `src/libslic3r/`.

---

## 1. What the landing is used for

`landing` is the material a normal object prints on layer 0, in the footprint frame of its rotation class.
The permission matrix (D §7.2-§7.3) uses it in three places:

- support base of another object vs this landing: forbidden, gap g (`SUPPORT_BODY_GAP_MM`);
- soft zones (purge and prime lines): forbidden, gap 0;
- the tower's L0 brim (`tower_brim`): forbidden.

It is **not** the mask that trims an object's own support base. That trim is the slicer's own uncompensated
layer-0 section grown by the first-layer gap with square joins (03 §4.2, D §5.2 B7). A larger trim mask would
under-predict the base.

Layer 0 material also belongs to band 0 (D §4.1 band rule), so `body[0]` contains the layer-0 body as well.
The landing is the precise first-layer test. `body[0]` is the coarser band test.

## 2. The rule

```text
S_0      = union over all normal parts of the section at z = h0 / 2            (01 §1.2 rule 4)
           with the closing, simplification and multi-part rules of 01 §1.3-§1.4,
           in the footprint frame (01 §1.1: centre = XY bbox centre of all normal parts)
landing  = S_0 ⊕ ε_L                      (round joins)
ε_L      = 0.05 mm                        wall_generator = classic
         = max(0.05, ½·(mbw − mfs)) mm    wall_generator = arachne (bound from the setting definitions, unmeasured)
```

- `h0` = `initial_layer_print_height`. The slice plane is the **middle** of layer 0 (P1S/A2L 0.10, U1 0.125),
  not the bed (z → 0) and not the layer top (z = h0). The goat's lowest walls flare outward by about 0.1 mm
  per 0.1 mm of height (table in §3.3), so either wrong plane moves the outline by about 0.1 mm.
- **Elephant-foot compensation is ignored.** The printed outline lies up to 0.15 mm inside S_0, so ignoring it
  is conservative (§3.2). It is never subtracted, because it is switched off locally on narrow features (§2.1).
- `mbw` = `min_bead_width`, `mfs` = `min_feature_size` of the wall generator, both percentages of the nozzle
  diameter (0.4 mm nozzle, 85 % / 25 %: ½·(0.34 − 0.10) = **0.12 mm**). Do not confuse this with the tree
  generator, which reads `min_feature_size` as millimetres (02 §0.4).
- The rule is rotation-equivariant: the printed first layer of the goat is the same at all 8 sampled rotations
  to 0.03 mm (§3.1). Unlike the supports (M §2.1), the landing of a rotation class is the rotated section.

### 2.1 Elephant-foot compensation: what the slicer does

B prints the compensation on layer 0 only. O/S/R print it on the first `elefant_foot_compensation_layers`
layers (U1: 1), tapering linearly. The value is 0.15 mm in all three projects (01 §1.5). The compensation is
**local** (O `ElephantFootCompensation.cpp:544-625`, the same code in B):

- m = width + spacing of the first-layer external-perimeter flow (here 0.50 + 0.45 ≈ 0.95 mm);
- at a contour point whose local feature width is d: no compensation if d < m; the full c = 0.15 mm if
  d > m + 2c; (d − m)/2 in between; the offsets are smoothed along the contour;
- an island whose bbox is narrower than m + 2c in X or Y, or whose area is below 5·(m + 2c)², is not
  compensated at all.

So narrow tips, thin legs and small islands print **at the section outline**, not 0.15 mm inside it. A
landing of `S_0 ⊖ 0.15` would be wrong exactly there (measured: 0.19 mm, §3.2).

### 2.2 Walls and tips

- **Classic walls** (all three owner projects: `wall_generator = classic`, `detect_thin_wall = 0`): the outer
  wall's material edge lies on the (compensated) contour, and gap fill stays inside it. Measured overshoot
  past S_0: at most 0.026 mm (§3.2), which is the 0.01 mm polygon simplification plus arc and
  polygonisation residue. ε_L = 0.05 mm covers it.
- **Arachne** (not in any owner project, so not measured): a feature thinner than `mfs` is not printed; a
  feature between `mfs` and `mbw` is printed with one bead widened to `mbw`, centred on the medial axis. The
  bead (and its round end cap) can pass the section by at most `mbw/2` minus the local half-width, so at most
  ½·(mbw − mfs). The report marks such a job `landing_measured: false` until corpus item L2 (§5) has run.
- The old figure "Arachne tip overshoot ≤ 1.66 mm" (D §4.2 before this part, PM §7 row 5) is the growth of the
  old probe *patch* over the mesh *silhouette* for support-free objects (`patch_overshoot_mm`). That measure
  mixes the flare of the first layers with the patch content. It is not a first-layer body overshoot, and this
  part supersedes it for the landing.

### 2.3 Unsupported settings for the landing

The model returns `Unsupported(<setting>)` (D §5.7) for: `xy_contour_compensation ≠ 0`,
`xy_hole_compensation ≠ 0`, `detect_thin_wall = 1` with classic walls (thin-wall widths unmeasured),
`fuzzy_skin ≠ none` when it applies to layer 0, `slicing_mode ≠ regular`, and raft layers > 0. The object brim
is a separate kind (03 §5) and does not enter the landing.

## 3. Evidence (corpus, all three printers, all rotations)

Method: `/tmp/landing05/measure.py` (scripts `section.py`, `measure_lib.py`, `detail.py`; output
`measure.json`). The mesh is the goat of each owner project (`projects/owner/<printer>_rot000.3mf`, all
normal parts, item transform applied, dropped to z = 0, item translation removed: object frame of M §1.2,
bbox ±18.843 × ±16.371). S_0 is the trimesh section at h0/2 (holes kept). The printed body is
`derived/owner/<printer>/rot*/run*.json` → `l1.body_polygon`: the union of every first-layer body line
buffered by half its comment width (round caps), rotated back into the object frame. Comment widths are
0.50 mm on the first-layer walls against flow widths of 0.49 mm, so the polygon errs on the large side.
Holes between lines are filled. Distances are sampled every 0.02 mm along the boundary.

### 3.1 Per printer (40 slices each: 8 rotations × 5 runs)

| printer | slicer | h0 / slice z | S_0 area (mm²) | printed area (mm²) | max outside S_0 (mm) | area outside S_0 | max outside S_0 ⊖ 0.15 (mm) | max inside S_0 (mm) |
|---|---|---|---|---|---|---|---|---|
| U1 | R 2.5.0-rc | 0.25 / 0.125 | 402.23 | 379.41-379.56 | **0.000** | 0 | 0.161-0.168 | 0.306 |
| P1S | B 02.08.02.61 | 0.20 / 0.10 | 399.88 | 376.98-377.17 | **0.020-0.026** | ≤ 0.005 mm² | 0.187-0.193 | 0.361-0.368 |
| A2L | B 02.08.02.61 | 0.20 / 0.10 | 399.88 | 377.01-377.11 | **0.020-0.026** | ≤ 0.005 mm² | 0.187-0.193 | 0.360-0.389 |

- The printed first layer is identical across the 5 runs of a rotation and differs across rotations by at most
  0.2 mm² of area and a few hundredths of a millimetre at the boundary. Body nondeterminism and rotation
  dependence are below the measurement resolution.
- The printed area is S_0 ⊖ 0.15 to within 1 mm² (377.79 mm² for P1S/A2L, 380.29 for U1). Elephant-foot
  compensation is applied on the long straight walls.
- The largest excess over S_0 ⊖ 0.15, 0.19 mm, sits at the same convex corner (object frame ≈ (13.76, −2.85),
  x-max side of the body) in every slice. There the compensation is locally reduced (§2.1), and the print
  reaches S_0 itself (0.020-0.026 mm past it).
- The largest shortfall (0.31-0.39 mm inside S_0) sits at two fixed spots of the body outline (object frame
  ≈ (9.62, −3.55) for B, (0.43, −3.09) for R): the compensation plus local wall geometry. It only makes the
  rule more conservative.

### 3.2 Consequences

- `landing = S_0 ⊕ 0.05` contains every printed first layer of the corpus with ≥ 0.024 mm to spare.
- `S_0 ⊖ 0.15` (applying the nominal compensation) would be **unsafe** by up to 0.19 mm.
- The rule over-reserves 0.15-0.39 mm per side. The goat's landing (perimeter 147 mm, holes included) is
  407.2 mm² (P1S/A2L) / 409.5 mm² (U1) against a printed 377-380 mm², about 30 mm² more. That is small next to
  the support base (≈ 650 mm² on L1, M §3).

### 3.3 Island structure and slice-plane sensitivity (goat)

S_0 has three islands: the body (340.3 mm² for P1S, 342.4 mm² for U1) and the two front feet (28.6-31.1 mm²
each). The printed layer has the same three islands. The silhouette (all z) is much larger than the landing:
±18.84 × ±16.37 mm against S_0 ≈ [−13.36, 14.16] × [−11.27, 15.11]. This is why support base vs foreign
landing, not the silhouette, binds the lattice (D §0).

| section z (mm) | 0.005 | 0.05 | 0.10 | 0.125 | 0.20 | 0.25 |
|---|---|---|---|---|---|---|
| S area (mm²) | 389.81 | 394.79 | 399.88 | 402.23 | 408.65 | 412.60 |
| x max (mm) | 14.071 | 14.114 | 14.158 | 14.181 | 14.247 | 14.292 |

## 4. Test hooks

- `tests/first_layer.rs` (WP I3, generated meshes): the slice plane is h0/2 (a cone whose radius changes with z
  gives the radius at h0/2); multi-part union; ε_L by wall generator; every unsupported setting of §2.3 returns
  `Unsupported` with its name.
- Calibration fixture (offline, WP I6): `l1.body_polygon` of every corpus slice lies inside the predicted landing
  (0 exceedance). The harness stores the per-slice maximum excess in `docs/calibration/landing.csv`.

## 5. Open items

- **L1. Per-band body.** `derive.py` keeps the body polygon of layer 1 only. The relation between the printed
  body and the mesh slab per band (`body[k]`, walls inside the outline, overhang sag) is still unvalidated
  (review §2). Extend `derive.py` to body polygons per band (no new slices needed).
- **L2. Arachne.** Slice the goat and one thin-tip synthetic shape with `wall_generator = arachne` in B and R,
  and measure the excess over S_0. That replaces the setting-derived ½·(mbw − mfs) bound.
- **L3. Snapmaker Orca 2.4.0.** No S first layer is in the corpus except inside the owner's GUI plates. Check
  one plate object against S_0 with the same script (the full-plate files hold the L1 support but not the body;
  re-run the extraction with body lines on layer 1).

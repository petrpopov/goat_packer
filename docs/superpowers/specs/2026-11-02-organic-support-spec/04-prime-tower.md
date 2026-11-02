# Prime tower footprint without slicing (part 4)

Date: 2026-11-02. Status: functional spec for the slicer-free envelope predictor. It describes what the
slicers do, in our own words, and how well that description reproduces real G-code. It does not
prescribe our implementation. No slicer source is copied or transliterated here; formulas and the
pseudo-code are written at the algorithm level, and `file:line` references let an implementer check the
behaviour.

Scope: the prime (wipe) tower of a by-layer, multi-colour plate: where its body, brim and chamfer print
on every layer, how big it gets, what moves it, and what is (not) deterministic. Supports, object brims
and the bed/exclusion rules are covered by parts 1-3 and by `/tmp/goat_research/zones-research.md`.

## 0. Conventions

**Versions and citation prefixes.** Paths are relative to `src/libslic3r/` unless prefixed `GUI:`
(`src/slic3r/GUI/`) or `CLI:` (`src/`).

| prefix | checkout in `/tmp/goat_src` | tower class | who uses it |
|---|---|---|---|
| B | `zones_bambu2861` (BambuStudio tag v02.08.02.61, same tag as `bambu_0208`, which lacks `GCode/`) | `WipeTower` (BBL, rib wall) | owner's P1S/A2L GUI, server P1S/A2L |
| O | `orca_242` (OrcaSlicer v2.4.2) | `WipeTower2` ("Type 2") | server U1 (`deploy/slicers.env:8`) |
| R | `orca_250rc` (OrcaSlicer v2.5.0-rc) | `WipeTower2` | local `/Applications/OrcaSlicer.app`, all local U1 slices |
| S | `snaporca240` (Snapmaker Orca v2.4.0) | `WipeTower2` (own fork) | owner's U1 GUI (the G-code that is printed) |

The tags B/O/R/S are the same in every part of this spec set (R was "O25" in the first draft of this part).

Orca picks its tower class from the printer: Bambu printers always get the BBL class ("Type 1"), every
other printer gets `wipe_tower_type` (default Type 2) (R `Print.hpp:1347`, O `Print.hpp:1086`); S always
uses Type 2 for non-Bambu printers (S `Print.cpp:3162`). The owner slices P1S/A2L only in BambuStudio,
so Orca's Type 1 port is out of scope (§10, Q6).

**Symbols.**

| symbol | meaning |
|---|---|
| `d` | nozzle diameter (0.4 on all three printers) |
| `pw` | tower line width = 1.25·`d` = 0.5 mm. Fixed ratio inside the tower code (B `GCode/WipeTower.hpp:420`); the tower ignores `line_width` settings |
| `h_l` | height of tower layer `l` (first layer `initial_layer_print_height`, then `layer_height`; per-object layer-height ranges change it) |
| `A(h)` | effective cross-section of a `pw`-wide bead of height `h`: `A = h·(pw − h·(1 − π/4))`. Volume `V` becomes line length `V / A(h)` (B `GCode/WipeTower.hpp:539-546`; same helper in all builds) |
| `s_l` | brim loop pitch on layer `l`: `s = pw − h_l·(1 − π/4)` (0.4571 at 0.20 mm, 0.4464 at 0.25 mm, 0.4399 at 0.28 mm) |
| `W0` | configured tower width `prime_tower_width` |
| `W, D` | width (x) and depth (y) of the tower body actually printed |
| `H_t` | z of the tower top (last tower layer) |
| `(X, Y)` | `wipe_tower_x[plate]`, `wipe_tower_y[plate]` from `project_settings.config` |
| `k_l` | number of tool changes planned on layer `l` |
| `minD(H)` | "minimum depth by height": linear interpolation of the table `{5→5, 100→20, 250→40, 350→60}` mm, clamped at both ends (B `GCode/WipeTower.cpp:1586-1625`) |

**Frames.** Every tower in this spec is in *plate* coordinates. Bambu writes P1S G-code as plate minus
`extruder_offset` (0, 2) (`src/colored_packer/` already handles it); A2L and U1 have zero offset. The
measurements below are converted to plate coordinates.

## 1. Results at a glance

Body = outer wall centre line; brim = outermost brim loop centre line. Add half a line width (0.25 mm)
for the material edge. All 147 G-codes available locally were measured with an arc-aware parser (G2/G3
sampled every 0.05 mm, §8).

| printer / slicer | body W × D (pred.) | body measured | L0 incl. brim (pred.) | L0 measured | chamfer | slices |
|---|---|---|---|---|---|---|
| P1S / B | 18.488 × 17.688 (rib wall, box 15 × 14) | 18.489 × 17.638 | 24.889 × 24.088 | 24.886 × 24.029 | 5 layers, 3 mm max | 62 |
| A2L / B | 28.579 × 28.579 (rib wall, box 24.5 × 24.5) | 28.578 × 28.578 | 34.983 × 34.983 | 34.967 × 34.967 | 5 layers | 10 |
| U1 / R | 20 × 14.9 | 20.000 × 14.900 | 29.820 × 24.720 | 29.820 × 24.720 | none | 71 |
| U1 / O (server) | 20 × 9.5 | 20.000 × 9.500 | 29.820 × 19.320 | 29.820 × 19.320 | none | 1 (docker) |
| U1 / S (owner GUI), W0 = 20 | 20 × 16.1 | 20.000 × 16.100 | 29.820 × 25.920 | 29.820 × 25.920 | 8 layers, 4 mm max | 2 |
| U1 / S (owner GUI), W0 = 16 | 16 × 19.7 | 16.000 × 19.700 | 25.820 × 29.520 | 25.820 × 29.520 | 8 layers | 2 |

Residuals: rectangular (Type 2) towers are exact to the printed 0.001 mm. Bambu rib towers are within
0.093 mm on every layer; the prediction is the larger one except at the anchor corner, where the printed
wall reaches up to 0.052 mm past `(X, Y)` (§8.3). A 0.1 mm tolerance on top of the half line width
covers every measured layer.

Key behaviours, each verified on real G-code:

1. **Object count does not change the tower** for these projects. Probe (1 goat), sample (3) and full
   plates (40-49 goats) produce the same tower extents (to 0.001 mm) in every slicer (§8.2). Reason: Bambu sizes
   the tower from prime volumes only, and Type 2 on a non-SEMM printer from `prime_volume` only; neither
   depends on purge-into-object volume.
2. **The tower depends on the plate's worst layer:** the most tool changes on one layer (`k_max`) and the
   z of the last layer with a tool change (`H_t`). Both follow from the painted model and
   `layer_config_ranges`. A slicer-free mesh pass recovers them (§4.1, validated in §8.4).
3. **The three U1 slicers print three different towers from the same project:** 9.5 (server Orca
   2.4.2), 14.9 (local Orca 2.5-rc) and 16.1 mm deep (owner's Snapmaker 2.4.0) at `W0` = 20. The server's
   final-slice check therefore cannot see a collision with the tower that the owner's GUI actually prints
   (§10, Q1).
4. **Tree-support nondeterminism does not reach the tower** while supports use filament 0 ("any"), which
   is the case in all three projects (§7).

## 2. Inputs

From `project_settings.config` (names differ between Bambu and Orca; "B / Type 2" columns):

| purpose | Bambu (B) | Type 2 (O, R, S) |
|---|---|---|
| enable | `enable_prime_tower` | same |
| anchor per plate | `wipe_tower_x/y[plate]` | same |
| rotation | `wipe_tower_rotation_angle` | same |
| width | `prime_tower_width` (rib: start value only, §4.3) | `prime_tower_width` |
| purge per change | `filament_prime_volume[new filament]`; 15 for every filament when `prime_volume_mode = Saving` (B `Print.cpp:3819-3824`) | `prime_volume` (global). With `purge_in_prime_tower` and `single_extruder_multi_material` both on: `flush_volumes_matrix` · `flush_multiplier`, minus what is wiped into objects (R `Print.cpp:5504-5550`) |
| floor of the saved change | n/a | `filament_minimal_purge_on_wipe_tower` (O, R only) |
| row spacing | `prime_tower_infill_gap` (%) | `wipe_tower_extra_spacing` (%), `wipe_tower_extra_flow` (%) |
| wall shape | `prime_tower_rib_wall`, `prime_tower_rib_width`, `prime_tower_extra_rib_length`, `prime_tower_fillet_wall` | `wipe_tower_wall_type` (rectangle/cone/rib), `wipe_tower_rib_width`, `wipe_tower_extra_rib_length`, `wipe_tower_fillet_wall`, `wipe_tower_cone_angle` |
| wall gaps | `prime_tower_skip_points` | `prime_tower_skip_points` (R only uses it for planning, §4.4) |
| brim | `prime_tower_brim_width` (−1 = auto) | same; S has no auto (minimum 0, S `PrintConfig.cpp:6089-6095`) |
| chamfer | always on, 3 mm (B `GCode/WipeTower.cpp:3706-3739`) | S only: `prime_tower_brim_chamfer` (default on), `prime_tower_brim_chamfer_max_width` (default 4); both hidden "develop" options (S `PrintConfig.cpp:6097-6110`) |
| multi-tool ramming | n/a | `filament_multitool_ramming`, `_volume`, `_flow`; `enable_filament_ramming` (O gates the reserved depth on it, §4.4) |
| tower everywhere | `timelapse_type = smooth`, `enable_wrapping_detection` | `timelapse_type = smooth` |
| other | `wipe_tower_no_sparse_layers`, `prime_tower_enable_framework`, `filament_adhesiveness_category` | `wipe_tower_no_sparse_layers`, `wipe_tower_filament` |

The three owner projects (`/tmp/goat_research/proj/*.json`):

| | U1 | P1S | A2L |
|---|---|---|---|
| filaments in project / used by the goat | 4 / 3 (0, 1, 2 zero-based) | 3 / 3 | 4 / 3 |
| layer height, first layer | 0.28, 0.25 | 0.28, 0.20 | 0.28, 0.20 |
| purge per change | `prime_volume` 20; minimal purge 15 | Saving → 15 | Default → 45, 45, 30, 30 |
| width / wall | 20, rectangle | 20, rib (width 8, extra 0, fillet) | 35, rib (width 8, extra 0, fillet) |
| spacing | extra_spacing 120 %, extra_flow 100 % | infill_gap 150 % | infill_gap 150 % |
| brim | 5, chamfer on, max 4 | 3 | −1 (auto) |
| multi-tool ramming | on, 5 mm³ at 25 mm³/s; `enable_filament_ramming` 0 | n/a | n/a |
| skip points | not in the 2.4.0 project; slicer default 1 | 1 | 1 |
| anchor | (244.5, 245) | (221, 207.25) | (287.385, 15.0) |
| support filament | 0 (any) | 0 | 0 |

From the model: the per-part extruder (`model_settings.config`), the per-triangle `paint_color`, the
object's `layer_config_ranges` (`Metadata/layer_config_ranges.xml`, extruder and layer-height overrides
by z band), height-range modifiers, and the object height.

## 3. Pipeline overview

```
painted model + layer ranges ──► A. filament set S_l per layer ──► B. tool changes k_l per layer, H_t
settings ──► C. depth per tool change (per slicer) ──► D. tower W, D, per-layer depth profile
          ──► E. wall outline per layer (rectangle / rib / cone) ──► F. brim + chamfer loops per layer
          ──► G. placement (anchor, rib offset, rotation, slicer-side clamps) ──► per-layer footprint
```

All steps are deterministic functions of the project (§7 lists the exceptions).

## 4. The algorithm

### 4.1 A. Filament set per layer

The slicer's tool ordering collects, per print layer, every filament that has an extrusion on it
(B `GCode/ToolOrdering.cpp:705-946`): wall and infill filaments of every region, painted (MMU-segmented)
regions with their painted filament, support and interface filament when non-zero and support exists on
that layer, and per-z extruder overrides from layer ranges. Supports, ironing and similar features with
filament 0 are "don't care" and never add a filament.

Slicer-free rule (ours), conservative:

1. Slice planes: `z_l` = cumulative layer heights; the plane of layer `l` is the middle of
   `[z_{l−1}, z_l]`.
2. For every triangle, decode its paint tree into the **set of states** it contains (decoder below) and
   take its z-extent in plate frame. State 0 means "unpainted": the part's extruder, replaced by the
   layer-range extruder where a `layer_config_ranges` band covers the plane. State `k ≥ 1` means filament
   `k`.
3. `S_l` = union of the states of all triangles whose z-extent contains the plane of layer `l`, mapped
   to filaments as in step 2, over all parts of all objects.
4. **Dilate by one layer:** `S'_l = S_{l−1} ∪ S_l ∪ S_{l+1}`. This covers the ±1-layer disagreement at
   band edges caused by MMU segmentation depth and range boundaries (§8.4).

Paint tree format, in our words: the attribute is a hex string; read it from the **last** character to
the first, one nibble per character. Pre-order walk: a nibble whose two low bits are non-zero is a split
node (the low bits give the number of split sides, the children follow); otherwise it is a leaf whose
state is the upper two bits, unless both upper bits are set, in which case the state is
`3 + 15·(number of following 0xF nibbles) + (next nibble)`. For a set of states the tree shape does not
matter: walk the nibbles linearly and collect leaf states (B `TriangleSelector.cpp:2076-2140`).

Taking a whole triangle's z-extent for every state in its tree over-reports colours at the edges of
painted features. That errs on the safe side.

### 4.2 B. Tool changes per layer and tower height

Per layer the slicer orders `S_l` so that the filament left loaded by the previous layer normally goes
first when it is present (Bambu: flush-minimising order over the layer's filaments, B
`GCode/ToolOrdering.cpp:2714-2829`; S: explicit rotation to start with the current filament, S
`Print.cpp:3379`; O/R: same ToolOrdering heritage). Hence

```
k_l = |S_l| − 1   if the previous layer's last filament is in S_l, or l is the first layer
k_l = |S_l|       otherwise
```

Type 2 also plans a no-op "change" on the first layer: the priming order puts the first printing filament
last in `all_extruders`, so the first layer's first filament never counts (S
`GCode/ToolOrdering.cpp:1100-1130`).

The last filament of a layer depends on the ordering heuristic. Our conservative form uses the dilated
sets: `k_l = |S'_l| − 1` if `S'_{l−1} ⊆ S'_l`, else `|S'_l|`.

Tower layers (B `GCode/ToolOrdering.cpp:948-967`; same in all builds): a layer has a tower iff some layer
at or above it has `k > 0`. The "partitions" count propagates downwards. Therefore

```
H_t = z of the highest layer with k_l > 0.
```

The tower is printed on every layer from the first up to `H_t`, including layers without a change; those
print only the wall and sparse infill. Exceptions:
- `timelapse_type = smooth` or `enable_wrapping_detection` (Bambu): tower on every object layer, minimum
  depth (B `GCode/WipeTower.cpp:4282-4309`).
- `wipe_tower_no_sparse_layers`: layers without changes are skipped and the tower is compacted in z. The
  footprint is unchanged, the z-schedule is not. Off in all three projects.
- **O only:** when the tower ends below the last print layer, a "final purge" is still printed at the
  last print layer's z (measured: a 18.5 × 1.2 mm patch at z = 44.77 inside the body rectangle, mid-air).
  R skips it (R `Print.cpp:5590-5597`). S emits the final change there with travel only and no
  extrusion (measured). B prints nothing there (measured).

### 4.3 C/D. Depth: Bambu 2.8.2.61 (rib wall, single nozzle)

Depth is reserved per tool change from the **prime** volume of the incoming filament. Flush volumes go to
the purge chute or into objects and never into the tower (B `Print.cpp:3785-3843`, B
`GCode/WipeTower.cpp:2902-2960`).

Per change into filament `f` on layer `l`, with current width `W`:

```
L     = V_f / A(h_l)                                     purge line length
rows  = ceil( L / (W − 2·pw) )
depth = rows · g          g = row pitch = (infill_gap/100)·pw   (0.75 mm at 150 %)
```

The row pitch comes from the block-gap computation (B `GCode/WipeTower.cpp:4438-4518`). With one nozzle
there is no ramming, so a block's gap is `pw + (gap − 1)·pw`. After this step the code resets its
spacing factor to 1.

Each adhesion category (`filament_adhesiveness_category`; all PLA here = one category) gets a **block**.
A block's depth on layer `l` is the sum of the depths of the changes into that category on `l`. It is
made non-increasing with height: each layer takes the max of itself and the layer above. The block's
depth is its layer-0 value. Blocks are stacked in y (B `GCode/WipeTower.cpp:4311-4436`).

Width and depth (B `GCode/WipeTower.cpp:4520-4609`):

1. With `W = W0`, compute every change's depth and `B0 = Σ_blocks block depth`.
2. **Square it off:** `W = ceil_to_multiple_of_pw( sqrt( (B0 + pw) · W0 ) )`.
3. Recompute every change's depth with the new `W` and get `B = Σ blocks`.
4. `D = B + pw`. Every layer's own depth `D_l = (its non-increasing block sum) + pw`. A layer whose
   block sum is below `pw` is not printed (B `GCode/WipeTower.cpp:4694`).
5. **Rib length:** `L_r = max(diag, rib extension) + extra_rib_length`, never below `diag = sqrt(W² + D²)`.
   If `B < minD(H_t)`, the ribs (not the box) grow to at least `minD(H_t)·√2`.
6. **Rib width:** `r = min(prime_tower_rib_width, min(W, D)/2)`.

`H_t` here is the tower's own top (m_plan's last z, B `GCode/WipeTower.cpp:4672`), not the object height.

Growth with tool changes (all changes on a layer into the same volume, one category):
`B ≈ k_max · rows(W) · g`, where `W` itself grows roughly with `sqrt(k_max)`. Predicted with §8 tools:

| | k_max = 1 | 2 | 3 |
|---|---|---|---|
| P1S box W × D / wall bbox | 12.5 × 11.75 / 15.44 × 14.87 | 15 × 14 / 18.49 × 17.69 | 18.5 × 16.25 / 22.40 × 20.65 |
| A2L box / wall bbox | 20.5 × 20.0 / 24.55 × 24.15 | 24.5 × 24.5 / 28.58 × 28.58 | 28.5 × 28.25 / 32.57 × 32.35 |

With `k_max = 1` the first layer (0.20 mm, more rows) dominates. The first layer's own height always
takes part in the max.

Not in the owner projects, but in the same code: dual-nozzle/H2-series ramming adds a "nozzle change"
depth (B `GCode/WipeTower.cpp:2949-2958`). Without the rib wall the tower keeps `W0`, and when
`B < minD(H_t)` it spreads the rows by `minD/B` instead of growing ribs. `prime_tower_enable_framework`
freezes every layer at layer-0 depth.

### 4.4 C/D. Depth: Type 2 (O, R, S)

Common to all three: a non-SEMM multi-tool printer purges `V = prime_volume` per change. Every non-zero
flush entry is replaced by `prime_volume` (R `Print.cpp:5504-5513`; S, O alike). The planned depth of
one change is a **ramming band** plus a **wipe band**, and the layer's depth is the sum over its changes.
The tower depth is `D = max_l(depth_l) + pw`, where the per-layer depths are made non-increasing with
height (each layer takes at least the depth of the layers above it; R `GCode/WipeTower2.cpp:2316-2339`).
`W = W0` for rectangle and cone walls.

Notation: ramming line width `rlw = 2·pw` (multi-tool), ramming step 1, ramming spacing
`es_r = extra_spacing`, wipe spacing `es_w = extra_spacing·extra_flow`, `ef = extra_flow`.

A wipe band of volume `V'` across draw width `w` takes `rows_w = floor( (V'/A(h)) / ef / w ) + 1` rows at
pitch `pw · spacing`.

| step | R (2.5.0-rc) | O (2.4.2, server) | S (Snapmaker 2.4.0, owner) |
|---|---|---|---|
| draw width `w` | `W − 3·pw` | `W − 3·pw` | `W − pw − 2·rlw` for ramming *and* wipe planning |
| ramming volume | `multitool_ramming_volume` | `0.25 s · multitool_ramming_flow` (6.25 mm³ for 25 mm³/s) | `multitool_ramming_volume` |
| ramming length `Lr` | `vol / A_rlw(h)` (A with `rlw` instead of `pw`) | same | same |
| ramming band | `(floor(Lr/w)+1)·rlw·es_r` when ramming happens | same, but **only if `enable_filament_ramming`**; the multi-tool ram is still printed, just not reserved | `max(1, ceil(Lr/w))·rlw·es_r` when any ramming is on |
| first wipe line credit (a length) | `w − (Lr mod w)` only when ramming happens and the wall has no gap points; with multi-tool ramming + skip points (the U1 case) or without ramming: 0 | `w − (Lr mod w)` always | `w − (Lr mod w)` always |
| wipe volume `V'` | `V − credit·A'(h)`, `A'` = bead area at width `pw·ef` | same | same |
| wipe pitch | `pw·ef` on the first tower layer, `pw·es_w` above | same | `pw·es_w` on every layer |
| "save on last wipe" | yes: the layer's first change into a wall filament is re-planned with `max(minimal_purge, V − finish-layer volume)` (finish layer = sparse infill + wall), 5 passes | yes | **no** (S `GCode/WipeTower2.cpp:2713-2740` never calls it) |
| rib wall | squared like Bambu: `W = ceil_pw(sqrt(maxdepth·W0))`, replanned; ribs grow to `minD(H_t)·√2` if short | no squaring | squared (`sqrt(D·W0)`), replanned |
| sources | `GCode/WipeTower2.cpp:2251-2313, 2341-2395, 2470-2509`; `.hpp:323-327` | `GCode/WipeTower2.cpp:2203-2251, 2279-2325, 2364-2380` | `GCode/WipeTower2.cpp:2427-2470, 2928-2951, 2713-2735` |

In practice the finish-layer volume always exceeds `V − minimal_purge` (5 mm³ here, about 40 mm of
wall), so for O/R the first wall-filament change of each layer is planned with exactly
`minimal_purge`.

U1 worked numbers (h = 0.28, `W0` = 20, V = 20, minimal purge 15):

| | change 1 | change 2 | `k_max`=2 → D |
|---|---|---|---|
| R | ram 2.4 + wipe 7 rows·0.6 = 6.6 | ram 2.4 + 9 rows·0.6 = 7.8 | 14.4 + 0.5 = **14.9** |
| O | ram 0 + 6 rows·0.6 = 3.6 | 9 rows·0.6 = 5.4 | 9.0 + 0.5 = **9.5** |
| S | ram 2.4 + 9 rows·0.6 = 7.8 | 7.8 | 15.6 + 0.5 = **16.1** |
| S, `W0` = 16 | 2.4 + 12·0.6 = 9.6 | 9.6 | 19.2 + 0.5 = **19.7** |

Growth with `k_max` at `W0` = 20 (first layer 0.25 mm with one change included):

| | k=1 | 2 | 3 |
|---|---|---|---|
| R | 7.1 | 14.9 | 22.7 |
| O | 4.1 | 9.5 | 14.9 |
| S | 8.9 (first layer dominates) | 16.1 | 23.9 |

Type 2 has no height-based minimum depth for rectangle walls in any of the three builds. The measured O
tower is 9.5 deep, below `minD(41.4)` = 10.7. Only the GUI estimates apply `minD` (§6).

**SEMM with purge-in-tower** (not used here): the volume per change is the flush-matrix entry times the
multiplier minus what `mark_wiping_extrusions` puts into infill, objects or supports, plus the minimal
purge. That part **depends on object count and object infill volume**: more objects mean a smaller tower
when any `flush_into_*` is on.

### 4.5 E. Wall outline per layer

**Type 2 rectangle** (U1): the wall centre line of layer `l` is the axis-aligned rectangle
`[0, W] × [y_l, y_l + D_l]` in tower-local coordinates, with `D_l = depth_l + pw` and
`y_l = (D − D_l)/2` (centred once the layer is shallower than the tower; R
`GCode/WipeTower2.cpp:2545-2546`). The content rotates 180° every layer
(`m_internal_rotation += 180`), but the outline does not move.

Measured example (S, 48-goat plate, `W0` = 16): full depth 19.7 up to z = 29.93, the last layer with two
changes; 10.1 deep and centred from z = 30.21 to `H_t` = 41.69.

**Bambu rib wall** (B `GCode/WipeTower.cpp:5041-5122`, rounding at `:128-186`):

1. Box `[0, W] × [0, D_l]` (layer depth incl. `pw`, centred by `y_shift` the same way; the shift is
   rounded to multiples of `pw`, `:4695-4700`).
2. Two strips of half-width `r/2` along the full-tower diagonals (0,0)→(W,D) and (W,0)→(0,D), with
   square ends. They stay on the full `W × D` tower even when the box is shallower. Each strip is
   lengthened at both ends by `e(z) = max(0, L_r − diag)/2 · (H_t − z)/H_t`: the rib extension shrinks
   linearly to zero at the tower top.
3. Outline = union of box and strips.
4. With `prime_tower_fillet_wall`, every outline corner whose turn angle lies between 30° and 150° is
   replaced by a tangent circular arc with tangent length `min(2 mm, adjacent edge length / 2.1)`
   (sampled at 20 points). The result is united with the box again.
5. That polygon is the wall centre line. With `prime_tower_skip_points` the wall is cut open at each
   tool change's entry point (gaps of 2.5·`pw`). The gaps do not change the union footprint.

For the P1S/A2L goats `L_r = diag` (no extension), so the outline is the same on every layer at full
depth. The wall then sticks out of the box by about 1.74-1.84 mm (P1S) or 2.04 mm (A2L) per side at the
rounded rib ends.

Arc fitting (P1S on, A2L off) re-emits the outline as G2/G3 within 0.1 mm (B `GCode/WipeTower.cpp:20-23,
1030-1040, 1272-1295`). The measured residual is ≤ 0.06 mm. **The repo's G1-only tower parser sees
16.3 × 14.6 of this 24.9 × 24.0 first layer** (`/tmp/goat_research/pipeline-map.md` §4).

**Type 2 rib wall** (not used here): same construction as Bambu. R squares the tower and applies the
rib offset (below). S squares it but applies no rib offset. O neither squares nor offsets, so its ribs
reach into negative local coordinates, i.e. left of and below `(X, Y)`.

**Type 2 cone** (`wipe_tower_wall_type = cone` only, in all three builds): the first layer's outline is
the box united with an ellipse centred on the box. Its y-radius is `Rc = tan(θ/2)·H_t`; its x-radius is
`Rc/σ`, where `σ ≥ 1` widens a too-narrow base (R `GCode/WipeTower2.cpp:2153-2185`). On layer `l` the
radius is `tan(θ/2)·(H_t − z_l)`.

### 4.6 F. Brim and chamfer

Brim loops are successive outward offsets of the layer-0 wall outline by `s_0`, with mitred corners
(mitre limit 3). Rectangles stay rectangles; rib outlines keep their notches.

```
B      = prime_tower_brim_width, or if −1 (B, O, R): min(8, 8·H_t/100)   (B GCode/WipeTower.cpp:1627-1630)
n_0    = floor( (B + s_0/2) / s_0 )          loops on the first layer
reach  = n_0 · s_0                           outermost loop centre line beyond the wall centre line
```

Auto brim uses the tower top `H_t` in the slicer (A2L: 41.36 → 3.31 mm → 7 loops) but the object
height in the GUI estimate (44.74 → 3.58 → 8 loops). The measured A2L has 7 loops.

Chamfer, on tower layer `j ≥ 1` counted from the first tower layer, only while that layer's depth still
equals the first layer's depth:

```
n_j = max(0, min( floor((B + s_j/2)/s_j), floor(C / s_j) ) − j)
```

- B: always on, `C = 3` mm (B `GCode/WipeTower.cpp:3706-3739`). At 0.28 mm: 5, 4, 3, 2, 1 loops on
  layers 1-5, nothing from layer 6 (z ≈ 1.88).
- S: only if `prime_tower_brim_chamfer`, `C = prime_tower_brim_chamfer_max_width` (U1: 4). At 0.28 mm:
  8 … 1 loops on layers 1-8 (z 0.53-2.49), nothing from layer 9 (S `GCode/WipeTower2.cpp:2303-2338`).
  S prints no brim at all when the width is 0. Its auto value is impossible (minimum 0).
- O, R: no chamfer; brim on the first layer only (R `GCode/WipeTower2.cpp:2102-2131`, O `:2112-2140`).

### 4.7 G. Placement

Tower-local point `p` → plate:

| build | transform | layer-0 anchoring |
|---|---|---|
| B | `Rot(α)·p + (X, Y) + o`, with `o = −min(bbox of the layer-0 wall polyline)` taken unrotated (B `GCode.cpp:740-746`, `GCode/WipeTower.cpp:5117-5120`) | at α = 0 the wall bbox min corner lands on `(X, Y)`. Rectangle walls: `o` = 0 |
| R | `Rot(α)·(p + o) + (X, Y)`; `o` only for rib walls (R `GCode.cpp:1006`, `GCode/WipeTower2.cpp:2680-2686`) | rectangle: box corner at `(X, Y)` |
| O, S | `Rot(α)·p + (X, Y)`, no `o` (O `GCode.cpp:1132-1136`, S `GCode.cpp:444-448`) | box corner at `(X, Y)` |

`(X, Y)` is the value *after* slicer-side clamps (§6), which can differ from the 3MF.

Bambu computes `o` from the layer-0 wall polyline after the skip-point gaps are removed. When a gap
removes the extreme vertex on layer 0, other layers reach past `(X, Y)`. Measured: up to 0.031 mm (P1S)
and 0.052 mm (A2L) in x.

### 4.8 Per-layer footprint (what the packer consumes)

For each tower layer `l` with top `z_l`, in plate coordinates, before adding the half line width:

```
Body_l   = wall outline of layer l (§4.5), placed (§4.7)
Ring_l   = offset(Body_0 outline, n_l·s_l) minus Body_l interior, n_l from §4.6 (n_0 on the first layer)
Tower_l  = Body_l ∪ Ring_l           for z_l ≤ H_t (O: plus the mid-air final purge at the print top, inside Body)
```

Conservative and simple: `Body_0` (the full-depth, full-rib outline) for every `z ≤ H_t`, plus the
rings on their layers.

## 5. Differences between builds (summary)

| behaviour | B | O | R | S |
|---|---|---|---|---|
| tower class / wall | BBL, rib (P1S/A2L) | Type 2 | Type 2 | Type 2 |
| purge sizing | prime volume per new filament, chute takes flush | `prime_volume` | `prime_volume` | `prime_volume` |
| ramming reserved (U1) | n/a | no (`enable_filament_ramming` 0) | yes | yes |
| finish-layer saving | n/a | yes | yes | no |
| first-layer wipe pitch | n/a (solid, pitch = gap) | `pw·ef` | `pw·ef` | `pw·es_w` |
| rib squaring / rib offset | yes / yes | no / no | yes / yes | yes / no |
| min depth by height | ribs (rib) or spread rows | none | ribs only | none |
| brim auto | yes (tower height) | yes | yes | no |
| chamfer | always, 3 mm | no | no | option, 4 mm |
| mid-air final purge | no | yes (prints) | skipped | travel only |
| U1 measured depth (`W0`=20) | — | 9.5 | 14.9 | 16.1 |
| CLI moves stored anchor | no | no | **yes, every slice** (§6) | (CLI segfaults, not used) |
| GUI re-clamp on open | yes, 15 mm, estimate | no | yes | yes, 1 mm + brim, every scene reload |

## 6. Slicer-side clamps: can the tower move?

These rules decide which `(X, Y)` actually prints. Our output must already satisfy them, or the tower the
packer reserved is not the tower that prints.

**B (BambuStudio 2.8.2.61).** When a project is opened, every plate's stored position is re-clamped
(GUI: `Plater.cpp:8930-8937` → `PartPlate.cpp:4808-4895`):

```
x ← clamp(x, plate_min_x + 15, plate_max_x − Ŝx − 15)
y ← clamp(y, 15,               plate_max_y − Ŝy − 15)      (brim not included)
```

`Ŝ` is the GUI estimate (GUI: `PartPlate.cpp:2191-2262`), not the sliced size. Rib wall:
`Ŝx = Ŝy = r̂/√2 + max(d̂ + extra_rib, d_v)` with `d_v = sqrt(V̂·(n−1)/layer_height · gap)`,
`d̂ = max(minD(object height), d_v)` and `r̂ = min(rib_width, d̂/2)`. Here `V̂` = max prime volume over
the project's filaments (15 in Saving) and `n` = filaments used by the plate. The CLI never moves the
tower. After slicing the GUI does not move it either (`GLCanvas3D.cpp:3880-3950` computes an offset but
does not apply it); dragging is clamped at 0.2 mm (sliced) or 15 mm (`Selection.cpp:1338`).
Evidence: the A2L project's anchor 287.385 = 330 − 15 − 27.615 sits exactly on the bound, and
`Ŝ` = 27.615 needs `n` = 3, the plate's filaments, not the project's 4. The repo's `gui_tower_estimate`
counts all project filaments, which errs on the safe side. The P1S estimate is 17.16 (repo test). Real
bodies: 18.49 × 17.69 (P1S), 28.58 (A2L).

**S (Snapmaker Orca 2.4.0, owner's U1 GUI).** On *every* scene reload, i.e. on open and again after
each slice (GUI: `GLCanvas3D.cpp:3866-3910`):

```
m = 1 + prime_tower_brim_width             (raw setting: 6 mm for U1)
x ← x_max − W0 − m  if x + W0 + m > x_max,   x ← m  if x < m
y ← y_max − D̂ − m   if y + D̂ + m > y_max,    y ← m  if y < m
```

`x_max, y_max` = bed-shape bbox maximum in plate-local coordinates: 270.5 and 271 for U1. Inferred: both
owner plates sit exactly on `x = 270.5 − W0 − 6` (244.5 with `W0` 20, 248.5 with 16).
`D̂ = max(minD(object height), prime_volume·(n−1)/(0.2·W0))` before slicing (S `Print.cpp:3109-3140`,
GUI: `PartPlate.cpp:1687-1755`). **After slicing it is the real D.** So a tower placed against the top
edge using the estimate (11.27 for U1) moves down after the first slice if `y + D + 6 > 271`, and the
user must re-slice. The owner's 48-goat plate (y = 245, D = 19.7) is 0.3 mm inside that bound.
**The repo uses 1 mm for every Orca-family slicer (`src/plate_centre/gui_tower.rs:11-13`); for S the
margin is 6 mm at x and y.**

**O (Orca 2.4.2, server).** Opening a 3MF keeps the stored position (GUI: `Plater.cpp:6493-6499`); a
printer-preset change resets it to the default and clamps with `1 + brim` (`PartPlate.cpp:4206-4277`).
Dragging is clamped at 1 mm sliced / `brim + 0.5` unsliced (`Selection.cpp:1278`). A plain CLI slice
keeps the stored position; measured: 244.5 stays 244.5. Arrange paths clamp, but the packer always
passes `--arrange 0`.

**R (Orca 2.5.0-rc, local).** GUI and **CLI** clamp on every slice (CLI: `OrcaSlicer.cpp:6479-6512`,
GUI: `PartPlate.cpp:4057-4110`). The margin is `1 + b̂ + cone bulge`, with `b̂` = estimated real brim
`n_0·s_0` (4.91 for U1) and plate width 270 in local coordinates. Within 5 mm of the valid range it
**snaps**; farther out it is re-placed with a 15 mm comfort margin and a search for the nearest spot clear
of objects. Measured: every local U1 slice of an anchor stored at x = 244.5 printed at 244.09
(= 270 − 20 − 5.91), consistently over 69 slices. The owner's S keeps 244.5. The far-out "relocation"
depends on object positions, so a far-out anchor is not a stable input for R.

**Between slices of one project and plate**, the tower footprint is identical (§8.2). It moves only
through the clamps above: once on open (B), on every slice (R, deterministic snap), or after the
first slice (S, when the real depth exceeds the estimate near the top/right edge).

## 7. Determinism and nondeterminism

| # | source | affects the tower? | realizations |
|---|---|---|---|
| N1 | Tree-support realization (part 1 S1-S7) | only if supports or interfaces use their own filament: a layer's set `S_l` then depends on whether a branch exists there, which changes `k_l`, `H_t` and therefore D, the ribs and the auto brim | with filament 0 (all owner projects): none. Otherwise any `k_l` from "support absent" to "support present"; reserve with "present on every layer that can carry support" |
| N2 | Tool-order heuristic (flush-minimising order, Bambu) | which filament ends a layer, so `k_l` = #S_l or #S_l − 1 (#: number of filaments) at layers whose set changes | deterministic for a given project; our model brackets it (§4.2) |
| N3 | MMU segmentation (paint projected into the volume) | ±1 layer on where a colour band starts or ends | deterministic in the slicer; our mesh rule is off by ±1 layer at band edges, hence the dilation |
| N4 | Skip-point gaps (B, R) | where the wall is cut and so the Bambu rib offset (≤ 0.052 mm) | deterministic |
| N5 | Arc fitting (B with `enable_arc_fitting`) | ≤ 0.1 mm re-emission of the outline | deterministic |
| N6 | Slicer build (O vs R vs S on U1) | depth 9.5 / 14.9 / 16.1; brim chamfer; anchor snap | deterministic per build, but the builds differ: the predictor must target the build that produces the printed G-code |
| N7 | GUI clamps (§6) | anchor | deterministic; S is two-stage (estimate, then real depth) |
| N8 | SEMM purge-in-tower with `flush_into_*` | depth depends on object count and infill | deterministic; not used here |

Measured: no tower variation across 3 probe realizations, 3 sample realizations and final plates in
every build.

## 8. Validation

### 8.1 Data and method

- 147 G-code files: `/tmp/goat_baseline/*.work`, `/private/tmp/goat_p1s/*`, `/private/tmp/goatdbg/*`,
  `/tmp/goat_research/slices`, the owner's Snapmaker 2.4.0 G-codes
  (`koza-mini/print/scarf_fix/snapmaker_u1/koza48_29h_*.gcode`,
  `koza-mini/print/snapmaker_u1/gcode/koz40_24h_*.gcode`). Plus one Orca 2.4.2 slice of the U1 probe
  (single goat) in the `goat-runner-orca:2.4.2` image:
  `/Users/petrpopov/.cache/goat_tower_o242/out/plate_1.gcode`.
- Parser (`/tmp/goat_research/tower/measure.py`, `summary.py`): extruding moves (E > 0) inside
  `WIPE_TOWER_START/END`, `WIPE_TOWER_BRIM_START/END` or the "Prime tower" feature, minus nozzle wipes.
  G2/G3 arcs are sampled along the arc. Bambu G-code is shifted by `extruder_offset`. Inside tower
  sections no extrusion falls in a nozzle-wipe block (checked for all three printers).
- Predictors (our own code from this spec): `predict_bambu.py` (rib geometry with shapely; brim =
  mitred offsets), `predict_type2.py` (three Type 2 variants). Mesh step:
  `paint_layers_dilated.py`. Tables: `summary_all.txt`, `group.py`.

### 8.2 Stability

| build | slices | anchors seen | body | L0 |
|---|---|---|---|---|
| B P1S | 62 (probe ×3, sample ×3, finals of 40-43 goats, server probe) | (221, 207.25), (223.841, 223.841), (228.866, 228.866) | 18.484-18.489 × 17.580-17.638, identical per layer in every file | 24.885 × 24.029 everywhere |
| B A2L | 10 | (287.385, 15) | 28.550 × 28.573 (layer 10) | 34.967² |
| R U1 | 69 (+2 from 2.5-alpha) | stored 244.5 → printed 244.09; (17.77, 17.86); (17.77, 230.654) | 20 × 14.9 | 29.82 × 24.72 |
| S U1 | 4 owner plates (40 and 48 goats; two realizations each) | (244.5, 243.3), (248.5, 245) | as §1 | as §1 |

Within one build and project the tower does not depend on object count, support realization or anchor.
Only translation varies.

### 8.3 Per-layer residuals, Bambu (relative to `(X, Y)`; measured − predicted, mm)

P1S (predicted body 0..18.488 × 0..17.688; ring reach `n·s`):

| layer (z) | loops | measured x-range | y-range | Δ left / right / bottom / top |
|---|---|---|---|---|
| 0 (0.20) | 7 | −3.229 .. 21.657 | −3.202 .. 20.827 | −0.029 / −0.031 / −0.002 / −0.061 |
| 1 (0.48) | 5 | −2.229 .. 20.657 | −2.196 .. 19.838 | −0.029 / −0.031 / +0.004 / −0.050 |
| 2 (0.76) | 4 | −1.790 .. 20.218 | −1.756 .. 19.398 | −0.030 / −0.030 / +0.004 / −0.050 |
| 3 (1.04) | 3 | −1.343 .. 19.771 | −1.317 .. 18.958 | −0.023 / −0.037 / +0.003 / −0.050 |
| 4 (1.32) | 2 | −0.905 .. 19.333 | −0.881 .. 18.513 | −0.025 / −0.035 / −0.001 / −0.055 |
| 5 (1.60) | 1 | −0.466 .. 18.893 | −0.441 .. 18.074 | −0.026 / −0.035 / −0.001 / −0.054 |
| ≥ 6 | 0 | −0.031 .. 18.458 | 0.000 .. 17.638 | −0.031 / −0.030 / 0 / −0.050 |

A2L (predicted body 0..28.579 both axes):

| layer | loops | measured x-range | y-range | Δ left / right / bottom / top |
|---|---|---|---|---|
| 0 | 7 | −3.239 .. 31.728 | −3.202 .. 31.765 | −0.037 / −0.053 / 0 / −0.016 |
| 1 | 5 | −2.203 .. 30.728 | −2.189 .. 30.729 | −0.002 / −0.052 / +0.012 / −0.051 |
| 2 | 4 | −1.802 .. 30.287 | −1.750 .. 30.328 | −0.041 / −0.053 / +0.011 / −0.012 |
| 3 | 3 | −1.333 .. 29.807 | −1.311 .. 29.859 | −0.012 / −0.093 / +0.010 / −0.041 |
| 4 | 2 | −0.899 .. 29.373 | −0.872 .. 29.425 | −0.018 / −0.086 / +0.009 / −0.034 |
| 5 | 1 | −0.464 .. 28.938 | −0.433 .. 28.990 | −0.024 / −0.081 / +0.007 / −0.029 |
| ≥ 6 | 0 | −0.052 .. 28.526 | 0.000 .. 28.578 | −0.052 / −0.053 / 0 / −0.001 |

A negative "left" or "bottom" value means the print reaches past the prediction (unsafe direction): at
most 0.052 mm. Every other residual is on the safe side, at most 0.093 mm. Type 2 rectangles, rings and
chamfers match to 0.001 mm (S chamfer: layer `j` reach measured 3.519, 3.079, …, 0.440 = `n_j·s_j`).

### 8.4 Model-derived tool changes vs G-code

| project | real `k_max` / `H_t` | mesh rule `k_max` / `H_t` (dilated) | per-layer agreement |
|---|---|---|---|
| U1 (R) | 2 / 41.41 | 2 / 41.97 | undilated rule: all two-change bands found, edges off by ≤ 1 layer except single boundary layers (real 2 changes at 41.41 where the rule says 1) |
| P1S (B) | 2 / 41.36 | 2 / 41.92 | same; real 2 at the single layers 33.24 and 41.36 (ordering at band edges) |
| A2L (B) | 2 / 41.36 | 2 / 41.92 | same as P1S |

Only `k_max` and `H_t` enter the tower size. The dilated rule over-reserves `H_t` by 2 layers (0.56 mm),
which adds no width. The mesh pass takes about 4 s in Python on the 150k-triangle goat.

## 9. What the packer should reserve (proposal for the implementation part)

1. **Target build per printer:** B for P1S/A2L. For U1, S, because the owner prints the GUI's G-code.
   If the server's O G-code is ever printed directly, reserve the union of S and O; S is larger in every
   direction here (16.1 vs 9.5 deep, same 20 width, plus the chamfer).
2. **Zones, per §4.8**, with material edge = centre line + `pw/2` + 0.1 mm tolerance (lead decision,
   2026-11-02; design §4.2, §7.2):
   - **tower body** (`tower_body`), z-ranged: the column on every layer up to `H_t`, **plus the chamfer rings
     of layers 1..5 (B, z ≤ 1.88 at 0.28 mm) or 1..8 (S, z ≤ 2.49)**, each ring with its own layer interval.
     Hard for every foreign body and every support layer in every 2 mm band its pieces meet. The chamfer
     rings are solid printed loops above the first layer, so they belong to the body, not to the brim;
   - **tower brim** (`tower_brim`): the L0 ring only, z ∈ (0, h0]. Hard for bodies' first layers (landing).
     Support bases and object brims may overlap it (owner rule; GUI warns, CLI needs `--no-check`,
     zones-research §9). Because band 0 also contains the support base (design §4.1), the base can in
     practice overlap only the part of the L0 ring outside the layer-1 chamfer ring (B P1S: 2 of 7 loops;
     S U1: 3 of 11).
3. **Anchor feasibility** before packing, using the target build's clamp (§6): for S,
   `6 ≤ x ≤ 270.5 − W0 − 6` and `6 ≤ y ≤ 271 − max(D̂, D) − 6` (margin 1 + `prime_tower_brim_width`, with the
   real depth D because S re-clamps after every slice); for B, the 15 mm rule plus the GUI estimate Ŝ with
   `n` = the plate's used filaments; for R, inside `1 + b̂` (+ cone bulge) so that its CLI never snaps
   (`x ≤ 270 − W0 − 1 − b̂` for U1); for O, inside `1 + brim` (no clamp on open).
4. **Input to `tower_scan`** (design §9.3, which replaces the tower contest and the free-spot search): this
   part supplies, per target flavour, the anchor range of item 3 and the per-layer footprint of §4.8 (body
   with chamfer rings, L0 brim). `tower_scan` sweeps anchor candidates over that range (flush with each side
   every 2 mm, plus the corners and the source anchor) and scores them with this predicted footprint, never
   with a probe measurement. The probe's tower is the R tower on U1, not the owner's.

## 10. Open questions

1. **Which U1 slicer is authoritative?** Server final slices (O 2.4.2) print a 9.5-deep tower; the
   owner's GUI (S 2.4.0) prints 16.1 (`W0` 20) and adds a 4 mm chamfer. A layout verified on the server
   can collide in the owner's slice anywhere 9.5-16.1 mm above the tower's front edge. Fix by predicting S
   (this spec) and/or pinning the server to a build that reproduces S. S's CLI segfaults (CLAUDE.md),
   so no slice-based check of S exists.
2. **S plate bbox:** `x_max = 270.5`, `y_max = 271` are inferred from two owner plates on the bound, not
   read from the S bed-shape code path. One GUI open of a probe 3MF with x = 245 would confirm it; S
   should then move the tower to 244.5.
3. **S post-slice re-clamp:** confirm in the S GUI that a tower stored at `y > 271 − D_real − 6` moves
   after the first slice (code reading says it does).
4. **Per-layer filament sets at band edges** come from MMU segmentation, which this spec does not model.
   The dilation covers the goat, but not a feature thinner than one layer that segmentation erases or
   creates. A bigger model set (other painted projects) should be checked with `paint_layers_dilated.py`
   against their G-code.
5. **Support / interface with a dedicated filament** (N1): not exercised by any owner project. If it
   ever is, `k_l` depends on the tree realization, and the predictor must assume support on every layer
   that part 1 marks as possibly supported.
6. **Orca Type 1** (Bambu printers sliced in Orca) and **Type 2 rib/cone walls** are described from
   code only; no G-code was measured.
7. **R relocation** for anchors more than 5 mm out of range searches for a spot clear of objects. If
   R stays in local use, keep anchors inside its range (`x ≤ 270 − W0 − 1 − b̂` for U1) so it only snaps.

## Appendix A. Constants

| constant | value | where |
|---|---|---|
| tower line width ratio | 1.25 × nozzle | B `GCode/WipeTower.hpp:420`, S/O/R same |
| min depth by height table | 5→5, 100→20, 250→40, 350→60 mm | B `GCode/WipeTower.cpp:1586-1588` |
| auto brim | `min(8, 8·H/100)` | B `:1627-1630` |
| Bambu chamfer max | 3 mm | B `:3710` |
| S chamfer max (default) | 4 mm | S `PrintConfig.cpp:6105-6110` |
| fillet | 2 mm tangent, turn 30-150°, 20 points | B `:128-186`, `:22` |
| Bambu arc-fit tolerance | 0.1 mm | B `:20-23` |
| skip-point gap | 2.5·`pw` | B `:5110`, R `GCode/WipeTower2.cpp:2673` |
| GUI margin | B 15 (0.2 when dragging a sliced tower); O/R/S 1 (+ brim in the S/O/R clamps) | B `libslic3r.h:77-78`, O `:90`, R `:100-103`, S `:93` |
| R comfort margin / snap tolerance | 15 mm / 5 mm | R `libslic3r.h:103`, GUI `PartPlate.cpp:4090` |

## Appendix B. Pseudo-code (ours)

```
predict_tower(project, build):
    S_l  := filament sets per layer from paint + parts + layer ranges, dilated ±1 layer      (§4.1)
    k_l  := |S_l| − 1 if S_{l−1} ⊆ S_l or l = 0, else |S_l|                                  (§4.2)
    H_t  := top z of last layer with k_l > 0
    if build = B:   (W, D, L_r, r, D_l) := bambu_depth(k_l, h_l, prime volumes, W0, gap)        (§4.3)
    else:           (W, D, D_l)       := type2_depth(build, k_l, h_l, V, minimal purge, W0)  (§4.4)
    for each tower layer l:
        outline_l := wall outline (rectangle | rib with taper | cone)                       (§4.5)
        rings_l   := brim loops (l = 0) or chamfer loops (B, S)                              (§4.6)
    anchor := target build's clamp(stored anchor, estimate)                                 (§6)
    return per-layer polygons placed by the build's transform                               (§4.7)
```

# Organic support spec set: consistency review

2026-11-02. Reviewer pass over parts 01-04 of this directory, the architecture
(`../2026-11-02-slicer-free-packer-design.md`, cited as **D §n**), the calibration corpus
(`/tmp/goat_calib/MANIFEST.md`, cited as **M §n**), the research reports in `/tmp/goat_research/`
and `CLAUDE.md`. Source spot checks use `/tmp/goat_src` (B = `bambu_0208` / `zones_bambu2861` at
v02.08.02.61, O = `orca_242`, R = `orca_250rc`, S = `snaporca240`; paths under `src/libslic3r/`
unless prefixed `GUI:`). No repo file other than this one was changed. One small review experiment ran
locally on the mesh only (appendix A). No slicer was run.

Verdict: the support chain itself is complete. Every stage from layers to the first-layer flange has
a behaviour description, and 11 of 13 spot-checked load-bearing claims hold in the source. The set is
**not yet consistent enough to implement against**, for four reasons:
1. The design's nondeterminism model (S1 and S2 as the main ensemble axes) does not match the corpus.
2. The U1 target flavour (S) has no calibration data.
3. The tower permissions and clamps in the design contradict part 04.
4. Several passages read as structural transliterations.

---

## 1. Prioritized fix list

**P0: blocking (safety, targets, licence)**

| # | fix | where | why |
|---|---|---|---|
| 1 | Make tower chamfer rings on layers ≥ 1 **hard** for `support[k]`/`interface[k]` (and bodies). Only the L0 tower brim stays "allowed" for the support base and object brims. | D §7.2 row "support[k] / interface[k]", column "tower brim / chamfer" (`allowed`); D §4.2 `tower_brim[l]` | 04 §9.2 calls the chamfer "solid printed loops, hard for supports and bodies on those layers". The owner's allowance covers tower *brim* overlap at L0 only. B prints 5 chamfer layers up to z ≈ 1.88 (verified, B `GCode/WipeTower.cpp:3706-3722`), S prints 8 layers up to z ≈ 2.49. |
| 2 | Correct the tower anchor clamp per flavour. **S: margin = 1 + `prime_tower_brim_width`** (6 mm for U1), applied on every scene reload, including after slicing with the real depth. **R: the CLI clamps/snaps on every slice.** B: 15 mm plus the GUI estimate, with *n* = the plate's filaments. Update D §3.1 row "GUI tower anchor margin", D §6 `gui_anchor_range`, D §9.3, `CLAUDE.md:36` and `src/plate_centre/gui_tower.rs:11-13`. | D §3.1, §6; CLAUDE.md | 04 §6. Verified in S `GUI:GLCanvas3D.cpp:3883` (`margin = WIPE_TOWER_MARGIN + tower_brim_width`). A tower placed with a 1 mm margin moves 5 mm in the owner's GUI, and the packer's tower keepout is then wrong. |
| 3 | Restate the δ risk using the corpus. D §5.5 bases its initial δ_L1 1.0 / δ_band 1.5 and its goal ≤ 0.3 / 0.5 on a 0.81 / 1.18 mm observation (TSR §4.4). The corpus shows more. | D §5.5, §0 "Measured headroom", §13 Q1 | Packer CLI plates put L1 support up to 2.4 mm (one plate 3.0 mm) and band support up to 4.4 mm (3.7 mm) outside a 5-run single-object union (M §2.3). The S GUI lies 0.7-4.1 mm (L1) and 1.5-5.4 mm (bands) outside the R CLI union (M §2.4). If the ensemble does not reproduce these realizations, the targets fail (RP §5.3: P1S/A2L already fail at g + δ ≈ 1.5). |
| 4 | Re-found the nondeterminism model (D §3.2, §5.4): **(a)** add **S9** (process-static arc thresholds, 01 §5.5.4) as an ensemble axis; **(b)** replace the raw thread list `{1,2,4,…,64}` with 02 §4.7's exact **T-class enumeration**; **(c)** demote S1 from "main axis" to "unproven" until the discriminating experiment in §5.3 has run. | D §3.2 table, §5.4 schedule | R contains the same S1 race (R `Support/TreeSupport3D.cpp:1023-1026` inside the parallel loop at `:1371`, unchanged from O `:976-979`, `:1324`). Yet it gave **1 realization in 30 observations**, full plates included (M §2.4, §5.3). R fixed exactly S2 (`:2432`) and S9 (`Fill/FillBase.cpp:2696-2701`). S2 is constant on one machine, so B's run-to-run spread (7 L1 families, 16 line-level realizations in 53 P1S 0° observations) is not explained by S1 + S2 + S9 as specified. S3 is ruled out (appendix A). The B source is unidentified: this is the largest model risk. |
| 5 | Give the U1 target flavour (S, D §14 Q2) calibration data. The corpus has **no** S single-object slices and **no** O 2.4.2 support slices. All U1 CLI data is R, which differs from S in S2 class, S9, flat foot, smoothing anchor, collision simplification, tower depth and CLI tower snap. | M §1.1; D §11 harness | See §5. Without it the U1 envelope is unvalidated, and the R corpus (deterministic) understates the variability the owner actually prints. |
| 6 | Fix the B coordinate grid. B uses **10 nm units with int32 `coord_t`** (B `libslic3r.h:40,58`); O/S/R use 1 nm with int64. 01 §0 says "1 nm" and 01 §4 gives B values in nm (`lh 280 000`, `move_fast 279 999`, base spacing `5 359 911`). D §5.2 says "1 nm grid, like upstream's scaling". Make the grid a flavour property. | 01 §0 (l. 45-46), §4 table; D §5.2 rules | 02 §1.1 is right. Every truncation of B speeds, radii and cells changes (02: V_f = 27 999 units = 0.27999 mm). |
| 7 | Decide the boolean backend's fidelity target. D §14 Q6 chose `i_overlay`. 02 §6 and 01 Q3 say exact reproduction needs Clipper-1 semantics: miter-limit floor of 2, arc-tolerance rule, the fork's 0.005·\|d\| input decimation, and B's int32 grid. Clipper itself is Boost-licensed, so an independent Rust port is allowed. The Slic3r decimation can be implemented from its behaviour description. | D §14 Q6 vs 02 §6 | The discrete decisions downstream (ε_A tests, quickselect over integer bbox centroids, 40 µm dedup cells, radius keys) can flip. Any non-reproduced branch structure becomes δ, and δ is already the binding risk (fix 3). |
| 8 | Rewrite the passages listed in §6. They mirror upstream control flow and identifiers. | 01 §5.3, §5.4, §5.5.3; 02 §3.3, §3.4, §4.2, §4.4, §5.4, §5.5; 03 §2.4 | Licensing rule (D §1). |

**P1: correctness of the spec text**

| # | fix | where |
|---|---|---|
| 9 | Define one band-assignment rule for every kind. Support is banded by `print_z`, but its material occupies (z_{l−1}, z_l] (03 §9). Bodies are banded by mesh slab. A support layer with print_z 2.16 (material 1.88-2.16) is checked only against `body[1]`, while a foreign body top at z = 1.95 lives only in `body[0]`. Assign each layer to every band its interval meets, or extend the body slabs by one layer height. Also settle whether band 0 includes layer 0: D §4.1 says the first layer is "its own kind, not band 0", while 03 §9 and the repo convention put layer 0 in band 0. | D §4.1, §4.2; 03 §9 (l. 877-882) |
| 10 | Write the **landing** behaviour spec (D §6, §13 Q4). No part covers it. Use the corpus `l1.body_polygon` (every slice) to measure the printed first-layer body against the section and fix the overshoot rule. | new part (05) |
| 11 | D §4.2 `support_base` says "trimmed by own `landing ⊕ first_layer_gap`". D §5.2 B7 and 03 §4.2 say the trim is the slicer's own **uncompensated L_0 ⊕□ g** (square joins), not the conservative landing kind. Align the kinds table with B7. | D §4.2 (l. 176) |
| 12 | D §3.1 row "auto-brim width formula: same in all four" and D §6 ("identical in all four versions") are wrong. B multiplies the thermal term by 0 (verified, B `Brim.cpp:798`: `0. * thermalLength …`). O/S/R use it. The thin-island extra brim (O/S/R) and the no-brim set closing (B and S only) also differ (03 §5.2-5.3, §10). | D §3.1, §6 |
| 13 | 03 §3.7 step 3 says "Orca 2.4.2 and Snapmaker additionally force strictly simple output". Wrong: O and S pass the constant `false` (O `Support/TreeSupportCommon.hpp:591`, O `Support/TreeSupport3D.cpp:4151`, S `:3981`). B's two-argument overload and R's default are the strict ones. This is consistent with 02 §1.1. | 03 §3.7 (l. 447-448) |
| 14 | Pin small numeric disagreements. The tip seed circle is a 45-gon in Orca but a **15-gon in B** (02 §1.3; 01 §5.4 says 45 for all). The vertical-enforcer circle is a 315-gon only on the 1 nm grid (≈ 100-gon in B; 01 §2.6). `⊕mi(1.2)` in 01 and 03 must note the Clipper floor of 2 (02 §1.1). | 01 §2.6, §5.2-5.4; 03 §2.2 |
| 15 | Add the unsupported-settings policy. 01 lists about 15 rejects: variable layer height, ZAA, `precise_z_height`, honeycomb/lightning base, soluble/zero-gap interfaces, raft, support on model, disjoint parts (01 Q5), non-default XY compensation, S mixed-filament/dithering modes, non-organic styles. D must say what the default job does on them: hard error, or automatic `--support-source slicer`. The webapp accepts arbitrary projects. | D §5, §10.1 |
| 16 | D §3.1 critical-only row for O/R/S: "keeps cantilevers (+ enforcers)" misses the **sharp-tail supports** (01 §3.7 O step 6, §7). | D §3.1 (l. 113) |
| 17 | D §9.7 B canonical threshold "foot radius 3.75 + xy 0.75 + 1 mm". Derive it from 02 §2.2 instead. The B bed obstacle acts through *rounded radius keys* plus xy_min (centre clearance 6.06 mm for a full foot, 2.81 mm for a 1 mm branch in regular mode). As an envelope-to-edge distance per band that is ≤ ≈ 1.9 mm, so 5.5 mm is safe but over-strict. The P1S cutter is part of the obstacle (02 §2.8), so the threshold applies to it too. | D §9.7 |
| 18 | D §10.2.2 says Bambu single-object G-code "has no object labels". The corpus finds that single-object Bambu plates **do** carry `; OBJECT_ID:` (M §1.3). Keep the parser fix, but correct the statement. | D §10.2 |
| 19 | D §3.2 S2 asserts "Docker `--cpus` does not change it". 02 §7 Q2 marks the oneTBB cgroup behaviour as unverified. Mark it as an assumption in D. | D §3.2 |
| 20 | Unify the version prefixes. B/O/R/S (D, 01) vs B/O24/S24/O25 (02) vs B/O/O25/S (03, 04). | all parts |
| 21 | 04 §9.4 still proposes "tower contest corners and free-spot search". D §9.3 replaces both with anchors inside the search. Reword 04 §9.4 as input to `tower_scan`. | 04 §9.4 |

**P2: follow-ups**

| # | fix |
|---|---|
| 22 | `CLAUDE.md:79` ("vanilla Orca on the server … matches the user's Snapmaker Orca 2.4.0 GUI") holds for the L1 patch bbox only. The server's O 2.4.2 tower is 9.5 mm deep against the owner's S 16.1 mm (04 §1). Supports differ by up to 4-5 mm (M §2.4). The server verify step therefore cannot catch tower or support conflicts of the S GUI. Record this in CLAUDE.md and in D §10.2.6. |
| 23 | Turn corpus findings 7 and 8 into fixtures for 01 §3: `arch` gets no support with critical-only; R does not support the mushroom cone, which 01 §3.6 explains as a single-layer cluster, so small. B supports the cone because of the floating-ring cantilever rule (01 §3.2.5). Both are already predicted by 01, so they are cheap acceptance tests. |
| 24 | Corpus finding 2: B adds supports at z 26-41.6 at **225° only**, in 5 of 5 runs. 01 has candidate mechanisms: axis-aligned bbox rules in sharp-tail and small-overhang removal, and B's floating-ring cantilever. None is shown to be the cause. Add it to 01 §8 as Q9. It is required before `--rotations` admits classes beyond {0°, 180°}. |
| 25 | Smoothing σ (03 Q1) and connector fidelity (01 Q1): decide both before WP S3 starts. They set how far "exact" can be from "envelope plus δ". |

---

## 2. Coverage: does the spec set produce every geometry kind of D §4.2?

| kind | stages needed | where specified | status |
|---|---|---|---|
| `support_base` (L0) | A1-A5, B1-B6, flange, canonical-only bed clip, material edge | 01 §1-§5 → 02 §2-§5 → 03 §2-§4, §6, §7 | **covered**. Open: connector fidelity (01 Q1), σ (03 Q1), S9 axis missing from D |
| `support[k]` | as above + band assembly | 03 §3, §9.2 | **covered**. Gaps: band boundary rule (fix 9); lost-branch drop columns unmeasured on goats (03 Q7) |
| `interface[k]` | roof stack, interface relabelling, material vs polygon | 01 §5.4; 03 §3.8, §7 | **covered**. Interface material may exceed P by +0.03 (hard bound +0.2, 03 Q4) and must enter δ or the dilation |
| object `brim` | auto width, ring, no-brim trim, loops | 03 §5 | **covered**; validated by one shape (`spire`) only |
| support brim | — | 03 §4.3 (none in any version; verified O `Brim.cpp:592-630`) | **covered** (empty) |
| `landing` | first-layer printed body: perimeters, Arachne tips, EFC | none | **gap** (fix 10) |
| `body[k]`, `silhouette`, `part_hulls`, `part_boxes` | mesh slab projection | D §4.2 (geometry, not slicer behaviour); 01 §1.3-1.5 give the slicer's section rules | covered for the mesh. The printed-vs-mesh relation (outer wall inside the outline, overhang sag) is unstated and unvalidated per band |
| `tower_body`, `tower_brim[l]` | filament sets, k_l, H_t, depth, wall, brim + chamfer, placement, clamps | 04 §4-§6 | **covered** for B / O / R / S. Open: S plate bbox inferred (04 Q2), MMU band edges (04 Q4). Permissions conflict with D (fix 1) |
| shared-object / canonical copy | one computing object per identical group | 03 §6, D §9.7 | covered |
| ensemble axes | S1, S2, S3, S9 | 01 §3.9, §5.5.4, §5.7; 02 §4.7 | covered in the parts; **not** carried into D §3.2 / §5.4 (fix 4) |
| GUI acceptance checks (verify) | centreline bbox, exclusions, ConflictChecker | D §10.2.3 references a separate spec | **gap**, but needed only for `--verify` |

Stage map from D §5.2 to the parts: A1-A2 → 01 §1; A3 → 01 §2; A4 → 01 §3; A5 → 02 §2; B1 → 01 §5;
B2 → 02 §3; B3 → 02 §4; B4 → 02 §5; B5 → 03 §2; B6 → 03 §3; B7 → 03 §4; B8 → 03 §7, §9.
No stage is missing.

---

## 3. Contradictions

| # | statement | conflicts with | evidence | resolution |
|---|---|---|---|---|
| C1 | D §7.2: support over the tower chamfer is allowed | 04 §9.2: the chamfer is hard for supports | source (B chamfer always on) | 04 is right (fix 1) |
| C2 | D §3.1 / §6, CLAUDE.md:36: the Orca-family GUI margin is 1 mm; the CLI keeps the anchor | 04 §6: S uses 1 + brim (6 mm) and re-clamps after slicing; R's CLI snaps | S `GUI:GLCanvas3D.cpp:3883`; 69 R slices printed at 244.09 instead of 244.5 (04 §6) | 04 is right (fix 2) |
| C3 | D §5.5: initial δ 1.0 / 1.5, goal 0.3 / 0.5 | M §2.3-2.4: exceedances of 2.4-4.4 mm against 5-run unions; GUI vs CLI 0.7-5.4 mm | corpus | fix 3 |
| C4 | D §3.2: S1 is the primary run-to-run axis and S2 is covered by the thread list; no S9 | 01 §5.5.4 (S9); 02 §4.7 (exact T-classes); M §5.3 (R deterministic despite S1) | source + corpus | fix 4 |
| C5 | 01 §0, §4 and D §5.2: 1 nm grid for all | 02 §1.1: B uses 10 nm, int32 | B `libslic3r.h:40,58` | 02 is right (fix 6) |
| C6 | D §14 Q6: `i_overlay` | 02 §6: Clipper-1-compatible engine for exact runs | — | decision needed (fix 7) |
| C7 | D §4.2: `support_base` trimmed by `landing ⊕ gap` | D §5.2 B7 and 03 §4.2: trimmed by uncompensated L_0 ⊕□ g | B `Support/SupportCommon.cpp:441`, O `:382` | B7 is right (fix 11) |
| C8 | D §4.1: the first layer is not band 0 | 03 §9: band 0 contains layer 0 (repo `learning.rs` convention) | — | fix 9 |
| C9 | D §3.1 / §6: auto-brim formula identical in all four | 03 §5.2: B Θ = 0 | B `Brim.cpp:798` | 03 is right (fix 12) |
| C10 | 03 §3.7: O and S force strictly simple output | 02 §1.1: O and S are not strict | O `Support/TreeSupportCommon.hpp:591` | 02 is right (fix 13) |
| C11 | 01 §5.4: 45-gon tip seed | 02 §1.3: 15-gon in B | `Polygon.cpp` circle rule on the 10 nm grid | 02 is right (fix 14) |
| C12 | D §3.1: O critical-only keeps "cantilevers (+ enforcers)" | 01 §3.7, §7: also sharp tails | O `Support/TreeSupport.cpp:1043-1115` per 01 | fix 16 |
| C13 | D §10.2.2: Bambu single-object G-code is unlabelled | M §1.3: it carries `; OBJECT_ID:` | corpus | fix 18 |
| C14 | CLAUDE.md:79: server vanilla Orca matches the user's GUI | 04 §1 item 3 (tower 9.5 vs 16.1 mm); M §2.4 (supports 0.7-5.4 mm) | G-code | fix 22 |
| C15 | CLAUDE.md:47: one core makes supports repeatable, implying multi-core runs vary | M §2.4: multi-core R is repeatable | corpus + R fixes | consistent once fix 4 is written down: R is repeatable because of its S2/S9 fixes, not because of the CLI (this answers M §8 Q4) |
| C16 | D §3.2 S2: Docker `--cpus` does not change T | 02 §7 Q2: unverified | — | fix 19 |
| C17 | 04 §9.4: tower contest and free spot | D §9.3: removed | — | fix 21 |

Checked and consistent: first-layer expansion rule (D §3.1 vs 03 §4.2); flat foot vs hemisphere; B bed
obstacle; keep-main pruning; smoothing anchor limit (O and R only); shared-object rule (D §0.3, 03 §6);
no organic support brim; tower growth independent of object count (D §14 Q3, 04 §1); mfs bug and radius keys
(02) vs the measured foot clearance (02 §5.1); band and frame conventions between M §1.2 and D §4.1.

---

## 4. Source spot checks (load-bearing claims)

| # | claim | spec | source | verdict |
|---|---|---|---|---|
| 1 | Merge bucket size is 4 if ⌊(n+2)/4⌋ ≥ T, else 2 (B/O/S); R fixed at 4 | 02 §4.3, D §3.1 | O `Support/TreeSupport3D.cpp:2385-2391`; B `:2238`; S `:2389`; R `:2432-2434` | **holds** |
| 2 | S1: tips inserted under one mutex with first-come dedup in cells of `(min_radius+1)/10`, per insert layer, from a parallel loop | 01 §5.7 | O `:974-1000` (loop `:1324`); B `:851-854`; R `:1023-1026` (loop `:1371`, unchanged from O) | **holds in code**; empirically inert in R (C4) |
| 3 | S9: the arc thresholds 1.3/2/3 × spacing are `static` in B, O and S, and removed in R | 01 §5.5.4 | O `Fill/FillBase.cpp:2464-2466`; B `:2367-2369`; S `:2456-2458`; R `:2696-2701` (with the comment explaining why) | **holds** |
| 4 | B coordinates are 10 nm / int32 | 02 §1.1 (and against 01 §0) | B `libslic3r.h:40` (`#if 1 … int32_t`), `:58` (`SCALING_FACTOR = 0.00001`); O `:43,60` | **02 holds, 01 fails** |
| 5 | First layer: B −1 means auto 2 mm; O uses max(0, value − ε); n = max(5, ⌈e/w0⌉) steps; trim = lslices ⊕ gap with square joins | 03 §4.2, D §3.1 | B `Support/SupportCommon.cpp:436-455`; O `:286-288`, `:382-389`; `SUPPORT_SURFACES_OFFSET_PARAMETERS` = jtSquare (O `:44`) | **holds** |
| 6 | Smoothing push = (clamped depth + 0.1)², capped 0.5 before squaring; smoothing step capped 0.2; at most 100 iterations; stop when nothing moved > 1e-4 | 03 §2.4 | B `Support/TreeSupport3D.cpp:3749-3752`, `:3787-3793` (the nudge vector already has length `nudge_dist` and is multiplied by it again), `:3815-3833` | **holds** |
| 7 | Flat foot on bed roots in O and R, hemisphere in S | 03 §3.2, D §3.1 | O `:2943-2957`; R `:2988`; no equivalent in S | **holds** |
| 8 | B tower chamfer always on, ≤ 3 mm, one loop fewer per layer, stops when the layer depth changes | 04 §4.6, D §3.1 | B `GCode/WipeTower.cpp:3706-3722` (clean tag, `git blame` = release commit) | **holds** |
| 9 | S GUI re-clamps the tower with margin 1 + `prime_tower_brim_width` on every reload | 04 §6 | S `GUI:GLCanvas3D.cpp:3866-3910` | **holds**; contradicts D/CLAUDE.md (C2) |
| 10 | Auto-brim thermal term is ×0 in B and active in O | 03 §5.2 | B `Brim.cpp:798`; O `:298` | **holds**; contradicts D (C9) |
| 11 | O and S force strictly simple output in the final per-layer clean-up | 03 §3.7 | O `Support/TreeSupport3D.cpp:4151` passes `polygons_strictly_simple` = `false` (`TreeSupportCommon.hpp:591`); S `:3981` the same; B `:5201` uses its two-argument overload | **fails** (C10) |
| 12 | Radius keys are 0, 0.4, 1.5, 2.6, then ×1.5 (3.9, 5.85, …) | 02 §2.2 | O `Support/TreeModelVolumes.cpp:835-862`, factor/threshold `TreeModelVolumes.hpp:32-33` | **holds** (the 1.5 → 2.6 step comes from the post-loop increment) |
| 13 | `min_feature_size` 25 % is read as 25 mm | 02 §0.4 | O `Support/TreeSupportCommon.hpp:58`, B `:51` (`scaled(config.min_feature_size.value)`); all three projects have `"25%"` | **holds** |

---

## 5. Is the calibration corpus sufficient?

### 5.1 What it validates well

- B single-object behaviour at 8 rotations × 5 runs for P1S and A2L, R at 8 × 5 for U1. All slices are
  sequential, on one machine, with the packer's exact CLI shape.
- The rotation finding (support is computed in the plate frame; a rotated template is wrong by up to
  5.7 mm) confirms D's S5 and the "one ensemble per rotation class" rule.
- "One realization per rotation class per slice" holds in all 79 multi-object G-codes (M §2.5). This
  confirms D §0.3.
- Tower bboxes are stable across rotations, runs and object counts (M §2.9, 04 §8).
- 72 synthetic slices give rule-level fixtures (critical-only, threshold, small overhang, one brim case).

### 5.2 What it does not cover

| gap | consequence | needed (sequential single-object slices unless noted) |
|---|---|---|
| **No S (owner U1 GUI) single-object data**; only 5 GUI plates (four at 0°, one at 45°, none at 180°). S's CLI segfaults. | The U1 target envelope cannot be calibrated. R (all U1 CLI data) is a different flavour and a different S2 class (02 Q6). | Owner GUI exports of a single goat at 0° and 180°, ≥ 5 each. Also one batch after a fresh GUI start and one after slicing another project first, to probe S9 session history (01 Q2). |
| **No O 2.4.2 support slices** (one tower-only docker slice) | The server verifier's flavour is unvalidated (D §10.2.6). O has S1, S2 by T and static S9, which makes it the only CLI proxy for S's nondeterminism. | `goat-runner-orca:2.4.2`: 10 runs at 0° and 180°, at a fixed cpuset, then cpusets {1, 2, 4, 14} (T-classes, 02 §4.7) |
| **5 runs per rotation, B unions not saturated** (16 line-level realizations and 7 L1 families in 53 P1S 0° observations, still growing) | δ cannot be set with a published exceedance rate (D §5.5) | ≥ 40 more runs at 0° and 180° for P1S and A2L (≈ 4 s each, ≈ 11 min per printer) |
| **S2 classes**: everything ran on one 14-thread Mac; macOS cannot pin cores | Only the owner's T = 14 class is sampled. The server's class is unknown (02 Q2). | B and O on Linux with `taskset`/cpuset T ∈ {1, 2, 4, 8, 14}, 5 runs each at 0° |
| **Source of B variability unidentified** (C4) | The ensemble may not contain the realizations B prints | Discriminating run: O 2.4.2 ×10 at a fixed cpuset. If it varies: S1 or S9 (R has S1, so mainly S9). If it does not: the B spread comes from a B-only source, and B's code paths outside 01-03 must be re-read. Plus B at cpuset 1 ×5 (removes S1 and fixes the S9 order). |
| Realization identity is line-level (`support_hash`, buffered line areas) | It conflates toolpath variation (S9 in base/interface connection, B lightning void fill) with footprint variation. Envelopes only care about footprints. | Recount distinct realizations and exceedances on outer boundaries (`support_patch_polygon`, per-band patches, Hausdorff of the boundary) |
| GUI vs CLI for B: only one P1S GUI plate (L1 616.7, never produced by the CLI) | Assumption "owner GUI ≡ CLI distribution" (02 Q1) is open | Owner GUI single-goat P1S/A2L exports at 0°, 5 each |
| **No A2L full plates**; no full plates at 180° or with mixed 0/180 for any printer | S8 (contention) unmeasured for A2L; the double-lattice member (D §9.2 B, always on per §14 Q1) and canonical-first ordering with two classes are unvalidated | Owner-sliced (off-peak, one at a time) A2L plate and one mixed 0/180 plate per printer through `plate_audit` (D §11). No full-plate slicing by agents. |
| **No near-edge / near-cutter single-object slices** (M §8 Q8) | S4 clip and the B corridor obstacle (02 §2.8, 03 Q2) are uncalibrated | B P1S single goat at a bed-edge midpoint, a corner and against the cutter, 3 runs each |
| Body / landing ground truth per band | The landing rule (fix 10) and the body-band assumption are unvalidated | Already in the G-code: extend `derive.py` to body polygons per band (no new slices) |
| Rotation sensitivity sampled at 45° steps only (225° anomaly) | Classes beyond {0°, 180°} cannot be admitted | P1S 5° sweep, 1 run each (72 slices ≈ 5 min) |
| Synthetic set: no paint, no blockers, no disjoint parts, 0/90 only | 01 Q5 (disjoint fast path), paint projection timing and blocker volumes (R) are untested | Two-cube disjoint object (R and B); a painted synthetic with an enforcer patch |

### 5.3 Stage-level validation

The corpus can validate only end-to-end G-code geometry. Tips, nodes and influence areas are not
observable, which makes S3/S4 debugging slow. An option for the owner to decide: a local, never
distributed, instrumented build of R or O 2.4.2 that dumps tips and node positions. The upstream debug
SVG switches already exist. Running AGPL code privately does not affect our licence. Reading its dumps
is behaviour observation, not copying.

---

## 6. Licensing: passages to rewrite

The rule (D §1): behaviour in our own words. No upstream control-flow correspondence and no upstream
identifiers in operative text. Citations may name upstream functions. The passages below restate
upstream procedures step for step, often with the same branch structure, special cases and constants
in the same order. Rewrite each as (a) the observable result (an invariant, formula or order
property), (b) the constants, and (c) recorded test vectors where the exact order matters.

| where | quote (excerpt) | problem | rewrite as |
|---|---|---|---|
| 02 §3.4 (l. 528-537) | "3. k := (d > c) ? ⌊(d − c)/s⌋ : 0. 4. If d − k·s > c: if (k+1)·s ≤ d then k := k+1, else final := true. 5. step := s. If n > 1 and k + [d < c or d mod s ≠ 0] < n: … 6. … After steps with i mod 10 = 7, apply S(0.015 mm)." | Line-by-line restatement of the stepped safe offset, including its ternaries and the iteration-index simplification | The goat regime is one statement (s ≈ 25 mm ≫ d: two half-steps with wall subtraction, then one unchecked grow). The general rule becomes: "grow in at most N equal steps of at most s, subtracting W after each step; simplify periodically", with the step-count formula as maths |
| 02 §4.2 (l. 696-709) | "2. Sort the three entries at lo, c and hi in place with three conditional swaps … 5. Swap the entries at c and hi − 1. Set i := lo, j := hi − 1. 6. Repeat: advance i by one at least once …" | The exact partition code of the AABB builder in prose | "Median-split order produced by a median-of-three Hoare quickselect (textbook variant, pivot = median of first/middle/last, ties …)". Pin the result with recorded permutations on fixed inputs. |
| 02 §4.4 (l. 733-759) | "while i + 1 < e: for j := i+1 … e−1: if pairMerge(dst = M[i], src = M[j]) succeeds: move M[e−1] into slot j; e := e − 1 …" and "the absorbed src slot is refilled: the earliest still-unabsorbed src (at S's moving start pointer) is moved into slot s" | Loop and swap-remove structure of the merge rounds | The order invariants (destination = left element; survivor order after a round) as statements, plus test vectors |
| 02 §5.5 (l. 963-973) | "visit its edges in the order (q_{k−1}→q₀), (q₀→q₁) … Keep a flag 'x projects beyond the end of the previous edge', initialised from the edge (q_{k−2}→q_{k−1}) … t ≤ 0 and the flag is set: candidate = vertex a. Clear the flag." | Mirrors upstream's per-edge loop and its flag variable (R names it `projected_p_beyond_prev_segment`) | "Nearest boundary point. Ties go to the first ring, then the first edge in ring order starting with the closing edge. Projections are truncated toward zero per component." |
| 02 §5.4 (l. 950-955) | "When a deleted element is found at i, drop the trailing deleted ones again, then move the last element into slot i and re-examine slot i." | Swap-remove loop | The resulting order property in one sentence |
| 02 §3.3 (l. 496-508) | `e = 5 units … if K(r⁺, P.min) == K(r_P, P.min) or r⁺ < R_inc: e += Δr else: e_s += min(Δr, (V_f + e) − (V_s + e_s))` | Code-shaped block mirroring upstream's extra-speed branches | A case table: speed bonus by (key unchanged / radius below R_inc / foot zone) |
| 01 §5.3 (l. 693-707) | `while rest ≠ ∅ and acc + w_s/8 < extra_outset: acc += w_s/8; rest = rest \ grow_clear(reg, 1.5·acc, C0); reg = reg ∪ (grow_clear(rest, acc, C0) \ F_m)` | Same loop, same operations, same 1.5 factor as O `Support/TreeSupport3D.cpp:1363-1376` | "Seven outset rounds of w/8 each (goats). Unsupported overhang within 1.5·acc of the tip region is dropped, and the rest is added as fringe." Formulas, not a loop. |
| 01 §5.4 (l. 718-729) | `while t < R and m − t ≥ 1: next = A_t \ (F_{m−t−1} ⊕mi(1.2) 5 µm); if area(next) < 1 mm²: if t > 0: lines = …; stop; roof[t] = A_t …` | Mirrors upstream's roof loop (O `:1138-1166`) | "The roof stack depth is the largest t ≤ R for which every successive area minus the next layer's forbidden region is ≥ 1 mm²", plus the truncation rule in prose |
| 01 §5.5.3 (l. 784-816) | the 10-step description of the support-base connector, e.g. "6. For every remaining chord end, look at its two arcs: if their scores overlap the window [1.3·ls, 2·ls] …" | Prose, but it follows a ~600-line upstream function step for step and names it (`connect_base_support`) | Acceptable as prose only if implementers do not mirror the step order. Better: decide 01 Q1 first. If a chords-only generator with a calibrated margin suffices, delete the 10 steps. |
| 03 §2.4 (l. 180-199) | "for iteration in 0 .. 99: remember every sphere's position as prev … a = clamp(depth + 0.1, 0, 0.5); move S in XY away from q by a² … step = ½·(target − S) … shortened to at most 0.2" | Algorithm-level, but it follows upstream's loop body order (B `:3753-3818`) | Lower risk. Keep the formulas (push a², cap, weights, stop rule) and drop the loop skeleton. |
| upstream names used as our operation names | `moveInsideIfOutside` (02, 6×), `validWithRadius` (02, 3×, an upstream lambda name, O `:1658`), `propagate_down` (02, 2×, upstream member, O `:1601`), `ceilRadius` (02, 03), `pairMerge`, `smooth_outward` (03, 4×) | D §1 checklist: "no upstream identifier names" | Use our names (`nearest_inside`, `radius_valid`, `step_down`, `radius_key`, `try_merge`, `fill_notches`). Keep upstream names only inside citations. |

Formulas, constant tables, version tables and measured data (all of 04, 03 §4-§10, 02 §1-§2, 01 §1-§4) are
fine as written.

---

## 7. Open questions raised by this review

1. **Where does B's run-to-run spread come from?** The candidates S1, S2 (constant per machine), S3 (ruled
   out, appendix A) and S9 (two families only) do not explain 7 L1 families in 53 P1S observations, while
   R, which has the same S1 code, is deterministic. The answer decides whether the ensemble of D §5.4 can
   ever cover B (fix 4, §5.2).
2. **U1 target data.** Can the owner export single-goat S GUI slices (0°/180°, a few runs each)? Without
   them the U1 envelope is unvalidated (fix 5).
3. **Fidelity target.** Is the goal "reproduce upstream realizations" (needs a Clipper-1-compatible
   engine, B int32/10 nm, connector and smoothing ports) or "a sound envelope with a calibrated δ"
   (`i_overlay` is enough, δ larger)? The corpus exceedances (C3) suggest that the second may miss
   P1S ≥ 43 / A2L ≥ 70 at one orientation.
4. **Tower chamfer vs supports** (fix 1): confirm with the owner that "supports may overlap the tower brim"
   means L0 only.
5. **Band boundary rule** (fix 9): which convention should `validate_result` and `verify_outputs` share?
6. **GUI session effect of S9** (01 Q2): should the docs tell the owner to restart the slicer before
   slicing a packed plate, which pins S9 to the CLI families?

---

## Appendix A. Review experiment: can the S3 island guard fire on the goat?

Spec 01 §3.9 / Q4: the overhang guard trips when one layer has more than 100 overhang islands. That would
make the guard a deterministic trigger with a racy switch layer, and a candidate for B's spread and the
225° anomaly. Script: `/tmp/review00/islands.py`. It uses trimesh sections at the 01 §1.2.2 slice planes,
all parts unioned, the extrudable filter X_k, and P_k = X_k ∖ (X_{k−1} ⊕ d_k) with shapely mitre offsets.
Paint and cluster merging are ignored, and the island count is only an approximation of upstream's.

| project (θ) | rotation | max islands on one layer | layers > 100 |
|---|---|---|---|
| P1S (40°) | 0°, 225° | 40 (layer 1); next 14, 14, 10, 9 | 0 |
| A2L (35°) | 0° | 33 (layer 1) | 0 |
| U1 (30°) | 0° | 17 (layer 53) | 0 |

Conclusion: the island trigger does not fire on the goat, so S3 explains neither B's spread nor the 225°
supports. The approximation can differ from upstream by a few islands, not by a factor of 2.5. This
answers 01 Q4 provisionally; the A4 implementation must log the exact count.

# Slicer-free production packer: architecture

Date: 2026-11-02. Status: design, not yet agreed. Scope: the production path of
`colored_3mf_packer` and the packing core. It replaces slicer probes, sample slices, the tower
contest and the mandatory final slice with deterministic models of what the slicer will print.
The slicer stays available as an optional verification step.

Inputs: `/tmp/goat_research/{tree-support-research,zones-research,core-map,pipeline-map,nesting-research,real-plates-analysis}.md`
(cited below as TSR, ZR, CM, PM, NR, RP), the calibration corpus `/tmp/goat_calib/MANIFEST.md` (cited as
M) and the slicer checkouts in `/tmp/goat_src`.
The behaviour specs live in `2026-11-02-organic-support-spec/`: 01 layers, overhangs, tips; 02 volumes,
propagation, merging, placement; 03 smoothing, drawing, first layer, brims; 04 prime tower; 05 landing
(first-layer body). `00-review.md` there is the consistency review; its P0/P1 fixes and the lead's
decisions on it (2026-11-02, §14) are folded into this revision. This document fixes the interfaces those
specs fill, and it marks every version difference and source of nondeterminism that the architecture must
handle.

Slicer source prefixes in this document: **B** = BambuStudio v02.08.02.61 (`bambu_0208`,
`zones_bambu2861`), **O** = OrcaSlicer 2.4.2 (`orca_242`), **R** = OrcaSlicer 2.5.0-rc
(`orca_250rc`), **S** = Snapmaker Orca 2.4.0 (`snaporca240`). Paths are relative to `src/libslic3r/`
unless they start with `slic3r/`.

---

## 0. Decisions

1. **The supports are predicted by our own implementation of the slicers' organic tree behaviour**
   (owner, 2026-11-02, paraphrased: build the supports the way Bambu/Orca do, not line by line, so that the licence holds and the result is the same). The code is
   written from behaviour specs, and it is never a translation of AGPL source (section 1). One
   generator, with a small set of explicit *flavour* switches (B / O / R / S), runs an
   **ensemble** over the slicer's known nondeterminism. The union of the ensemble plus a
   calibrated margin per band is the support envelope. **Fidelity target** (lead, 2026-11-02): a
   behaviourally faithful generator (the same stages, formulas, constants and integer grid per
   flavour: B 10 nm units with 32-bit truncation, O/R/S 1 nm), an ensemble over the *known* axes
   (the exact merge-bucket thread classes of 02 §4.7, the two connector-threshold families S9, and
   tip-insertion interleavings S1 as an optional axis), plus a margin δ calibrated against the corpus.
   B's run-to-run spread has a source nobody has identified yet (§3.2), so δ must cover the corpus
   realizations until WP S4 finds it. Rejected: our own support meshes in the
   3MF (TSR §5.1; this changes the print), blocker fences (they do not work in B/O/S, TSR §0.6),
   and cached slicer probes as the production source (that is not slicer-free; they are kept
   only as an offline calibration tool).
2. **Every object's geometry is split into typed kinds**: body per 2 mm band, landing (first-layer
   body), object brim, support base, support bands and interfaces. The prime tower has a body
   kind (the column plus the chamfer rings of layers ≥ 1, each with its z-range) and an L0 brim
   kind. All collision rules are one **permission matrix** over kinds, coded once. Printer
   profiles declare only geometry and class (hard / restricted / soft).
3. **One plate realization per rotation class.** Identical copies share one support generation,
   and only the first copy in model order is clipped by the bed (ZR §0.1, §5; TSR §2.10). So a
   plate prints exactly one realization per rotation class, translated to every copy. The
   assembler puts the most interior copy of each class first, which makes the free-space
   prediction valid for every copy.
4. **The packing core becomes a deterministic portfolio**: general lattices (with HNF phase
   scoring and the tower anchor folded in), 0/180 double lattices, two-block lattices, the
   existing sweeps, edge fill, and an iteration-budgeted improvement loop. Candidates are
   compared on raster facts. **Only the published winner** gets the independent vector
   validation. Clearance becomes Euclidean (vector buffer, then conservative cover).
5. **The default job makes zero slicer calls.** Full slicing becomes a separate, optional
   `verify` step with GUI-equivalent checks. A time window still needs the slicer for
   estimates, but only small (1- and k-object) slices, never full plates.

Measured headroom (CM §10.2, NR §3.3, RP §5.3, §6): with real realized supports, the lattice
portfolio reaches U1 51-52 / P1S 43 / A2L 69-72 at one orientation, and 55 / 47 / 79 with 0/180.
The binding constraint is the support base against the foreign landing, and the density depends
on the envelope at about 0.7-0.95 objects per mm of edge (NR §3.4). **The calibrated margin δ is
therefore the main risk to the targets** (section 5.5, open question Q1). The corpus makes the risk
concrete: realizations the slicers print lie up to 3.0 mm (first layer) and 4.4 mm (bands) outside a
5-run single-object union for B, and the owner's U1 GUI (S) lies 0.7-4.1 / 1.5-5.4 mm outside the R CLI
union (M §2.3-§2.4, §5.2). P1S and A2L already miss their targets at g + δ ≈ 1.5 mm (RP §5.3). The
targets hold only if the ensemble reproduces most of those realizations, so that δ stays small.

---

## 1. Licensing rule (clean room)

BambuStudio, OrcaSlicer and Snapmaker Orca are AGPL-3.0. The packer is a network service, so
translated code would make it AGPL.

- Behaviour specs describe *what* the slicer computes: prose, formulas, constants and their
  meaning, step order, data flow, edge cases. They cite `file:line` so a reviewer can check the
  behaviour. They contain no pasted or transliterated code.
- Implementers code from the specs. They may read the slicer source to settle a behavioural
  question. They do not copy its structure, identifiers, comments or control flow: our modules,
  names and data types follow this design, not upstream (`descend.rs`, not
  `increase_areas_one_layer`).
- Review checklist item: "no upstream identifier names, no line-by-line correspondence".
- Where exact order matters (AABB reordering, merge rounds, nearest boundary point, list
  compaction, the fill connector), the specs state the result as invariants and order properties,
  and pin it with **recorded test vectors**, not with a step-by-step procedure.
- Operation and field names in the specs and the code are ours: `nearest_inside`,
  `radius_valid`, `step_down`, `radius_key`, `try_merge`, `fill_notches`, `to_bed`, `hold_until`,
  `hint`, … (glossary in 02 §1.4). Upstream names appear only inside citations.
- Existing code that calls itself a port goes through the same review and, if needed, is rewritten
  from a behaviour description: `src/plate_centre/gui_tower.rs` ("ports
  `PartPlate::estimate_wipe_tower_size`") is in WP I3.
- Clipper (Boost licence) and `geo`/`i_overlay` (MIT/Apache) are permissive and may be linked.

---

## 2. Targets, budgets, invariants

| | U1 (270×270) | P1S (256×256, cutter) | A2L (330×320) |
|---|---|---|---|
| target count | ≥ 48 | ≥ 43-44 | ≥ 70 |
| measured with realized supports, 0° (RP §5.3, g = 0.5) | 52 | 43 | 72 |
| measured, 0/180 (RP §6) | 55 | 47 | 79 |

- **Stable count:** the same input gives the same 3MF byte for byte (any thread count; x86 vs arm64
  is a target, verified by the bench), and the count does not depend on run-to-run slicer noise, because nothing is sliced.
- **Time (M3 Max, 8+ threads):** packing stage < 5 s for all three goats, including vector
  validation of the winner. The support model costs < 20 s on a cold cache and ~0 s on a warm
  cache. The whole default job takes < 30 s cold and < 10 s warm. Today it takes 170 s plus slices.
- **Memory:** < 1 GB per job (today 3.3 GB RSS before the final slice; 5-8 GB with it).
- **Safety:** the published layout is safe against the *envelope* (every ensemble realization
  plus δ), within numerical tolerance. This is not a proof for every future realization
  (`docs/packing-safety.md` stays true). Calibration publishes the observed exceedance rate.
- Kept invariants: `validate_result` stays independent of the search's no-fit maps.
  `verify_outputs` stays an independent second opinion. `PrinterProfile` is the only source of
  printer geometry. Effort extends a prefix and never drops an earlier winner. Rayon runs only
  over independent units, with an ordered reduction.

---

## 3. Slicer facts the architecture depends on

### 3.1 Version matrix (only the differences that change the geometry or the acceptance)

| behaviour | B 02.08.02.61 | O 2.4.2 | R 2.5.0-rc | S 2.4.0 | where |
|---|---|---|---|---|---|
| `tree(auto)` + style `default` | organic; **Tree Hybrid if variable layer height** | organic; variable layer height is refused | as O | as O | B `Support/SupportParameters.hpp:143-152`; O `:199`, `Print.cpp:1413` |
| equal-height `layer_config_ranges` collapse into a fixed profile (so they stay organic) | yes | yes | yes | yes | O `Slicing.cpp:244-315` |
| branch distance / angle keys | `tree_support_branch_distance`, `_branch_angle` | `*_organic` keys | `*_organic` | `*_organic` | `Support/TreeSupportCommon.hpp` B:82-83, O:85-86, R:103-104, S:86-87 |
| slow angle, tip diameter, top rate | fixed 25°, 0.8 mm, 30 % | settings | settings | settings | B `TreeSupportCommon.hpp:80-89` |
| `support_critical_regions_only` | keeps cantilevers, sharp tails and overhang ∩ (enforcer grown) | clears plain overhangs, keeps cantilevers, enforced areas and sharp-tail supports | as O | as O | B `Support/TreeSupport.cpp:1216-1250`; O `:1043-1115` (01 §3.7) |
| "fully floating contour ⇒ cantilever" | yes | no | no | no | B `TreeSupport.cpp:1009-1050` |
| merge bucket size | from thread count (S2) | from thread count | **fixed 4** | from thread count | `TreeSupport3D.cpp` B:2238-2244, O:2385-2391, R:2432-2434, S:2389-2395 |
| support-blocker volumes as branch obstacles | no (off-by-one) | no | **yes** | no | `TreeModelVolumes.cpp` B:490, O:472, R:525, S:472 |
| bed outside (and wrapping area) as a routing obstacle | **yes**, in the slicing object's frame | no (clip only) | no | no | B `TreeModelVolumes.cpp:67-88,134`; O `:39-44,67-69` |
| root cap at the bed | hemisphere | **flat foot** | flat foot | hemisphere | O `TreeSupport3D.cpp:2943-2957` |
| smoothing constrained to linked layers | no | yes | yes | no | O `TreeSupport3D.cpp:3156-3202` |
| keep-main connectivity pruning per tree | yes | no | no | no | B `Support/TreeSupport3D.cpp:4933-4960,5075-5164` |
| first-layer support expansion | −1 = auto 2 mm, grown in steps | max(0, e − fine) | as O | as O | `Support/SupportCommon.cpp` B:343,404,448; O:288; R:456; S:359 |
| per-nozzle support exclusion volumes, `bed_exclude_volumes` | no | no | yes | no | R `TreeSupport3D.cpp:3488-3522`; R/main `PrintConfig.cpp` |
| auto-brim width formula (volume groups) | thermal term ×0 | thermal term on | on | on | `Brim.cpp` B:760-806 (`:798`), O:260-306 (`:298`), R:301-347, S:761-806; 03 §5.2 |
| extra 5 mm brim for thin islands | off (always-false guard) | on | on | on | O `Brim.cpp:514-520`, B `:946`; 03 §5.2 |
| no-brim set closed by one brim spacing | yes | no | no | yes | O `Brim.cpp:674` (commented); 03 §5.3 |
| integer grid (`coord_t`) | **10 nm units, 32-bit** | 1 nm, 64-bit | 1 nm, 64-bit | 1 nm, 64-bit | B `libslic3r.h:40,58`; O `:43,60` |
| arc thresholds of the support-base connector | process-static (S9) | static | **per call** | static | B `Fill/FillBase.cpp:2367-2369`; R `:2696-2701`; 01 §5.5.4 |
| organic "support brim" | none (the wide L1 patch is the expansion) | none | none | none | O `Brim.cpp:594,612` (commented) |
| `bed_exclude_area` encoding | groups of 4 points = rectangles | one polygon | one polygon | one polygon | B `slic3r/GUI/PartPlate.cpp:397-413`; S `PrintConfig.cpp:9201-9225` |
| body vs exclusion | per-part 2D convex hull ∩ zone = error (touching OK) | same rule | same | same | B `Print.cpp:944-993`; S `Print.cpp:1390-1440` |
| GUI: toolpath outside plate | centreline bbox (+ arc points) vs printable bbox, ε = 3e-4 | same | same | same | B `BuildVolume.cpp:507-520`, `slic3r/GUI/GCodeRenderer/LegacyRenderer.cpp:1286-1326` |
| GUI: toolpath vs exclusion | every segment, exact | — | — | convex hull of all toolpaths vs exclusion bboxes | B `GCode/BedExcludeChecker.cpp:119-146`, `slic3r/GUI/GCodeRenderer/BaseRenderer.cpp:313-364`; S `slic3r/GUI/GCodeViewer.cpp:2524-2545` |
| GUI: body within 3.5 mm of the edge (spiral lift) | no | no | no | **serious warning** (part bbox, strict <) | S `slic3r/GUI/3DScene.cpp:1381-1401` |
| CLI: toolpath outside | per object bbox + 2 mm, `-104` | + 2 mm, `-102` | as O | — | `GCode/GCodeProcessor.cpp:1845` |
| ConflictChecker collinear-overlap rule | yes | yes | yes | absent | ZR §0.7 |
| tower brim and chamfer | L0 brim + **chamfer always on**, ≤ 3 mm, one loop narrower per layer (5 layers, z ≤ 1.88 at 0.28 mm) | L0 brim only (measured) | L0 brim only | L0 brim + `prime_tower_brim_chamfer` ≤ max width (U1: 8 layers, z ≤ 2.49) | B `GCode/WipeTower.cpp:3706-3735`; S `GCode/WipeTower2.cpp:2303-2345`; 04 §4.6 |
| tower anchor moved by the slicer | GUI on project open: [15, bed − Ŝ − 15], Ŝ = GUI estimate with *n* = the plate's filaments; CLI never | GUI keeps the stored anchor on open (clamps with 1 + brim only on a preset change); CLI never | **CLI and GUI on every slice**: margin 1 + estimated brim (+ cone bulge); snaps when within 5 mm of the range, relocates (object-dependent) beyond | GUI on **every scene reload**, including after each slice with the real depth: margin 1 + `prime_tower_brim_width` (U1: 6 mm) | 04 §6; B `slic3r/GUI/PartPlate.cpp:4808-4895`; R `OrcaSlicer.cpp:6479-6512`; S `slic3r/GUI/GLCanvas3D.cpp:3866-3910` |

### 3.2 Nondeterminism: sources, realizations, coverage

The ids S1-S10 name sources of variation. They are unrelated to the work packages S1-S4 of §12; outside the
§12 tables this document writes those as "WP S1" … "WP S4".

| id | source | flavours | realizations it can produce | status (corpus) | coverage in this design |
|---|---|---|---|---|---|
| S1 | **Tip insertion race.** Parallel per-layer tasks append tips (roof layers insert up to 3 layers lower) into a shared per-layer list under a mutex. First come wins the ~0.04 mm dedup cell. List order drives AABB sorting and merge partner choice (01 §5.7). | B, O, R, S (same code in R: `Support/TreeSupport3D.cpp:1023-1026` in the parallel loop at `:1371`) | any interleaving of the ≤ 4 writer sequences of a layer list; which of two coincident tips survives | **unproven.** R has the same race yet gave 1 realization in 30 rotation-0 observations, full plates included (M §2.4, §5.3). | optional ensemble axis `interleave` (off by default): canonical member = writers in ascending layer order; seeded interleavings from our own PCG on request. Switched on by default only if the discriminating experiment (§11, calibration item C5) shows realizations. |
| S2 | **Merge bucket size** = 4 if ⌊(n+2)/4⌋ ≥ T, else 2, with T = the TBB arena size (02 §4.3) | B, O, S (R fixed to the T = 1 class) | one realization per **T-class**: T values that make the same 2/4 decision on every layer give identical runs (02 §4.7). At most (distinct ⌊(n_L+2)/4⌋) + 1 classes, ≈ 11 for a goat. The owner's 14-thread Mac is in the "all buckets of 2" class unless a layer has n ≥ 54. | constant on one machine, so it explains no run-to-run spread | ensemble axis `t_class`: the **exact** class walk of 02 §4.7 (T := 1; run; T := b + 1 until b = ∞). R: {T = 1} only. T follows the Linux affinity mask (`taskset`, `--cpuset-cpus`); that Docker `--cpus` quotas do not change it is an **assumption** (oneTBB cgroup handling unverified, 02 §7 Q2). |
| S3 | Overhang-detection guard (`Support/TreeSupport.cpp` O:848, B:961, R:990, S:821): once elapsed time exceeds 30 s **or** a layer has more than 100 overhang islands, sharp-tail detection and small-overhang removal stop for later layers. The flag is written from parallel layer tasks, so the switch layer is racy. | all | "guard tripped from layer j" for any j | island trigger ruled out on the goat (≤ 40 islands per layer at every checked angle; review appendix A) | The island-count trigger is deterministic and is modelled exactly; when it fires, the model returns `Unsupported` (§5.7) until a switch-layer ensemble axis exists. The time trigger is not modelled (goat detection takes ≪ 30 s). The report sets `overhang_guard_risk` above a calibrated mesh size. |
| S4 | bed obstacle and clip in the first copy's frame | B obstacle + clip; O/R/S clip | bent or clipped trees copied to every copy | confirmed (ZR §5) | removed by canonical-first ordering (section 9.7) and the hard bed rule for supports |
| S5 | rotation (fill-line angle, square joins, AABB splits) | all | one family per rotation matrix | confirmed: object-frame patches differ by IoU 0.74-0.86 between rotations (M §2.1, §4) | one ensemble per rotation class; rotated envelopes are never reused |
| S6 | flavour / version | — | different generator | U1: S GUI 0.7-4.1 mm (L1) / 1.5-5.4 mm (bands) outside the R CLI union at the same rotation (M §2.4) | flavour resolved per job (section 5.3) |
| S7 | float / libm across CPU architectures | all | rare flips of discrete decisions | not observed | our code uses `libm` and fixed operation order. Upstream's own flips are covered by δ. |
| S8 | full-plate contention (other objects slice at the same time) | B, S (observed) | B P1S CLI plates and samples put up to 26.5 mm² of L1 (2.4 mm out) and 189 mm² of bands (4.4 mm out) outside the 5-single-run union; one plate 3.0 / 3.7 mm (M §2.3, §5.2) | same families as single-object runs (M §2.5: one realization per rotation class per slice) | S2 is constant per machine and S1 is unproven, so contention is **not** explained by a known axis. Covered by δ, calibrated on the corpus plates. |
| S9 | **Process-static arc thresholds** of the support-base line connector (01 §5.5.4): initialised once per process from the first fill's line spacing | B, O, S (R fixed, `Fill/FillBase.cpp:2696-2701`) | two families in a CLI run (thresholds from the roof spacing 0.86 mm or from the base spacing: 5.36 mm B, 1.36 mm O/S); in a GUI session also any earlier slice's spacing | at most two families; does not explain B's spread alone | ensemble axis `arc_thresholds ∈ {roof, base}` (R: one member). GUI session history is not modelled (open question Q20). |
| S10 | process-static closing ε of the multi-region elephant-foot restore (01 §1.5) | all | only after objects with other closing radii in the same process | negligible | none |
| SU | **unidentified B source** | B (and S GUI) | B: 16 line-level realizations in 7 L1 families (≈ 657, 654, 652, 630, 617, 570, 565 mm²) in 53 P1S rotation-0 observations, still growing (M §2.3, §5.3); 4-5 distinct in 5 runs at every B rotation; S GUI: two slices of one 48-goat layout differ (M §2.4) | S1 + S2 + S9 as specified do not explain it; S3 is ruled out | **δ must cover every corpus realization** (§5.5). WP S4 investigates the source (calibration items C5-C7, §11). |

Not sources (TSR §3.1, 02 §4.7, 03 §8): Jacobi-style smoothing, memoized volume caches, membership-only hash
sets, the per-element moves and the merge buckets (disjoint writes), per-tree drawing.

---

## 4. Geometry kinds per object

### 4.1 Frame and bands

- **Footprint frame:** the source item's full transform (rotation, scale, mirror) is applied,
  then the frame is centred on the XY bbox centre of **all normal parts**. Fixes from PM §0.5 and
  §9.5: the first-component-only mesh and the dropped source rotation for clones.
- A **rotation class** θ is an extra Z rotation about that centre. Classes in use: {0°} or
  {0°, 180°} (option `--rotations`). Support-free objects keep today's free canonical frame and
  any angle (`frame.rs`).
- **Bands:** absolute index k covers z ∈ [k·2 mm, (k+1)·2 mm) (`LAYER_BAND_MM`). Empty and
  one-sided bands keep their index (current invariant).
- **Band rule (one rule for every kind; lead, 2026-11-02).** Printed layer l occupies the interval
  (z_{l−1}, z_l], with z_{−1} = 0 and z_l its print_z. Its material belongs to **every band the
  interval overlaps with positive length**: band k iff z_{l−1} < 2(k+1) and z_l > 2k. A layer whose top
  lies exactly on a band boundary belongs only to the lower band. Example (P1S grid): the layer with
  print_z 2.16 occupies (1.88, 2.16] and belongs to bands 0 and 1, so it is tested against a foreign
  body top at z = 1.95 in band 0. Bodies follow the same rule: `body[k]` is the union of the printed
  sections of the layers in band k, which the mesh slab [min z_{l−1}, max z_l] over those layers
  contains (§4.2). `validate_result`, `verify_outputs` and `realization_from_gcode` share this rule
  (one table test). The repo's present `floor(print_z / 2)` binning (`learning.rs`, the corpus'
  `derive.py`) differs at band edges; corpus band polygons are re-binned before calibration.
- **First layer:** "first layer" means the printed layer 0, interval (0, h0]. Its material is in
  band 0 **and** in its own first-layer kinds (`landing`, `support_base`), which carry the precise
  first-layer tests. The band copy makes band 0 the conservative superset.

### 4.2 Kinds

| kind | definition (material extent, line width included) | producer | used by |
|---|---|---|---|
| `body[k]` | projection of every normal part's material inside the slab [min z_{l−1}, max z_l] over the layers l of band k (§4.1 band rule; band k extended outward to whole layers), as a union of slab-clipped triangles, not sampled sections | `mesh_section` | body/body (option), support/body pairs, tower body (chamfer bands) |
| `silhouette` | ∪ `body[k]` | `mesh_section` | body/body (default), restricted/hard bbox tests |
| `part_hulls` | 2D convex hull per normal part | `mesh_section` | hard keepouts (the slicer's exclusion rule) |
| `part_boxes` | XY bbox per normal part | `mesh_section` | printable bounds, restricted band |
| `landing` | first-layer body (05): the union of the normal parts' sections at z = h0/2, grown by ε_L = 0.05 mm (classic walls) or max(0.05, ½·(min_bead_width − min_feature_size)) (Arachne, unmeasured). Elephant-foot compensation is ignored: it is applied only on wide features, and the corpus prints within 0.026 mm of the section in all 120 slices (05 §3) | `first_layer` (05) | support base vs foreign landing, soft zones, tower brim |
| `brim` | object brim ring: `brim_type`, `brim_width`, auto rule (03 §5); empty for the goats (auto rule gives 0; probes have no Brim feature, PM §3.2) | `first_layer::brim` | keepout reporting only (the slicer trims brims, ZR §0.6) |
| `support_base` | first-layer support: feet grown by the expansion in steps, each step trimmed by the slicer's **own uncompensated layer-0 section grown by `support_object_first_layer_gap` with square joins** (03 §4.2, B7), not by the conservative `landing` | `support_model` | support base vs foreign landing; hard zones; tower body (L0 column) |
| `support[k]` | support material of every printed layer in band k (§4.1 band rule; band 0 includes the layer-0 base) | `support_model` | support vs foreign `body[k]`; hard zones; tower body incl. chamfer rings |
| `interface[k]` | the part of `support[k]` printed as interface (role kept for diagnostics and the strict option) | `support_model` | the same as `support[k]`; `--interfaces-as-body` moves it into `body[k]` |
| `tower_body` | plate-level, z-ranged pieces: the tower column on every layer up to the tower top, **plus the chamfer rings of layers ≥ 1** (B: 5 layers, z ≤ 1.88; S: 8 layers, z ≤ 2.49; 04 §4.6), each ring with its own layer interval | `tower_model` | hard for bodies and supports in every band its pieces meet |
| `tower_brim` | plate-level: the **L0 brim ring only**, z ∈ (0, h0] | `tower_model` | allowed for support base and object brims (owner); hard for landings |

The skirt is not modelled. `skirt_loops > 0` is an explicit error, since the slicer does not clip
it (ZR §3.4; open question Q11).

### 4.3 Data type

```text
ObjectPrint            // one rotation class, footprint frame, mm, even-odd loops
  rotation_deg
  silhouette, part_hulls, part_boxes, landing, brim
  body:      Vec<Loops>   // index = absolute band, §4.1 band rule
  support_base: Loops     // layer 0 only (also contained in support[0])
  support:   Vec<Loops>   // index = absolute band, §4.1 band rule
  interface: Vec<Loops>
  provenance: { flavour, members, margins, cache_key }
```

`Footprint` (today's type) stays as the legacy serialized form. One adapter maps a v1
`Footprint` to an `ObjectPrint` (`loops − body` → `support_base`, `body_loops` → `silhouette`,
`support_landing_loops` → `landing`, `layer_zones` → bands), so old `--finish-state` files
still load and the slicer-learned mode can feed the new core.

---

## 5. Support model

### 5.1 Interface (plain functions, no trait)

```text
support_model::realizations(input: &SupportInput, schedule: &Schedule) -> Vec<Realization>
support_model::envelope(reals: &[Realization], margins: Margins) -> SupportEnvelope
gcode_footprint::realization_from_gcode(..) -> Realization     // verification / calibration only

SupportInput  { mesh: &ObjectMesh (all parts, paint), settings: &SupportSettings,
                rotation: Rotation3, flavour: Flavour }
Realization   { member: MemberId, base: Loops, support: Vec<Loops>, interface: Vec<Loops> }
```

A trait would add indirection with only one generator. The second source of realizations
(sliced G-code) produces the same `Realization` value, so the envelope builder does not care where
realizations come from.

### 5.2 Stages

Stage A runs once per (mesh, paint, settings, flavour, rotation) and is deterministic.
Stage B runs once per ensemble member. Our module names:

| stage | module | what it computes (behaviour spec gives the rules) |
|---|---|---|
| A1 | `layers` | the slicer's layer grid: first-layer height, fixed layer height, collapsed equal ranges; slice z at mid-layer. Variable layer heights are an explicit error ("Bambu switches to Tree Hybrid / Orca refuses"). |
| A2 | `sections` | per-layer object area (all parts), closing radius, extrudable filter (features narrower than a line width drop out) |
| A3 | `paint` | decode the painted enforcer/blocker facets (`paint_supports`) and project the down-facing ones per layer |
| A4 | `overhangs` | overhang areas by threshold angle, cantilevers, sharp tails, small-overhang removal, critical-only filter, enforcer union with its fixed grow; flavour switches from §3.1 |
| A5 | `clearance` | per layer and radius class: collision (own area grown by radius + xy, with z-gap layers), avoidance to the bed (slow, fast, fast-safe), placeable areas. B: the bed-outside ring is an extra obstacle. |
| B1 | `tips` | tip seeding in overhang and roof areas (line pattern at branch spacing, resampled at the connect length; the connector's arc thresholds from the member's S9 family), insertion layer offsets, first-come dedup **in the member's order** (S1: canonical unless the optional axis is on) |
| B2 | `descend` | influence areas layer by layer downward (slow, then fast, then the fallback), radius growth, foot growth near the bed |
| B3 | `merge` | per-layer merging with the member's T-class bucket rule (S2), destination by list order |
| B4 | `place` | node positions bottom-up (lazy: nearest valid point to the child) |
| B5 | `smooth` | organic smoothing: collision push-out plus Laplacian; anchored variant for O/R; B: keep-main pruning |
| B6 | `draw` | branch discs per layer; root cap (hemisphere B/S, flat O/R); interface roles; bed clip in the object frame |
| B7 | `base` | first layer: feet grown by the expansion in n steps, re-trimmed each step by the slicer's own layer-0 object area from A2 grown by the first-layer gap. Not the conservative `landing` kind: a larger trim mask would under-predict the base. |
| B8 | `bands` | per-layer material into `base`, `support[k]` and `interface[k]` (line width included) |

Rules:
- single-threaded inside a member; `libm` for tan, sin and sqrt; `Vec`/`BTreeMap` only, no
  hash-map iteration.
- **The integer grid is a flavour property** (02 §1.1): B uses 10 nm units with 32-bit
  coordinates (1 mm = 10⁵, `libslic3r.h:40,58`); O/R/S use 1 nm with 64-bit coordinates. Every
  mm → integer conversion truncates toward zero on the flavour's grid, so B's speeds, radii and cells
  differ in the last unit (V_f = 27 999 units = 0.27999 mm; r_b = 99 999 units because 2.0/10⁻⁵
  truncates to 199 999; tip seed circle 15-gon in B vs 45-gon in Orca; 01 §4, 02 §1.2).
- **Polygon backend** (lead, 2026-11-02): pure Rust `i_overlay` behind the facade
  `polygon_ops.rs`, which emulates Clipper-1 semantics wherever they change a discrete decision:
  the miter-limit floor of 2 (a requested 1.2 acts as 2), the arc-tolerance rule
  (τ' = min(τ, |d|/4), π/acos(1 − τ'/|d|) segments per turn, capped at π·|d|), the 0.005·|d| input
  decimation of offsets, the strictly-simple union for B's simplification, and the flavour grid
  and truncation. The facade stays swappable (a Clipper-1-compatible engine can replace
  `i_overlay` without touching the stages). WP S4 reports whether backend differences, measured as
  envelope differences between the two engines on the corpus inputs, dominate δ.
- Unsupported settings return an explicit error before stage A (§5.7).

### 5.3 Flavours

`Flavour` is a small value type (`family`: Bambu / Orca / SnapmakerOrca; `version`), matched at
the few points listed in §3.1 (flavour switches are slicer behaviour, so they are legitimate code
branches; printer differences never are). It is resolved per job in this order:

1. `--slicer-flavour` (explicit);
2. the source 3MF `Application` metadata when it parses to a known family and version (the goat
   projects say `BambuStudio-02.08.02.61`, `Snapmaker_Orca-2.4.0`). Orca forks that write
   "BambuStudio" are caught by the version shape and the profile's `slicer_kind` (CLAUDE.md: the
   kind still comes from `printer_model`);
3. the profile's `gui_flavour` (catalog default per vendor);
4. otherwise the nearest known version of the same family, with `flavour_approximated: true` in
   the report and the flavour's widest calibrated margins.

Option `--also-flavour <f>` packs against the union of two flavours (U1: owner's Snapmaker GUI
plus the server's vanilla Orca, Q2).

### 5.4 Ensemble schedule (our own pseudo-code)

The core ensemble is an **exact, finite enumeration** of the known axes: every T-class (S2, 02 §4.7)
for every connector-threshold family (S9, 01 §5.5.4). It needs no saturation rule. The tip-order axis
(S1) is optional because the corpus has not shown that it produces realizations (§3.2).

```text
families(flavour) = flavour.static_arc_thresholds ? [Roof, Base] : [PerCall]     # S9; R: PerCall

core_members(flavour):                     # one class walk per family; families run in parallel
  for fam in families(flavour):
      T = 1
      loop:
          r = stage_b(T, fam, Order::Canonical)      # canonical tip order = writers ascending
          emit (T-class (a, b], fam, r)              # (a, b] from r's per-layer bucket counts
          if flavour.fixed_bucket or b == ∞: break   # R: only T = 1
          T = b + 1

optional_members(flavour, k):              # S1, only with --tip-order-members k (default 0)
  for seed in 1..=k:                       # our own PCG, fixed seeds
      for (class, fam) in classes found by core_members:
          emit stage_b(class.representative_T, fam, Order::Seeded(seed))

run:                                       # deterministic for any core count
  reals = core ∪ optional, collected in (family, T, seed) order, deduplicated by hash
  envelope = union of reals per kind and band; then ⊕ δ (§5.5)
```

- Cost: at most (distinct ⌊(n_L+2)/4⌋ + 1) runs per family, ≈ 11 for a goat, so ≈ 22 stage-B runs
  per rotation class for B/O/S and 1 for R. Each (rotation class, family) walk is sequential, and the
  walks are independent, so rayon runs them in parallel (4 walks for {0°, 180°}). The < 20 s cold
  budget (§2) then holds at ≤ 1.5 s per stage-B run on 4 or more cores.
- The optional S1 axis keeps a result-only stop rule (no new area ≥ `SATURATION_MM2` = 1 mm² in any
  band for `QUIET_MEMBERS` = 8 members, cap `MAX_MEMBERS` = 96). The envelope is identical on every
  machine because nothing depends on time.
- `--slicer-threads T` may restrict the core to the class of a known slicing machine. Default: all
  classes, because the printing machine's T is unknown (owner GUI 14, server unknown).

### 5.5 Envelope and margins

- `SupportEnvelope` per rotation class: `base = (∪ base) ⊕ δ_L1`, `support[k] = (∪ support[k]) ⊕ δ_band`,
  and the same for `interface[k]`.
- **δ is not the pair gap.** δ covers model uncertainty: numerical differences from upstream
  (backend, grid), S3, S7, S8, the unidentified B source SU, and every other realization the
  ensemble missed. The pair gap g (support to foreign body,
  `SUPPORT_BODY_GAP_MM` = 0.5, renamed from `LAYER_ZONE_GAP_MM`) is the physical "never touch"
  clearance the owner asked for. Both apply.
- δ is per flavour, kept as constants in `support_model/flavour.rs`, each with an evidence comment
  (like `SLICER_KEEPOUT_SAFETY_MM`). It is not in profiles, because it describes the slicer, not
  the printer.
- **Risk, restated from the corpus (M §2.3-§2.4, §5.2).** The earlier initial values (δ_L1 = 1.0,
  δ_band = 1.5 mm) came from one plate lying 0.81 / 1.18 mm outside a 20-run union (TSR §4.4). The
  corpus shows far more:

  | flavour | what lies outside what | first layer | bands |
  |---|---|---|---|
  | B (P1S) | packer CLI plates and 3-object samples vs the 5-run single-object union, rotation 0 | up to 26.5 mm², **2.4 mm** | up to 189 mm², **4.4 mm** |
  | B (P1S) | worst single plate (`goat_p1s/run2 final_first`) | 47 mm², **3.0 mm** | 146 mm², 3.7 mm |
  | B (P1S) | owner GUI 40-goat plate vs the same union | 17.8 mm², 0.85 mm | 104.5 mm², 2.13 mm |
  | S vs R (U1) | owner Snapmaker GUI plates vs the R CLI 5-run union | 17-55 mm², **0.7-4.1 mm** | 171-340 mm², **1.5-5.4 mm** |
  | R (U1) | CLI plates vs the R 5-run union | 0.0 | ≤ 0.02 mm |

  If the ensemble reproduces none of the extra realizations, δ must exceed these distances, and the
  targets fail: P1S and A2L already miss at g + δ ≈ 1.5 mm (RP §5.3). The targets therefore depend on
  the ensemble reproducing the corpus families (fidelity target, §0 decision 1), and on finding SU
  (§3.2).
- **Initial values until the first calibration** (cover the corpus distances above): B δ_L1 = 3.1 mm,
  δ_band = 4.5 mm; S δ_L1 = 4.2 mm, δ_band = 5.5 mm (from the S-vs-R row, the only S data); R δ_L1 =
  δ_band = 0.1 mm; O: as S until the O 2.4.2 support slices of §11 exist. They cover the corpus only if
  the model envelope contains the 5-run unions; the first calibration against the model replaces them.
  They are known to miss the targets. **Goal after calibration: δ_L1 ≤ 0.3, δ_band ≤ 0.5**, reachable
  only if the exceedances measured against the *model* envelope (not the 5-run union) shrink
  accordingly.
- Calibration rule: δ = maximum observed exceedance of any recorded realization outside the
  ensemble union, per kind and band, over the campaign, + 0.1 mm. Exceedance is measured on the
  outer boundary of the footprint (`support_patch_polygon` per layer and per band, Hausdorff of the
  boundary), not on line-level identity, so toolpath-only differences (S9 in the base fill, B's
  lightning void fill) do not count. Every corpus realization takes part, full plates and GUI
  exports included. The report publishes the sample size and the realization count per flavour.
- The U1 target flavour S has no single-object calibration data (§11, item C1). Until it exists,
  S's δ comes from the S-vs-R row above and the U1 envelope counts as unvalidated in the report
  (`support_model.validated: false`).

### 5.6 Cache

Key = SHA-256 over: mesh bytes of all normal parts, paint strings, every support-relevant
setting (a fixed key list in `support_model/settings.rs`), flavour, rotation matrix, and
`MODEL_VERSION` (bumped on any behaviour change). Value = the `SupportEnvelope` plus the realization
hashes, as JSON in `--cache-dir` (default: none, so CLI runs are pure). The webapp passes a
persistent directory (Q9). A cache hit skips stages A and B.

### 5.7 Unsupported settings (lead, 2026-11-02)

The model specifies only what the parts describe. For anything else it returns
`Unsupported { setting, value, reason }` before stage A; it never guesses. The pipeline then falls
back to `--support-source slicer` when the job allows slicing (webapp default), or fails with the
setting named in the error (`--support-source model` explicitly, or no slicer configured). The report
records `support_source: "slicer"` and `unsupported: [<setting>, …]`.

| setting / condition | why | spec |
|---|---|---|
| variable layer height (non-fixed layer profile), `precise_z_height = 1`, Z anti-aliasing (`zaa_enabled`) | B switches to Tree Hybrid, Orca refuses; slice planes change | 01 §1.2 |
| S mixed-filament gradient / Z-dithering modes | rewrite the layer ranges | 01 §1.2.1 |
| `raft_layers > 0` | other first-layer and z-gap rules | 01 §4, 03 §4 |
| `support_on_build_plate_only = 0` (support on model) | to-model volumes and placement are unspecified | 01 §4, 02 §2.6, §5.3 |
| support style or type other than organic tree (`normal`, `tree_hybrid`, `tree_slim`, `tree_strong`, snug, …) | different generators | 01 §1.2.1 |
| soluble or zero-gap interfaces; `independent_support_layer_height = 1` | other roof and z-gap rules | 01 §4 |
| honeycomb or lightning support base pattern | other tip-line generators | 01 §4 |
| non-default `xy_contour_compensation`, `xy_hole_compensation`, B circle compensation, `make_overhang_printable` | change the section L_k | 01 §1.4, 05 §2.3 |
| `slicing_mode ≠ regular` | other fill rule | 01 §1.3 |
| objects whose parts have disjoint XY boxes on a layer | O/S/R fast-path behaviour unverified | 01 §1.4, Q5 |
| support blockers (volumes or paint) in R | blocker obstacles are only sketched | 02 §2.3 |
| `detect_thin_wall = 1` (classic walls), `fuzzy_skin` on layer 0 | landing overshoot unmeasured | 05 §2.3 |
| support or interface filament ≠ 0 | the tower depends on the tree realization | 04 §7 N1 |
| `skirt_loops > 0` | the skirt is not modelled (Q11) | §4.2 |
| an overhang guard island count > 100 on some layer | S3 would become an ensemble axis that is not implemented yet | 01 §3.9 |

`max_bridge_length > 0` is accepted and recorded in the report (bridge removal is not modelled;
evidence of no effect on the goat: U1, 01 §3.7.1; general rule open, 01 Q6).

---

## 6. Landing, brim and tower models

- `first_layer::landing(sections, settings, flavour)` (behaviour spec 05): the union of the normal
  parts' sections at z = h0/2, grown by ε_L = 0.05 mm for classic walls, or by
  max(0.05, ½·(min_bead_width − min_feature_size)) for Arachne (unmeasured, reported). Elephant-foot
  compensation is ignored: the slicer switches it off locally on narrow features, where the print
  reaches the section (05 §2.1). Corpus: every printed first layer (3 printers × 8 rotations × 5 runs)
  lies within 0.026 mm of the section, and 0.19 mm outside the nominally compensated outline at one
  narrow corner (05 §3).
- `first_layer::brim(landing, settings, flavour)`: `brim_type` (`auto_brim` uses the
  volume-group width rule; B multiplies the thermal term by 0, O/R/S use it; the thin-island extra
  brim exists only in O/R/S; the no-brim set is closed by one spacing only in B and S; 03 §5.2-§5.3,
  §3.1 rows), `brim_width`, `brim_object_gap`, brim ears. It is 0 for the goats in every flavour.
  Output is used for reports and the soft class only.
- `tower_model::printed_tower(settings, plate, flavour) -> TowerPrint { body: Vec<(z_range,
  outline)>, brim_l0: outline, gui_anchor_range }` (behaviour spec 04). `body` holds the column
  outline per layer range and the **chamfer rings of layers ≥ 1** (B always, ≤ 3 mm; S with
  `prime_tower_brim_chamfer`, ≤ `prime_tower_brim_chamfer_max_width`), each with its own z-range;
  they are tower *body* for the matrix. `brim_l0` is the first-layer brim only. Inputs: prime tower
  width, rib wall, rib width and extra length, infill gap, brim width (−1 = auto by height), filament
  prime volumes or flush matrix, the per-layer filament changes (derived from mesh paint and layer
  ranges), and the tower top H_t. It replaces the probe-measured keepout. The tower does not grow
  with object count (§14 Q3, 04 §8.2); the 0.3 `TOWER_GROWTH_RESERVE` is dropped once the model
  matches the measured towers (04 §1: rectangles exact, Bambu rib towers within 0.093 mm).
- `gui_anchor_range(flavour, tower)` (rewritten in WP I3 from 04 §6; it replaces the 1 mm Orca-family
  margin of `src/plate_centre/gui_tower.rs:11-13` and `CLAUDE.md:36`):
  - B: x ∈ [15, bed_x − Ŝx − 15], y ∈ [15, bed_y − Ŝy − 15], Ŝ = the GUI estimate (04 §6) with *n* =
    the plate's filaments (counting all project filaments, as the repo does, errs safe). Applied by the
    GUI when the project is opened; the CLI never moves the anchor.
  - S: margin m = 1 + `prime_tower_brim_width` (U1: 6 mm). x ∈ [m, x_max − W0 − m], y ∈ [m, y_max −
    max(D̂, D) − m], with D the **real** printed depth, because S re-clamps on every scene reload,
    including after each slice; x_max/y_max = the bed-shape bbox maximum (U1 270.5 / 271, inferred,
    04 Q2).
  - R: margin 1 + b̂ (estimated real brim n_0·s_0, U1 4.91 mm) + cone bulge, applied by the CLI and the
    GUI on every slice; an anchor within 5 mm of the range is snapped, one farther out is relocated
    depending on object positions. The packer keeps anchors inside the range, so R never moves them.
  - O: no clamp on open or CLI slice; the packer still keeps anchors inside [1 + brim, …] so that a
    preset change does not move the tower.
  The tower anchor is a packing variable (section 9.3), so the tower contest and
  `clear_tower_of_patches` go away.

---

## 7. Keepout classes and the permission matrix

### 7.1 Plate zone classes (profile data)

| class | meaning | examples |
|---|---|---|
| `hard` | nothing may print there; copies' supports are not clipped by the slicer | outside the printable area (implicit), `bed_exclude_area` (P1S cutter), `bed_exclude_volumes` (R, z-limited), enabled wrapping area, unknown macro-purge edge band |
| `restricted` | the model body may not enter; supports and brims may | Snapmaker GUI 3.5 mm spiral-lift band; any profile-declared body-only band or rect |
| `soft` | only first-layer body (and the tower body's L0) may not cover it; brims and supports may | purge or prime lines (`erCustom`, no slicer check, ZR §0.5), X1 calibration strip |

Plate-level kinds `tower_body` and `tower_brim` act as two more zones whose position is a packing
variable. `tower_body` is z-ranged: the column on every tower layer plus the chamfer rings on layers
≥ 1 (each ring with its own layer interval). `tower_brim` is the L0 brim ring only, z ∈ (0, h0]
(lead, 2026-11-02; 04 §9.2).

### 7.2 Kinds against plate zones (in code: `keepout/matrix.rs`)

Every zone has a z-range (default: all z). A kind in band k is tested only against zones whose
z-range overlaps band k (§4.1 band rule, the same positive-length overlap). First-layer kinds
(`landing`, `support_base`) are tested against the zones whose z-range meets layer 0. The silhouette
(all z) is tested against the union of a zone's z-ranges.

| kind | printable boundary (hard) | hard zone | restricted | soft (z ≤ z_max) | tower body (column + chamfer rings, by z) | tower brim (L0 only) |
|---|---|---|---|---|---|---|
| body / silhouette | each part box inside the printable area (ε 1e-4) | **part hulls** must not overlap (touching allowed) | each part box at distance ≥ width from the edge (equality allowed) | allowed | forbidden, gap c (silhouette: against the column ∪ every chamfer ring; `body[k]`: the pieces in band k) | covered by the landing row |
| landing | (covered by the body rows) | (covered) | (covered) | **forbidden**, gap 0 (the overshoot is in the landing) | forbidden (L0 column) | **forbidden** |
| brim | — (the slicer clips) | — (clipped) | allowed | allowed | — (trimmed) | — (trimmed) |
| support base | material inside the printable area inset by `print_edge_reserve_mm` | **forbidden** | allowed | allowed | forbidden, gap g (L0 column) | **allowed** (owner; GUI warning, CLI conflict without `--no-check`, ZR §0.7) |
| support[k] / interface[k] | same as the base | forbidden | allowed | allowed | **forbidden, gap g**, including the chamfer rings in every band they meet (B: band 0; S: bands 0-1) | allowed (only band 0 meets it, through the layer-0 base) |
| tower body | inside the printable area | forbidden (validate error) | allowed | forbidden at L0 | — | — |
| tower brim | inside the printable area (the GUI counts tower lines) | forbidden | allowed | allowed | — | — |

Consequence of the band rule: `support[0]` contains the layer-0 base and is tested against the chamfer
rings of band 0. So a support base may overlap only the part of the L0 tower brim that lies outside the
layer-1 chamfer ring (P1S/B: the outer 2 of 7 loops, ≈ 0.9 mm; U1/S: the outer 3 of 11 loops, ≈ 1.4 mm).
This is conservative. A per-layer test of support against the chamfer could recover the rest later.

Where the slicer is looser, the rule follows the owner and the physics. Example: the CLI accepts a
copy's support 2 mm off the bed, but copies are not clipped, so the GUI would refuse the plate.

### 7.3 Kinds against foreign kinds (pairs; symmetric, applied both ways)

| A | B | rule | gap |
|---|---|---|---|
| silhouette | silhouette | forbidden (default) | c = `--clearance` (0.7) |
| body[k] | body[k] | forbidden (option `--body-rule bands`; physically valid for FDM) | c |
| support base | landing | forbidden | g |
| support[k], interface[k] | body[k] | forbidden | g |
| support kinds | support kinds | **allowed** | — |
| brim | anything | allowed (the slicer trims brims around foreign bodies and L0 support islands; brims may overlap) | — |

`support_overlap_mm` (support cores) stays, with cores built from `support_base`.
`--interfaces-as-body` (strict, legacy) moves interfaces into the body kind. `PackPlan::pack_at`'s
"pack under both rules" is dropped: it treated the greedy search's non-monotonicity, which the
portfolio removes (CM §0.6).

### 7.4 Profile schema (backward compatible, printer-agnostic)

```json
"bed_rules": {
  "body_edge_band_mm": 3.5,
  "print_edge_reserve_mm": 0.0,
  "keepouts": [
    { "id": "cutter", "class": "hard", "rect": [0, 0, 18, 28], "source": "bed_exclude_area" },
    { "id": "purge", "class": "soft", "frame": "gcode", "z_max_mm": 0.2,
      "strips": [[18, 1, 240, 1, 0.81], [240, 1, 240, 11, 0.84]],
      "source": "machine_start_gcode" },
    { "id": "macro_purge", "class": "hard", "edge_band_mm": 5.0, "source": "start_gcode_macro" }
  ]
},
"gui_flavour": "bambu-02.08"
```

- A keepout's shape is exactly one of `rect`, `polygon`, `strips` (`[x1,y1,x2,y2,width]`) or
  `edge_band_mm`. `z_range_mm` is optional. `frame: "gcode"` means plate = G-code + the project's
  `extruder_offset` (P1S (0, 2)).
- Class semantics live in code (§7.2), never in the profile. A profile cannot redefine what
  "soft" allows.
- `body_edge_band_mm` is a restricted band along the printable edges. The effective band is
  max(profile value, flavour GUI band); the flavour band is 3.5 mm for S on rectangular beds.
- `print_edge_reserve_mm`: support and tower material stays inside the printable area inset by
  this value. 0 is about w/2 stricter than the GUI centreline rule.
- Old fields (`edge_margin_mm`, `support_edge_margin_mm`, `exclusion_rectangles`, `purge_lines`)
  are still read, written and kept consistent, because the webapp and `source_project
  --edge-margin` use `edge_margin_mm`.

### 7.5 Legacy mapping (profiles without `bed_rules`)

| old fields | new rules |
|---|---|
| `edge_margin_mm` m, no `support_edge_margin_mm` | hard `edge_band_mm` m (binds everything, as today) |
| `edge_margin_mm` m + `support_edge_margin_mm` s | restricted band m + `print_edge_reserve_mm` s |
| `exclusion_rectangles` | hard rects, plus `LEGACY_EXCLUSION_GROWTH_MM` = 4.0 kept until the offline full-plate audit near the P1S cutter passes. CLAUDE.md: do not lower it from single-object evidence. The exact hull rule and the "copies are not clipped" rule (ZR §0.1, §0.3) explain the old conflicts. |
| `purge_lines` | soft strips, frame `gcode`, `z_max_mm` = the project's first-layer height |

### 7.6 How `printer_catalog` derives it

- `extract`: printable area; `bed_exclude_area` → hard keepouts (vendor BBL: groups of 4 points
  = rects; Orca vendors: one polygon, decomposed with the existing `exclusion_rects`);
  `bed_exclude_volumes` (R and later) → hard with `z_range_mm`; `wrapping_exclude_area` when
  `enable_wrapping_detection = 1` → hard.
- `qualify`: `purge_detect` on the qualification probe → soft strips. The width comes from the
  extruded volume, w = E·A_filament/(L·h), not the nominal line width (P1S 0.81-0.84,
  U1 1.8; ZR §8). `z_max_mm` = purge Z. Firmware purges that are not visible (`PRINT_START`,
  `START_PRINT`, `G130`) → hard `edge_band_mm` 5 with `source: start_gcode_macro`, the same rule
  as `annotate` today.
- `annotate`: `gui_flavour` from the vendor's GUI (BBL → Bambu, Snapmaker → Snapmaker Orca,
  others → Orca); `body_edge_band_mm` 3.5 when the GUI is Snapmaker Orca and the bed is
  rectangular; the X1 family (`scan_first_layer = 1`) → soft rect [18,0,240,15] (Bambu's own
  "brim may grow over it" region, ZR §4).
- The tower gets nothing in the profile; it comes from the flavour plus project settings.

### 7.7 Migration of the hand-made profiles (no `source` block, edited by hand)

| | U1 `snapmaker_u1.json` | P1S `p1s.json` | A2L `bambu_a2l.json` |
|---|---|---|---|
| `body_edge_band_mm` | 3.5 (was `edge_margin_mm` 4.0; S GUI rule, equality passes) | 0 | 0 |
| `print_edge_reserve_mm` | 0.0 (was `support_edge_margin_mm` 0.7; plates printed supports 0.33-0.45 mm from the edge, RP §3.3; δ covers drift) | 0 | 0 |
| hard | — | cutter `[0,0,18,28]` | — |
| soft | purge `[85,1,185,1,1.8]`, gcode frame, z ≤ 0.25 | 4 strips, widths 0.81 / 0.84, gcode frame, z ≤ 0.2 | `[105,1,185,1,2.0]` (the firmware `G130` line is near y −0.2; kept conservative), z ≤ 0.2 |
| `gui_flavour` | `snapmaker-orca-2.4` | `bambu-02.08` | `bambu-02.08` |
| open | `PRINT_START` may prime other toolheads somewhere (Q7) | — | real `G130` geometry (Q7) |

Measured value (PM §0.3, CM §10.2): purge strips as soft plus no edge reserve take P1S
39 → 43 with today's core; U1's purge as soft gives 46 → 48.

---

## 8. Raster and collision engine

1. **Euclidean clearance.** For each kind mask that takes part in a pair with gap G, buffer the
   vector polygon by G/2 with round joins, then rasterize with `cover` (centre fill plus every
   boundary-crossed cell). At an integer shift, two such masks always overlap when the
   true distance is < G (no false negatives); they may also overlap up to G + √2·res (0.35 mm
   extra at 0.25 mm). This replaces the square kernel (CM §2: 0.7 mm became ≥ 1.0 mm axial and 1.41 mm diagonal, which penalized
   exactly the diagonal lattices that win). It does not change the conservative coverage invariant.
2. **Masks per rotation class** (`raster_mask.rs` → `kind_masks.rs`): `silhouette⊕c/2`,
   `landing⊕c/2`, `landing⊕g/2`, `base⊕g/2`, `support[k]⊕g/2`, `body[k]⊕g/2`, undilated
   `part_hulls`, `support_all` (for zones) and `core` when `support_overlap_mm` is set. Masks are
   built once per rotation and shared by every member.
3. **Zone pairs from the matrix.** `collision::zone_pairs` returns the list built by
   `keepout::pair_rules()` (§7.3) instead of hard-coded pairs. `no_fit` is unchanged (span
   Minkowski difference).
4. **Feasible origins per rotation** (`FreeMap::base`): AND over (zone, kind) entries of §7.2.
   Rect and box tests (printable bounds, edge bands, part boxes) are analytic origin ranges. Polygon
   zones are rasterized with `cover` and Minkowski-subtracted with the same span code (a static
   obstacle as "A"). The tower is a separate removable layer of this map (§9.3).
5. **Validation split** (`validation.rs`):
   - `validate_raster(layout)` re-rasterizes independently (cached by (rotation, sub-pixel offset);
     grid placements share offset 0) and checks every pair and zone. It runs for every candidate
     that reaches the shortlist.
   - `validate_vector(layout)` runs **only for the winner**. Zone polygons are built once per
     rotation in the footprint frame (rings XOR-ed once) and translated per placement. Pairs are
     culled by bbox and taken from a spatial hash, then checked in parallel with rayon and reduced
     in order. It checks the exact distances G and the exact class rules (hull ∩ zone, part box
     distances, strip distances). Measured today: 2-7.5 s per call, 25-35 calls per job. Target:
     one call, ≤ 0.5 s.
   - If the vector check fails after the raster check passed, that is a bug signal: it is logged,
     the next candidate is taken, and the report records it.
6. `placement_is_allowed` (public, test-only semantics, CM §5) is rewritten on top of the feasible
   map so that the tests check the production predicate.
7. `verify_outputs` gets its own copy of the matrix table (deliberate duplication so that it shares
   no packing code). A test asserts the two tables agree.

---

## 9. Packing portfolio

### 9.1 Module layout (core library)

```text
src/pack/                 replaces packing.rs, packing_search.rs, grid.rs, layout.rs (thin
                          compatibility wrappers keep pack_footprint/grid_layout for tests/benches)
  mod.rs        pack(prints, rules, options) -> PackOutcome   (orchestrates, selects, validates)
  context.rs    Ctx: kind masks per rotation, no-fit per rotation pair, base FreeMaps, tower layer
  no_fit.rs     span Minkowski difference (moved, unchanged)
  free_map.rs   FreeMap (bits) + FreeCounts (u16 counters, removable) + tower layer
  sweep.rs      today's BLF families / mixed palette (moved; fix the locked phase aliasing, CM §6.4)
  lattice/
    enumerate.rs  contact vectors on ∂N, det scan, Lagrange reduction, HNF dedup
    residue.rs    HNF residue histogram (best phases), tower-anchor rescoring
    single.rs     single-lattice member
    double.rs     0/180 double lattice (A on Λ, B on Λ + w, w on ∂N_AB)
    two_block.rs  one guillotine cut, two lattices
  edge_fill.rs  BLF fill of a seeded layout from 4 corners, all rotation classes
  improve.rs    GLS insertion + ruin-and-recreate on FreeCounts, iteration budget
  tower_scan.rs anchor candidates inside the GUI range, scoring per lattice/phase
  select.rs     ranking key and candidate shortlist
  robustness.rs robustness δ* (extra support dilation that still validates)
```

### 9.2 Members

| member | what | when | measured |
|---|---|---|---|
| A single lattice | all admissible lattices with det ≤ 1.25 × min (about 230-240), exact best phase per lattice by HNF residue histogram, top K = 24 → top R = 6 phases → edge fill | always | U1 51-52, P1S 43, A2L 69-71 (CM §10.2, NR §3.3) |
| B double lattice | A at θ, B at θ + 180 (both predicted, so allowed), det ≤ 1.10 × min, coarse at 0.5 mm then refined ±2 px at 0.25 | `--rotations 0,180` | U1 52-55, P1S 43-47, A2L 76-79 (NR §3.3, RP §6) |
| C two-block | x or y cut on a 2 mm grid; each side gets a lattice from A's/B's shortlist; the seam is checked through N | always (cheap) | not measured (estimate 0-2) |
| D sweeps | today's greedy families | always | A2L locked 70 vs lattice 69 on the proxy (NR §3.3) |
| E improve | GLS insertion of n+1 on the top 3-6 layouts, then ruin-and-recreate (remove the 2-4 lowest-slack items near an edge or the tower, insert k+1) | after A-D, iteration budget from `--effort` | +1 to +3 when there is slack (NR §3.3) |

### 9.3 Tower anchor inside the search

- Anchor candidates: the source anchor; positions flush with each side of the target flavour's GUI
  anchor range (§6 `gui_anchor_range`: B 15 mm + GUI estimate; S 1 + `prime_tower_brim_width` against
  the real depth; R 1 + estimated brim, so its CLI never snaps), every 2 mm; the four corners of the
  range. Candidates that put the tower body on a hard zone, the tower body's L0 on a soft zone, or the
  tower outside the printable area are dropped. Tower size and chamfer come from `TowerPrint`
  (04, deterministic per flavour; it does not depend on object count).
- For a lattice Λ with phase histogram H, the tower at anchor T removes the lattice points inside
  its forbidden-origin region R_T (tower body, chamfer rings included, against every kind in the bands
  its pieces meet; the L0 tower brim against landings).
  A coset r loses at most m_T points, where m_T bounds the points of one coset inside R_T
  (about area(R_T)/det + a boundary term, 5-10 here). So only phases with H[r] ≥ max H − m_T can
  win, each needs m_T point lookups, and a (lattice, anchor) score is exact and costs
  microseconds. This is where "tower anchors folded into the HNF residue scoring" happens.
- The top (lattice, phase, anchor) triples go to edge fill and improvement with the tower fixed.
  Sweeps and two-block members are scored at the best few anchors.
- `wipe_tower_x/y` of the 3MF is set to the chosen anchor (`relocate_wipe_tower_in_3mf`, as today).

### 9.4 Selection

Ranking key, compared lexicographically:
1. count, descending;
2. robustness δ*: the largest extra dilation of the support kinds in {0, 0.25, 0.5, 0.75, 1.0} mm
   that still validates (lattices: admissibility against N_δ; others: neighbour pairs with dilated
   masks). Computed only for the top 8 by count;
3. canonical clearance ≥ the flavour threshold (§9.7);
4. compactness: layout bbox area, or the centre-out radius on time-window plates;
5. a canonical tie key (member id, HNF tuple, residue, anchor, corner).

### 9.5 Determinism

- No RNG outside fixed-seed schedules. GLS "samples" are strided grids offset by the iteration
  number.
- Budgets are counted in iterations, never wall-clock time. `--effort` scales the iteration
  counts of a fixed schedule, so more effort runs a longer prefix and still selects among all
  earlier candidates.
- Rayon only maps independent units (lattice batches, anchors, members, pairs). Results are
  collected into a `Vec` in unit order, then reduced with the total-order key.
- All geometry keys are integer pixels. Floats enter only through vector validation, which decides
  pass/fail and never ranks.

### 9.6 Validation chain

`raster admissibility by construction` → `validate_raster` (shortlist) → `validate_vector` (winner)
→ `PackPlan::build` re-validates before assembly (cheap now: cached zones) → optional
`verify_outputs` → optional slicer `verify` step.

### 9.7 Object order in the 3MF (shared-support rule)

- For each rotation class, the copy with the largest *canonical clearance* goes first among the
  copies of its class. Canonical clearance = distance from the copy's envelope to the printable
  boundary ∪ hard zones ∪ the tower body. Other copies keep the packing order, or the centre-out
  order on window plates.
- Threshold per flavour, derived from 02 §2.2 and §2.8:
  - **B**: the bed outside, the exclusion zones and the P1S cutter are one routing obstacle. A branch
    of radius r keeps its *centre* at c(r) = q(r + 0.54) + 0.21 from it in regular mode (rounded
    radius keys q, 02 §2.2: 6.06 mm for a full 3.73 mm foot, 2.81 mm for a 1 mm branch), q(r) + 0.21
    in min-xy mode. The obstacle cannot change a realization whose drawn material stays at least
    t_B = max over the radii of the band of (c(r) − r) away from it. For the goat radii (0.4-3.75 mm)
    that is 2.70 mm (radii just above 3.36 mm, key 5.85), 2.33 mm for a full foot, 1.85 mm for a 1 mm
    branch, ≤ 1.51 mm in min-xy mode. Measured from the un-expanded, unclipped canonical envelope per
    band (the L0 flange expansion is not affected by the obstacle, 03 §6.2), plus that band's δ. The
    old 5.5 mm rule was safe but over-strict. The threshold applies to the cutter as well as the edges.
    The two 2 mm corridors through the bed centroid (02 §2.8) only weaken the obstacle there, so the
    same threshold stays safe.
  - **O/R/S**: 0 (clip only, and the hard rule already keeps envelopes inside).
  Below the threshold: `canonical_near_edge: true` in the report, and verification is recommended.
- `assembly` asserts the shared-mesh default mode (no `duplicate_mesh_per_object`). Without it,
  every copy computes its own trees, and edge copies can bend inward (B), which the envelope does
  not cover.
- Implemented in `src/bin/colored_3mf_packer/order.rs`, applied after centre-out ordering, so a
  window prefix always contains each class's canonical copy.

---

## 10. Pipeline (`colored_3mf_packer`)

### 10.1 Default path: zero slicer calls

```text
job      profile (+bed_rules), settings, extruder offset, flavour, mesh (all normal parts + paint),
         source item transform                                              (no stage)
check    settings against §5.7; Unsupported → --support-source slicer (or fail)  (no stage)
model    ObjectPrint per rotation class: mesh_section + first_layer + support_model (cache)
         TowerPrint from settings                                           stage "modelling"
prepare  PlateRules = profile rules + flavour GUI band + tower; pack portfolio
         with tower anchors; layout event                                   stage "packing"
order    canonical-first per class (+ centre-out on window plates)
assemble 3MF (shared meshes, wipe_tower_x/y at the anchor), build re-validates   "assembling"
finish   3MF + state v2 + report (final_sliced: false, prediction block)   stage "writing"
verify   only with --verify (or webapp «Проверить нарезкой»)              final_slice, gcode_check, repacking
```

Module changes: new `model.rs`, `order.rs` and `verify.rs`. `finish.rs` keeps only the writing.
`probe.rs` and `support_learning.rs` are reachable only through `--support-source slicer` (the
legacy mode, kept for A/B comparison and calibration; removal is a later decision, Q10).
`tower.rs`'s contest and free spot are removed in favour of §9.3. `prepare.rs` loses the
sample. `plan.rs` loses `pack_at`'s double pack.

### 10.2 Optional verification step (`verify.rs`)

1. Slice the assembled plate (`slicer_cli::slice_command`, `--arrange 0`, `--no-check` as today).
2. Parser fixes, useful in every mode:
   - Bambu single-object G-code *does* carry `; OBJECT_ID: N` (M §1.3: found in the corpus'
     single-object Bambu slices; PM §0.1 called it unlabelled, and the repo's
     `layer_zones_from_sample` fails on it today). Accept `; OBJECT_ID:` as a label. As a fallback, when a plate holds exactly one object and no label
     is found, attribute every non-tower, non-custom extrusion to it. End a label at tower and
     custom sections.
   - `prime_tower.rs` reads G2/G3 (I/J, sampled), so Bambu rib-wall towers measure 24.9×24.0, not
     16.3×14.6 (PM §0.4).
3. GUI-equivalent checks (`gcode_collisions/gui_checks.rs`, behaviour spec):
   - centreline bbox of non-custom extrusions, arc points included, inside the printable bbox,
     ε = 3e-4 (error);
   - exclusion test per extrusion segment against the exact polygons (the strictest of B and S;
     error);
   - ConflictChecker rule: centreline crossings more than 0.01 mm from segment ends, or collinear
     overlap > 0.01 mm, between different objects or an object and the tower walls/brim;
     support×support exempt. Support × L0 tower brim = reported, allowed (owner); support × chamfer
     ring (layers ≥ 1) and everything else = error;
   - the S 3.5 mm part-bbox band, from the layout (no G-code needed);
   - today's material checks stay: coverage, off-plate with half width, support↔foreign-body
     distance per layer.
4. The sliced realization is compared with the envelope per band. The report records
   `envelope_exceedance_mm`, and the value is appended to the calibration log
   (`--calibration-log`).
5. On failure, the realized geometry is unioned into the envelope (`learn`) and the plate is
   repacked **once**. Anything left over is fatal (as today). Nudge stays for now.
6. The verifier's flavour may differ from the target flavour (U1: server O vs user S). The report
   names both. Containment is checked against the verifier-flavour envelope, which validates the
   model; the layout's safety for the target relies on the target-flavour envelope. The server's O
   cannot see S-only conflicts: its tower is 9.5 mm deep against S's 16.1 mm (04 §1), and S's
   supports lie up to 4-5 mm outside the CLI's (M §2.4).

### 10.3 Time window

A time window needs plate time estimates, and that remains slicer-based. It moves off full
plates:

- T(N) ≈ a + b·N per (model, settings, flavour). a = start/prep plus tool changes and tower (the
  per-layer filament changes do not depend on N); b = per-object print plus travel. a and b come
  from two small slices (1 object and k = 3 objects at the packed pitch, cut from the assembled
  plate with `scheduler::subset_3mf_items`). Each slice is about 5 s and < 0.5 GB, runs
  sequentially and is cached. Profile `filament_change_seconds` corrections stay
  (`print_time.rs`).
- `fit_window` uses the line instead of secants through full plates. The final window plate is
  verified only with `--verify`. This changes CLAUDE.md's "a time window ignores
  `--no-final-slice`" and needs the owner's approval (Q5). Fallback: the current secant over full
  plates behind `--window-exact`.
- Stage ids: `probe_slice` for the time samples; `window_slice` only with `--window-exact`.

### 10.4 STL jobs

`source_project` (Orca `--export-3mf`, not a slice) stays as it is. Its project has no
`Application` of the user's GUI, so the flavour comes from the profile's `gui_flavour` (Q8). The
STL path then runs the same default pipeline.

### 10.5 State, report, webapp contract

- `PackState` v2: `ObjectPrint` per class, `PlateRules`, `TowerPrint`, anchor and canonical
  indices. v1 states load through the `Footprint` adapter.
- Report, new fields (snake_case): `support_source` (`model`|`slicer`), `slicer_flavour`,
  `flavour_approximated`, `support_model {members, saturated, margins_mm {first_layer, bands},
  cache}`, `robustness_mm`, `canonical_objects [{rotation_deg, index, clearance_mm}]`,
  `canonical_near_edge`, `tower {anchor, size, brim_mm, source}`, `bed_rules [{id, class}]`,
  `verification {slicer, gui_checks, envelope_exceedance_mm} | null`. Kept: `final_sliced`,
  `supports_detected`, `support_rule` (deprecated), `support_collisions`, `time`.
- Progress: new `Stage::Modelling` → id `modelling` in `src/progress.rs`. Mirror it in
  `webapp/.../runner/PackStage.kt`: `MODELLING` takes band 1-12 and `PROBE_SLICE` (time samples,
  legacy probes) keeps 12-15. Add
  a label in the frontend (for example, «Расчёт опор»). `ProgressTracker.kt` needs no logic
  change, only the enum entry, because unknown ids resolve to null today. The `layout` event is
  unchanged. The `slicer_progress` heartbeat appears only during verify and time slices.
- Webapp flow: «Сразу проверить нарезкой» / «Проверить нарезкой» map to `--verify` /
  `--finish-state … --verify`. The child-job mechanism is unchanged.

### 10.6 CLAUDE.md invariants that change (update in WP I4)

These invariants change: the sample slice, `SUPPORT_SLICE_RUNS` (legacy mode only), the tower
contest and free spot, `pack_at` packing under both bodies, the 4 mm exclusion blanket (after
the audit), `support_edge_margin_mm` (renamed through `bed_rules`), and "a time window ignores
`--no-final-slice`" (if Q5 is approved), and the GUI tower clamp of `CLAUDE.md:36` (1 mm for every
Orca-family slicer is wrong: S uses 1 + `prime_tower_brim_width` and re-clamps after slicing, R's CLI
snaps, §6). `CLAUDE.md:79` ("vanilla Orca on the server matches the user's Snapmaker GUI") holds for the
L1 patch bbox only (tower 9.5 vs 16.1 mm deep, supports up to 4-5 mm apart, 04 §1, M §2.4). Three
invariants are added: canonical-first order, the clean-room rule and the band rule of §4.1.

---

## 11. Tests and harness

**CI tests** (self-contained; generated data or `tests/fixtures/`; no slicer, no network):

| file | covers |
|---|---|
| `tests/mesh_section.rs` | all-parts slab projection on generated meshes; per-part hulls/boxes; the source rotation is kept |
| `tests/support_model.rs` | generated meshes (table with a cantilever, bridge, L-shape, painted enforcer facets written by the test): feet trimmed by the own uncompensated L_0 ⊕□ gap; expansion steps; flavour switches (flat foot vs hemisphere L1 area, bucket rule, critical-only differences incl. sharp tails, 10 nm vs 1 nm grid constants of 01 §4); the exact T-class walk (classes cover T = 1..64 with no gap, every class re-run gives the same hash) and the two S9 families; the order test vectors of 02 §8 (AABB order, merge rounds, compaction, nearest boundary point); **determinism** (identical envelopes for `RAYON_NUM_THREADS` = 1 and 8, run twice); every §5.7 setting returns `Unsupported` with its name |
| `tests/support_fidelity.rs` | a small decimated goat fixture with paint (`tests/fixtures/support/goat_small.3mf`, copied deliberately, ≤ 300 KB) plus recorded single-object realizations per flavour (`*.realization.json`), including at least one realization of each corpus L1 family of B. Every recording must lie inside the envelope with 0 exceedance beyond δ. |
| `tests/keepouts.rs` | the matrix as a table test; legacy-profile → `bed_rules` mapping; hull rule (touching allowed); soft strip blocks only the landing; restricted band uses part boxes with equality passing; tower brim (L0) allows support base; chamfer rings are tower body and forbid `support[k]` in their bands; the band rule (a layer (1.88, 2.16] is in bands 0 and 1; a layer ending exactly at 2.0 only in band 0; layer 0 in band 0 and in the first-layer kinds), shared with `verify_outputs`' copy of the table |
| `tests/first_layer.rs` | landing (05 §4): slice plane h0/2, multi-part union, ε_L per wall generator |
| `tests/pack_portfolio.rs` | lattice enumeration and HNF residue counts against brute force on small no-fit sets; tower-anchor rescoring against a full recount; effort-prefix monotonicity; byte-identical results across thread counts |
| `tests/clearance.rs` | property test with fixed-seed random polygons: raster-admissible ⇒ vector distance ≥ G; over-reserve ≤ √2·res |
| `tests/validation.rs` | the vector check catches an injected violation that the raster check missed; zone caching gives the same result as uncached |
| `tests/gui_checks.rs` | synthetic G-code: unlabelled Bambu single object, `; OBJECT_ID:`, tower arcs, segment inside an exclusion, centreline ε, crossing vs collinear conflicts |
| `tests/colored_profile.rs` | each catalog profile loads with `bed_rules` (existing uniqueness check extended) |
| webapp | `ProgressTracker` maps `modelling` |

**Offline harness** (never in CI; real projects by path; no full plates on the server):

- `examples/support_corpus.rs`: sequential single-object slices of a project at each rotation
  class. On Linux it also pins core counts (`taskset -c 0..T-1`) for S2; macOS uses its natural
  count. It stores realizations as JSON. This is the main fidelity loop: per band, how far
  each recorded realization lies outside the model envelope.
- `examples/goat_bench.rs`: the three owner projects. It reports counts, stage times and peak
  RSS, compares them with the §2 targets, and runs twice (1 and N threads) to diff the outputs byte
  for byte.
- `examples/plate_audit.rs`: given a packed 3MF and a G-code sliced off-peak by the owner
  (sequentially, one at a time), it runs the GUI-equivalent checks, support↔foreign-body gaps,
  and envelope exceedance per object and band. Output goes to `docs/calibration/<flavour>.csv`.
  This campaign sets δ and gates removal of the 4 mm exclusion blanket.

**Calibration data still missing** (review §5.2; all single-object and sequential unless noted, never
full plates by agents):

| # | data | why |
|---|---|---|
| C1 | **Owner Snapmaker Orca 2.4.0 GUI exports** of a single goat at 0° and 180°, ≥ 5 each; one batch right after a fresh GUI start and one after slicing another project first | the U1 target flavour S has no single-object data; S9 session effect (Q20) |
| C2 | `goat-runner-orca:2.4.2` (O): 10 runs at 0° and 180° at a fixed cpuset, then cpusets {1, 2, 4, 14} | the server verifier's flavour is unvalidated; O is the only CLI proxy for S's S1/S2/S9 |
| C3 | ≥ 40 more B runs at 0° and 180° for P1S and A2L (≈ 4 s each) | the B unions are not saturated (16 realizations in 53 observations) |
| C4 | B and O on Linux with `taskset` T ∈ {1, 2, 4, 8, 14}, 5 runs each at 0° | T-classes beyond the owner's 14 |
| C5 | discriminating run: O 2.4.2 ×10 at a fixed cpuset; B at cpuset 1 ×5 | if O varies, S1 or S9 is live (R has S1, so mainly S9); if not, SU is B-only and B's code outside 01-03 must be re-read |
| C6 | owner GUI single-goat P1S/A2L exports at 0°, 5 each | "owner GUI ≡ CLI distribution" (02 Q1) is open; one GUI plate shows a family (L1 616.7) the CLI never produced |
| C7 | an A2L full plate and one mixed 0/180 plate per printer, sliced off-peak by the owner, through `plate_audit` | S8 for A2L; the double-lattice member and canonical-first ordering with two classes |
| C8 | B P1S single goat at a bed-edge midpoint, a corner and against the cutter, 3 runs each | S4 clip and the B corridor obstacle (02 §2.8, 03 Q2), the §9.7 threshold |
| C9 | P1S 5° rotation sweep, 1 run each (72 slices) | the 225° anomaly (01 Q9); rotation classes beyond {0°, 180°} |
| C10 | recount realizations and exceedances on outer boundaries (`support_patch_polygon` per layer and band) instead of line hashes; extend `derive.py` to body polygons per band | δ on footprints, not toolpaths; landing/body validation (05 L1) |
| C11 | two-cube disjoint object (R and B); a painted synthetic with an enforcer patch | 01 Q5 fast path; paint projection |

**Acceptance gate for switching the default to `--support-source model`:**
- counts ≥ the targets at the calibrated δ;
- per flavour: calibration data for the target flavour exists (U1: C1), and every corpus realization
  of that flavour lies inside the model envelope ⊕ δ;
- 0 support/body contacts on ≥ 5 audited full plates per printer;
- byte-identical determinism;
- packing < 5 s.

---

## 12. Work packages

Three tracks: **S** (support and mesh geometry), **P** (packing core), **I** (integration). Each WP
owns its files. Only WP0 touches `src/lib.rs` and `Cargo.toml`.

| WP | owner | files (exclusive) | depends on | delivers |
|---|---|---|---|---|
| **0** types and stubs | P | `src/lib.rs`, `Cargo.toml` (`libm`, `i_overlay`, §5.2), `src/object_print.rs`, `src/keepout/{mod,matrix,rules}.rs` (types and the matrix table), `src/footprint.rs` (legacy adapter), `src/options.rs` (`PlateRules` field + legacy conversion; P3 takes it over afterwards), `tests/footprint.rs`, empty module stubs for every new module below | — | `ObjectPrint`, `PrintKind`, `Realization`, `Keepout`, `PlateRules`, `pair_rules()`, legacy `Footprint` adapter; compiles; table test |
| S1 mesh and paint | S | `src/embedded_mesh.rs`, `src/mesh_section.rs`, `src/paint.rs`, `tests/mesh_section.rs` | 0 | all normal parts, source transform, slab bands, hulls/boxes, paint decode |
| S2 support stage A | S | `src/support_model/{mod,settings,flavour,layers,sections,overhangs,clearance}.rs`, `src/polygon_ops.rs` | S1 | A1-A5 from 01 and 02 §2; flavour switches; flavour grid (B 10 nm / int32, O/R/S 1 nm); `polygon_ops` with the Clipper-1 emulation of §5.2; the `Unsupported` checks of §5.7 |
| S3 support stage B | S | `src/support_model/{tips,descend,merge,place,smooth,draw,base,bands}.rs` | S2 | B1-B8 from 01 §5, 02 §3-§5 and 03; realizations per member (T-class, S9 family, optional tip order) |
| S4 ensemble and envelope | S | `src/support_model/{ensemble,envelope,cache}.rs`, `tests/support_model.rs`, `tests/support_fidelity.rs`, `tests/fixtures/support/` | S3 | exact T-class × S9 enumeration, optional S1 axis with saturation, envelope, cache, initial δ; investigation of the unidentified B source SU (C5-C7); report on whether polygon-backend differences dominate δ |
| P1 clearance and masks | P | `src/span_raster.rs`, `src/raster_mask.rs` → `src/kind_masks.rs`, `src/collision.rs`, `tests/clearance.rs`, `tests/raster.rs`, `tests/support_landing.rs`, `tests/support_overlap.rs` | 0 | Euclidean masks, matrix pairs, feasible map with zones |
| P2 validation split | P | `src/validation.rs`, `src/validation_geometry.rs`, `src/bin/verify_outputs/*`, `tests/validation.rs` | P1 | raster/vector split, cached zones, parallel pairs |
| P3 portfolio skeleton | P | `src/pack/{mod,context,no_fit,free_map,sweep,select}.rs`, `src/packing*.rs`, `src/grid.rs`, `src/layout.rs`, `src/frame.rs` (wrappers), `src/options.rs` (after WP0), `tests/packing.rs`, `tests/constraints.rs` | P1 | Ctx, FreeCounts, sweeps moved, ranking, wrappers |
| P4 lattice + tower | P | `src/pack/lattice/{enumerate,residue,single}.rs`, `src/pack/{edge_fill,tower_scan}.rs`, `tests/pack_portfolio.rs` | P3 | member A, tower anchors in scoring |
| P5 double and two-block | P | `src/pack/lattice/{double,two_block}.rs` | P4 | members B, C |
| P6 improve and robustness | P | `src/pack/improve.rs`, `src/pack/robustness.rs` | P4 | member E, δ*, effort prefix |
| I1 verification fixes | I | `src/gcode_collisions/{parser,gui_checks}.rs`, `src/colored_packer/prime_tower.rs`, `src/gcode_footprint.rs` (`realization_from_gcode`), `tests/gui_checks.rs`, `tests/prime_tower.rs` | — (start now; WP0 only for the `Realization` type) | Bambu label fix, tower arcs, GUI-equivalent checks, G-code → `Realization` |
| I2 profiles and catalog | I | `src/colored_packer/profile.rs`, `src/colored_packer/bed_rules.rs`, `src/bin/printer_catalog/{extract,qualify,annotate}.rs`, `src/purge_detect.rs`, `profiles/*.json`, `tests/colored_profile.rs`, `tests/keepouts.rs` | 0 | schema, legacy mapping, derivation, migration of U1/P1S/A2L, regenerated catalog |
| I3 first layer, brim, tower | I | `src/first_layer.rs`, `src/tower_model.rs`, `src/plate_centre/gui_tower.rs` (clean-room review) | S1 | landing (05), brim (03 §5), `TowerPrint` with chamfer rings as z-ranged body (04), per-flavour `gui_anchor_range` (§6) |
| I4 pipeline | I | `src/bin/colored_3mf_packer/{main,args,job,model,prepare,plan,order,assembly,finish,verify,report,tower}.rs`, `src/progress.rs`, `webapp/backend/.../PackStage.kt`, frontend stage label, `CLAUDE.md`, `docs/packing-safety.md`, `docs/packing-search.md` | S4, P4, I2, I3 (P2 for timing) | default path, `--verify`, state v2, report, stages |
| I5 time window | I | `src/bin/colored_3mf_packer/{time_window,time_report}.rs`, `src/print_time.rs` | I4 | small-slice line, `--window-exact` |
| I6 harness | I | `examples/{support_corpus,goat_bench,plate_audit}.rs`, `docs/calibration/` | S4, I1 | corpus, bench, audit, δ calibration |

Order and parallelism:

```text
WP0 ─┬─ S1 ── S2 ──────── S3 ───────── S4 ─┐
     ├─ P1 ─┬ P2                            │
     │      └ P3 ── P4 ─┬ P5                ├─ I4 ── I5
     │                  └ P6                │
     ├─ I2 ─────────────────────────────────┤
     ├─ I3 (after S1) ──────────────────────┘
     └─ I1 (independent; start now) ─────────── I6 (after S4, I1) → calibration → gate
```

- **Milestone M1** (no support model yet): P1-P4 + I1 + I2 + the v1-`Footprint` adapter ship on top
  of the *slicer-learned* footprints. Measured with real footprints: U1 51-52, A2L 69-71, P1S 42-43
  (CM §10.2). It fixes the P1S/A2L probe regression and removes the 100 s of validation overhead.
- **Milestone M2:** S1-S4 + I3 + I4: the slicer-free default behind `--support-source model`.
- **Milestone M3:** I6 calibration campaign → δ → acceptance gate → default switch → I5.

---

## 13. Open questions

| # | question | needed for |
|---|---|---|
| Q1 | Calibrated δ_L1 and δ_band per flavour, and whether P1S ≥ 43 and A2L ≥ 70 hold with them (they fail at g + δ ≈ 1.5 mm, RP §5.3). The corpus exceedances (B up to 3.0 / 4.4 mm, S vs R up to 4.1 / 5.4 mm outside 5-run unions, §5.5) make this the binding question. If not: 0/180 double lattices (P1S 47, A2L 79 with real realizations) become the default. | gate |
| Q2 | U1: pack for Snapmaker Orca 2.4.0 (owner's GUI) only, or for the union with the server's vanilla Orca? Same-rotation comparison now exists: S GUI plates lie 0.7-4.1 mm (L1) / 1.5-5.4 mm (bands) outside the R CLI union at 0° (M §2.4). | §5.3 |
| Q3 | Tower growth with object count (P1S 18.5 → 23.2 mm) was measured with the arc-blind parser. Re-measure after I1, then model it or drop the 0.3 reserve. | I3 |
| Q4 | Exact landing rule. **Answered by 05:** section at h0/2 ⊕ 0.05 mm (classic walls; corpus max excess 0.026 mm in 120 slices); Arachne bound ½·(min_bead_width − min_feature_size), unmeasured (05 L2). The old "≤ 1.66 mm" was patch-vs-silhouette growth, not a landing measurement. | I3 |
| Q5 | May a time-window plate ship without a full slice (small-slice line T = a + bN)? Accuracy against past window plates is still to be measured. | I5 |
| Q6 | Polygon backend. **Decided** (§14): `i_overlay` behind `polygon_ops.rs` with Clipper-1 emulation where it changes discrete decisions; facade swappable; WP S4 reports whether backend differences dominate δ. | WP0 |
| Q7 | U1 `PRINT_START` purges of other toolheads, and the A2L `G130` line geometry: are they real keepouts? | I2 |
| Q8 | STL jobs: which GUI flavour the user will slice with (profile default, or ask in the UI). | I4 |
| Q9 | Envelope cache in the webapp: persistent volume vs SeaweedFS/DB; who invalidates it on `MODEL_VERSION` changes. | I4 |
| Q10 | Keep `--support-source slicer` long-term, or delete it after the gate? | after M3 |
| Q11 | Skirt (`skirt_loops > 0`): model it as a plate-level ring or keep it as an explicit error. | I3 |
| Q12 | S3 time trigger (30 s) on loaded user machines: is a "tripped at layer j" member needed for big meshes even when the island-count trigger does not fire? | WP S4 |
| Q13 | Rotations beyond {0°, 180°}: each costs one more ensemble and one more draw on the plate. | P5 |
| Q14 | Per-band body/body as the default? It is physically valid, but not binding for the goat (cell 834 vs 1072 mm², CM §0.5). | P1 |
| Q15 | **Where does B's run-to-run spread come from (SU)?** S1 (unproven: R has the same race and is deterministic), S2 (constant per machine), S3 (ruled out) and S9 (two families) do not explain 7 L1 families in 53 P1S observations. The answer decides whether the ensemble can ever cover B without a large δ. Experiments C5-C7. | WP S4 |
| Q16 | **U1 target data:** can the owner export single-goat S GUI slices (0°/180°, ≥ 5 runs each, C1)? Without them the U1 envelope is unvalidated. | gate |
| Q17 | Fidelity target: reproduce upstream realizations, or a sound envelope with a calibrated δ? **Answered** (§14). | WP S2-S4 |
| Q18 | Tower chamfer vs supports: does "supports may overlap the tower brim" mean L0 only? **Answered** (§14). | §7.2 |
| Q19 | Band boundary rule shared by `validate_result` and `verify_outputs`. **Answered** (§4.1, §14). | P1, P2 |
| Q20 | GUI session effect of S9 (01 Q2): should the owner restart the slicer before slicing a packed plate, which pins S9 to the CLI families? Needs C1's fresh-start batch. | WP S4 |
| Q21 | Bambu adds supports at z 26-41.6 mm at 225° only (5 of 5 runs, P1S and A2L; M §2.2). Which rule fires (01 Q9)? Required before `--rotations` admits classes beyond {0°, 180°}. | P5 |

---

## 14. Decisions on the open questions (lead, 2026-11-02)

- Q1: measure; the portfolio always includes the 0/180 double lattice (it is a member, not a fallback).
- Q2: the target flavour is the owner's GUI (source `Application`: Snapmaker Orca 2.4.0 for U1). Server
  verification with vanilla Orca checks the model, not the target. `--also-flavour` stays optional.
- Q3: the critic re-measured with arcs: the tower does not grow with object count (P1S 24.88 × 24.03 for 1, 3
  and 40 objects; U1 identical for 1 and 49). `tower_model` predicts the printed tower; the 0.3 growth
  reserve is dropped once the model matches the measured towers.
- Q5: out of scope for this round: the time window keeps today's slicer-based flow (opt-in feature).
- Q6: pure Rust, integer coordinates: `i_overlay` (already pulled in by `geo`) behind `polygon_ops.rs`. No
  C++ in the build; arc-approximation differences go into δ.
- Q7: keep today's conservative strips; Q8: profile `gui_flavour`; Q9: `--cache-dir`, webapp wiring later;
  Q10: keep `--support-source slicer` for calibration; Q11: skirt = explicit error; Q13: {0°, 180°} by
  default, `--rotations` for more; Q14: per-band body/body is an option, silhouette stays the default.

Decisions after the spec review (`2026-11-02-organic-support-spec/00-review.md` §7; lead, 2026-11-02):

- Q17 (fidelity target): a behaviourally faithful generator (same stages, formulas, constants and
  grids per flavour: B 10 nm with int32-like truncation, O/R/S 1 nm), plus an ensemble over the known
  axes (the exact T-classes of 02 §4.7, the two S9 connector families, S1 interleavings as an optional
  axis), plus δ calibrated against the corpus. B's spread has an unidentified source, so δ must cover
  the corpus realizations; WP S4 investigates the source (Q15).
- Q6 (refined): `i_overlay` behind `polygon_ops.rs`, emulating Clipper-1 semantics where they change
  discrete decisions (miter-limit floor 2, arc-tolerance rule, decimation, flavour grid). The facade
  stays swappable; WP S4 reports whether backend differences dominate δ.
- Q18 (tower): the chamfer rings on layers ≥ 1 belong to the tower **body** zone, z-ranged; only the
  L0 tower brim is the `tower_brim` zone that supports may overlap. Clamps: Snapmaker GUI margin
  1 + `prime_tower_brim_width`; Bambu 15 mm + GUI estimate; Orca 2.5 CLI snaps (§6).
- Q19 (bands): a printed layer occupies (z_prev, z]; its material goes into every 2 mm band its
  interval meets. Layer 0 material is in band 0 and in its own first-layer kinds (`landing`,
  `support_base`). `validate_result` and `verify_outputs` share the rule (§4.1).
- Unsupported settings: the model returns an explicit `Unsupported` error naming the setting; the
  pipeline falls back to the slicer path or fails (§5.7).
- Identifiers: the specs and the code use our names (`nearest_inside`, `radius_valid`, `step_down`,
  `radius_key`, `try_merge`, `fill_notches`, …), never upstream names (§1, 02 §1.4).
- Still open: Q15 (SU source), Q16 (owner S exports), Q20 (S9 session effect), Q21 (225° supports).

## Appendix A. Where the facts come from

- Shared supports per rotation class and first-copy clipping: B `Print.cpp:1853-1868,1957-2043,2179-2185`;
  O `Print.cpp:2192-2270`; S `Print.cpp:2267-2278,2366-2441`; clone mesh sharing B
  `Format/bbs_3mf.cpp:5271-5312`; support clip in the slicing object's frame B
  `Support/TreeSupport.cpp:626-629,2505-2522`, `Support/TreeSupport3D.cpp:4643-4646`. Experiments D/E/F
  (ZR §5).
- Organic stages: TSR §2.1-§2.8 (O `TreeSupport.cpp:668-1105`, `TreeSupport3D.cpp:962-1003,1246-1340,1588-2042,2093-2391,2550-2578,2689,3030-3290,3601,3673`;
  B equivalents in TSR §2); first-layer expansion O `SupportCommon.cpp:288-390`, B `:343-460`.
- Brim clipping and overlap: B `Brim.cpp:1011,1029-1049,1054-1077,1084-1139`; `Arrange.cpp:115`.
- Acceptance: §3.1 rows; ConflictChecker `GCode/ConflictChecker.cpp:189-201,223-246,289-313`; ready-to-print
  B `slic3r/GUI/PartPlate.hpp:494-503`.
- Tower: estimate B `slic3r/GUI/PartPlate.cpp:2191-2262`; min depth `GCode/WipeTower.cpp:1587-1626`; auto brim
  `:1627-1630`; chamfer `:3706-3735`; S `GCode/WipeTower2.cpp:2303-2345`.
- Packing measurements: CM §10, NR §3, RP §5-§6. Pipeline facts: PM §0-§9.

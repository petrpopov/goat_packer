# Slicer-free production packer: architecture

Date: 2026-11-02. Status: design, not yet agreed. Scope: the production path of
`colored_3mf_packer` and the packing core. It replaces slicer probes, sample slices, the tower
contest and the mandatory final slice with deterministic models of what the slicer will print.
The slicer stays available as an optional verification step.

Inputs: `/tmp/goat_research/{tree-support-research,zones-research,core-map,pipeline-map,nesting-research,real-plates-analysis}.md`
(cited below as TSR, ZR, CM, PM, NR, RP) and the slicer checkouts in `/tmp/goat_src`.
Separate behaviour specs, written later and dated, give the detailed behaviour of the organic
support generator, the first layer and brim, the prime tower and the GUI acceptance checks. This
document fixes the interfaces those specs must fill, and it marks every version difference and
source of nondeterminism that the architecture must handle.

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
   calibrated margin per band is the support envelope. Rejected: our own support meshes in the
   3MF (TSR §5.1; this changes the print), blocker fences (they do not work in B/O/S, TSR §0.6),
   and cached slicer probes as the production source (that is not slicer-free; they are kept
   only as an offline calibration tool).
2. **Every object's geometry is split into typed kinds**: body per 2 mm band, landing (first-layer
   body), object brim, support base, support bands and interfaces. The prime tower has a body
   kind and a brim/chamfer kind. All collision rules are one **permission matrix** over kinds,
   coded once. Printer profiles declare only geometry and class (hard / restricted / soft).
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
therefore the main risk to the targets** (section 5.5, open question Q1).

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
| `support_critical_regions_only` | keeps cantilevers, sharp tails and overhang ∩ (enforcer grown) | clears plain overhangs, keeps cantilevers (+ enforcers) | as O | as O | B `Support/TreeSupport.cpp:1216-1250`; O `:1086-1089` |
| "fully floating contour ⇒ cantilever" | yes | no | no | no | B `TreeSupport.cpp:1009-1050` |
| merge bucket size | from thread count (S2) | from thread count | **fixed 4** | from thread count | `TreeSupport3D.cpp` B:2238-2244, O:2385-2391, R:2432-2434, S:2389-2395 |
| support-blocker volumes as branch obstacles | no (off-by-one) | no | **yes** | no | `TreeModelVolumes.cpp` B:490, O:472, R:525, S:472 |
| bed outside (and wrapping area) as a routing obstacle | **yes**, in the slicing object's frame | no (clip only) | no | no | B `TreeModelVolumes.cpp:67-88,134`; O `:39-44,67-69` |
| root cap at the bed | hemisphere | **flat foot** | flat foot | hemisphere | O `TreeSupport3D.cpp:2943-2957` |
| smoothing constrained to linked layers | no | yes | yes | no | O `TreeSupport3D.cpp:3156-3202` |
| keep-main connectivity pruning per tree | yes | no | no | no | B `Support/TreeSupport3D.cpp:4933-4960,5075-5164` |
| first-layer support expansion | −1 = auto 2 mm, grown in steps | max(0, e − fine) | as O | as O | `Support/SupportCommon.cpp` B:343,404,448; O:288; R:456; S:359 |
| per-nozzle support exclusion volumes, `bed_exclude_volumes` | no | no | yes | no | R `TreeSupport3D.cpp:3488-3522`; R/main `PrintConfig.cpp` |
| auto-brim width formula (volume groups) | same | same | same | same | `Brim.cpp` B:760-806, O:260-306, R:301-347, S:761-806 |
| organic "support brim" | none (the wide L1 patch is the expansion) | none | none | none | O `Brim.cpp:594,612` (commented) |
| `bed_exclude_area` encoding | groups of 4 points = rectangles | one polygon | one polygon | one polygon | B `slic3r/GUI/PartPlate.cpp:397-413`; S `PrintConfig.cpp:9201-9225` |
| body vs exclusion | per-part 2D convex hull ∩ zone = error (touching OK) | same rule | same | same | B `Print.cpp:944-993`; S `Print.cpp:1390-1440` |
| GUI: toolpath outside plate | centreline bbox (+ arc points) vs printable bbox, ε = 3e-4 | same | same | same | B `BuildVolume.cpp:507-520`, `slic3r/GUI/GCodeRenderer/LegacyRenderer.cpp:1286-1326` |
| GUI: toolpath vs exclusion | every segment, exact | — | — | convex hull of all toolpaths vs exclusion bboxes | B `GCode/BedExcludeChecker.cpp:119-146`, `slic3r/GUI/GCodeRenderer/BaseRenderer.cpp:313-364`; S `slic3r/GUI/GCodeViewer.cpp:2524-2545` |
| GUI: body within 3.5 mm of the edge (spiral lift) | no | no | no | **serious warning** (part bbox, strict <) | S `slic3r/GUI/3DScene.cpp:1381-1401` |
| CLI: toolpath outside | per object bbox + 2 mm, `-104` | + 2 mm, `-102` | as O | — | `GCode/GCodeProcessor.cpp:1845` |
| ConflictChecker collinear-overlap rule | yes | yes | yes | absent | ZR §0.7 |
| tower brim | L0 + **chamfer always on**, ≤ 3 mm, one loop narrower per layer | L0 only (measured) | L0 only | `prime_tower_brim_chamfer` ≤ max width | B `GCode/WipeTower.cpp:3706-3735`; S `GCode/WipeTower2.cpp:2303-2345` |
| GUI tower anchor margin | 15 mm | 1 mm | 1 mm | 1 mm | B `slic3r/GUI/PartPlate.cpp:2284-2293`, `libslic3r.h:77` |

### 3.2 Nondeterminism: sources, realizations, coverage

| id | source | flavours | realizations it can produce | coverage in this design |
|---|---|---|---|---|
| S1 | **Tip insertion race.** Parallel per-layer tasks append tips (roof layers insert up to 3 layers lower) into a shared per-layer list under a mutex. First come wins the ~0.04 mm dedup cell. List order drives AABB sorting and merge partner choice. | B, O, R, S | any interleaving of the writer tasks' sequences for a layer (each task keeps its own order); 2-4 writers per layer list; which of two coincident tips survives | ensemble axis `interleave`: member 0 = writers in ascending layer order; member 1 = descending; then seeded interleavings from our own PCG with fixed seeds (section 5.4) |
| S2 | **Merge bucket size** = 4 if the bucket count would reach the thread count, else 2 | B, O, S (R fixed) | one realization per distinct per-layer 2/4 pattern; depends on the slicing machine's hardware threads (1..64; Docker `--cpus` does not change it) | ensemble axis `threads` ∈ {1, 2, 4, 6, 8, 10, 12, 14, 16, 20, 24, 32, 64}, deduplicated by output hash; R: {4 only} |
| S3 | Overhang-detection guard (`Support/TreeSupport.cpp` O:848, B:961, R:990, S:821): once elapsed time exceeds 30 s **or** a layer has more than 100 overhang islands, sharp-tail detection and small-overhang removal stop for later layers. The flag is written from parallel layer tasks, so the switch layer is racy. | all | "guard tripped from layer j" for any j | The island-count trigger is deterministic and is modelled exactly; when it fires, the switch layer becomes an ensemble axis. The time trigger is not modelled (goat detection takes ≪ 30 s). The report sets `overhang_guard_risk` above a calibrated mesh size. |
| S4 | bed obstacle and clip in the first copy's frame | B obstacle + clip; O/R/S clip | bent or clipped trees copied to every copy | removed by canonical-first ordering (section 9.7) and the hard bed rule for supports |
| S5 | rotation (fill-line angle, square joins, AABB splits) | all | one family per rotation matrix | one ensemble per rotation class; rotated envelopes are never reused (RP §4: 180° realizations rotated back match only at IoU 0.82-0.86) |
| S6 | flavour / version | — | different generator | flavour resolved per job (section 5.3); U1 GUI (S) vs server CLI (O) measured 5.4 mm L1 / 8 mm bands apart, mixed with rotation (TSR §3.2) |
| S7 | float / libm across CPU architectures | all | rare flips of discrete decisions | our code uses `libm` and fixed operation order. Upstream's own flips are covered by δ. |
| S8 | full-plate contention (other objects slice at the same time) | B (observed) | the 40-goat P1S plate lay 0.81 mm (L1) / 1.18 mm (bands) outside a 20-run single-object union | this is S1/S2 under other timings. The explicit interleaving ensemble is expected to cover it; calibration measures the remainder into δ. |

Not sources (TSR §3.1): Jacobi-style smoothing, memoized volume caches, membership-only hash sets.

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
  one-sided bands keep their index (current invariant). "First layer" means the printed
  layer 0 (z ≤ `initial_layer_print_height`). It is its own kind, not band 0.

### 4.2 Kinds

| kind | definition (material extent, line width included) | producer | used by |
|---|---|---|---|
| `body[k]` | projection of every normal part's material inside slab k (a union of slab-clipped triangles, not sampled sections) | `mesh_section` | body/body (option), support/body pairs, tower brim (low bands) |
| `silhouette` | ∪ `body[k]` | `mesh_section` | body/body (default), restricted/hard bbox tests |
| `part_hulls` | 2D convex hull per normal part | `mesh_section` | hard keepouts (the slicer's exclusion rule) |
| `part_boxes` | XY bbox per normal part | `mesh_section` | printable bounds, restricted band |
| `landing` | first-layer body: the section at the layer-0 slice height, plus thin-tip overshoot (perimeter/Arachne tips, measured ≤ 1.66 mm, PM §7 row 5). Elephant-foot compensation is ignored: it shrinks the printed outline by 0.15 mm, so ignoring it is conservative. | `first_layer` (behaviour spec) | support base vs foreign landing, soft zones |
| `brim` | object brim ring: `brim_type`, `brim_width`, auto rule; empty for the goats (auto rule gives 0; probes have no Brim feature, PM §3.2) | `first_layer::brim` | keepout reporting only (the slicer trims brims, ZR §0.6) |
| `support_base` | first-layer support: feet grown by the expansion in steps, trimmed by own `landing ⊕ first_layer_gap` (TSR §2.8) | `support_model` | support base vs foreign landing; hard zones; tower body |
| `support[k]` | support material printed above layer 0 in band k | `support_model` | support vs foreign `body[k]`; hard zones; tower body |
| `interface[k]` | the part of `support[k]` printed as interface (role kept for diagnostics and the strict option) | `support_model` | the same as `support[k]`; `--interfaces-as-body` moves it into `body[k]` |
| `tower_body` | plate-level: tower column, all heights to the tower top | `tower_model` | hard for bodies and supports |
| `tower_brim[l]` | plate-level: brim at L0 plus chamfer rings per layer up to the chamfer top | `tower_model` | soft for support base and brims; hard for bodies in those bands |

The skirt is not modelled. `skirt_loops > 0` is an explicit error, since the slicer does not clip
it (ZR §3.4; open question Q11).

### 4.3 Data type

```text
ObjectPrint            // one rotation class, footprint frame, mm, even-odd loops
  rotation_deg
  silhouette, part_hulls, part_boxes, landing, brim
  body:      Vec<Loops>   // index = absolute band
  support_base: Loops
  support:   Vec<Loops>   // index = absolute band
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
| B1 | `tips` | tip seeding in overhang and roof areas (line pattern at branch spacing, resampled at the connect length), insertion layer offsets, first-come dedup **in the member's order** (S1) |
| B2 | `descend` | influence areas layer by layer downward (slow, then fast, then the fallback), radius growth, foot growth near the bed |
| B3 | `merge` | per-layer merging with the member's bucket rule (S2), destination by list order |
| B4 | `place` | node positions bottom-up (lazy: nearest valid point to the child) |
| B5 | `smooth` | organic smoothing: collision push-out plus Laplacian; anchored variant for O/R; B: keep-main pruning |
| B6 | `draw` | branch discs per layer; root cap (hemisphere B/S, flat O/R); interface roles; bed clip in the object frame |
| B7 | `base` | first layer: feet grown by the expansion in n steps, re-trimmed each step by the slicer's own layer-0 object area from A2 grown by the first-layer gap. Not the conservative `landing` kind: a larger trim mask would under-predict the base. |
| B8 | `bands` | per-layer material into `base`, `support[k]` and `interface[k]` (line width included) |

Rules: single-threaded inside a member; integer coordinates (1 nm grid, like upstream's
scaling) for all polygon booleans and offsets, behind one facade `polygon_ops.rs` (backend
choice: Q6); `libm` for tan, sin and sqrt; `Vec`/`BTreeMap` only, no hash-map iteration.

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

```text
members(flavour):
  threads = flavour.fixed_bucket ? [4] : [1,2,4,6,8,10,12,14,16,20,24,32,64]
  for t in threads:                       # S2 axis first: cheap, coarse variety
      yield Member { threads: t, interleave: Ascending }
  for t in threads:  yield Member { t, Descending }
  for seed in 1..=MAX_SEEDS:              # S1 axis: fixed seeds, our PCG
      for t in threads_subset(seed):  yield Member { t, Seeded(seed) }

run(schedule):                            # deterministic for any core count
  envelope = empty; quiet = 0
  for batch in members.chunks(BATCH):     # BATCH fixed (e.g. 8), run in parallel
      reals = batch.par_map(stage_b)      # collected in member order
      for r in reals (in order):
          if hash(r) seen: quiet += 1; continue
          grown = envelope.add(r)         # per-band area added, mm²
          quiet = grown < SATURATION_MM2 in every band ? quiet + 1 : 0
      if quiet >= QUIET_MEMBERS or members_run >= MAX_MEMBERS: stop
```

The stop rule depends only on results, never on time, so the envelope is identical on every
machine. Constants (BATCH 8, SATURATION_MM2 1, QUIET_MEMBERS 8, MAX_MEMBERS 96) are starting
values with comments; the calibration campaign tunes them.

### 5.5 Envelope and margins

- `SupportEnvelope` per rotation class: `base = (∪ base) ⊕ δ_L1`, `support[k] = (∪ support[k]) ⊕ δ_band`,
  and the same for `interface[k]`.
- **δ is not the pair gap.** δ covers model uncertainty: numerical differences from upstream,
  S3, S7, and realizations the ensemble missed. The pair gap g (support to foreign body,
  `SUPPORT_BODY_GAP_MM` = 0.5, renamed from `LAYER_ZONE_GAP_MM`) is the physical "never touch"
  clearance the owner asked for. Both apply.
- δ is per flavour, kept as constants in `support_model/flavour.rs`, each with an evidence comment
  (like `SLICER_KEEPOUT_SAFETY_MM`). It is not in profiles, because it describes the slicer, not
  the printer.
- Initial values until calibration: δ_L1 = 1.0 mm, δ_band = 1.5 mm (TSR §4.4: single-object
  union vs plate 0.81 / 1.18 mm). **Goal after calibration: δ_L1 ≤ 0.3, δ_band ≤ 0.5.**
  With g = 1.0 the counts drop to U1 52 / P1S 42 / A2L 69 (RP §5.3), so the initial δ alone
  probably misses P1S and A2L. The acceptance gate (section 11) measures the counts at the
  calibrated δ.
- Calibration rule: δ = maximum observed exceedance of any recorded realization outside the
  ensemble union, per kind, over the campaign, + 0.1 mm. The report publishes the sample size.

### 5.6 Cache

Key = SHA-256 over: mesh bytes of all normal parts, paint strings, every support-relevant
setting (a fixed key list in `support_model/settings.rs`), flavour, rotation matrix, and
`MODEL_VERSION` (bumped on any behaviour change). Value = the `SupportEnvelope` plus the realization
hashes, as JSON in `--cache-dir` (default: none, so CLI runs are pure). The webapp passes a
persistent directory (Q9). A cache hit skips stages A and B.

---

## 6. Landing, brim and tower models

- `first_layer::landing(sections, settings, flavour)`: the layer-0 slice plus the thin-feature
  overshoot (elephant-foot compensation ignored, see §4.2). The behaviour spec defines the overshoot
  rule (Q4). Until it exists, the landing is the section grown by the flavour constant
  `LANDING_TIP_RESERVE_MM` near convex features narrower than two line widths.
- `first_layer::brim(landing, settings, flavour)`: `brim_type` (`auto_brim` uses the
  volume-group width rule, identical in all four versions, §3.1; it is 0 for the goats),
  `brim_width`, `brim_object_gap`, brim ears. Output is used for reports and the soft class only.
- `tower_model::printed_tower(settings, plate, flavour) -> TowerPrint { body_size, height,
  brim_l0_mm, chamfer: Vec<(z, width)>, gui_anchor_range }`. Inputs: prime tower width, rib
  wall, rib width and extra length, infill gap, brim width (−1 = auto by height), filament prime
  volumes or flush matrix, the per-layer filament changes (derived from mesh paint and layer
  ranges), and the object height of all parts (41.45 → 44.74 mm on the goat, PM §0.5). It
  replaces the probe-measured keepout.
  - The behaviour spec must explain the observed growth with object count (P1S 18.5 → 23.2 mm,
    `tower.rs:25`). That growth was measured with the arc-blind parser (PM §0.4), so it must be
    re-measured first (Q3).
  - Until then: size = max(settings estimate, planned depth) × (1 + `TOWER_GROWTH_RESERVE`), with
    the current 0.3 kept and its comment.
- `gui_anchor_range` stays `plate_centre::gui_tower_anchor_range` (margin from the flavour:
  15 mm B, 1 mm O/R/S). The tower anchor is a packing variable (section 9.3), so the tower
  contest and `clear_tower_of_patches` go away.

---

## 7. Keepout classes and the permission matrix

### 7.1 Plate zone classes (profile data)

| class | meaning | examples |
|---|---|---|
| `hard` | nothing may print there; copies' supports are not clipped by the slicer | outside the printable area (implicit), `bed_exclude_area` (P1S cutter), `bed_exclude_volumes` (R, z-limited), enabled wrapping area, unknown macro-purge edge band |
| `restricted` | the model body may not enter; supports and brims may | Snapmaker GUI 3.5 mm spiral-lift band; any profile-declared body-only band or rect |
| `soft` | only first-layer body (and the tower body's L0) may not cover it; brims and supports may | purge or prime lines (`erCustom`, no slicer check, ZR §0.5), X1 calibration strip |

Plate-level kinds `tower_body` and `tower_brim` act as two more zones whose position is a packing
variable.

### 7.2 Kinds against plate zones (in code: `keepout/matrix.rs`)

Every zone has a z-range (default: all z). A kind in band k is tested only against zones whose
z-range meets band k. First-layer kinds are tested at layer 0.

| kind | printable boundary (hard) | hard zone | restricted | soft (z ≤ z_max) | tower body | tower brim / chamfer |
|---|---|---|---|---|---|---|
| body / silhouette | each part box inside the printable area (ε 1e-4) | **part hulls** must not overlap (touching allowed) | each part box at distance ≥ width from the edge (equality allowed) | allowed | forbidden, gap c | forbidden in bands below the chamfer top, gap c |
| landing | (covered by the body rows) | (covered) | (covered) | **forbidden**, gap 0 (the overshoot is in the landing) | forbidden | forbidden |
| brim | — (the slicer clips) | — (clipped) | allowed | allowed | — (trimmed) | — (trimmed) |
| support base | material inside the printable area inset by `print_edge_reserve_mm` | **forbidden** | allowed | allowed | forbidden, gap g | **allowed** (owner; GUI warning, CLI conflict without `--no-check`, ZR §0.7) |
| support[k] / interface[k] | same as the base | forbidden | allowed | allowed | forbidden, gap g | allowed |
| tower body | inside the printable area | forbidden (validate error) | allowed | forbidden at L0 | — | — |
| tower brim | inside the printable area (the GUI counts tower lines) | forbidden | allowed | allowed | — | — |

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

- Anchor candidates: the source anchor; positions flush with each side of the GUI anchor range,
  every 2 mm; the four corners of the range. Candidates that put the tower body on a hard zone, the
  tower body's L0 on a soft zone, or the tower outside the printable area are dropped. Tower size
  comes from `TowerPrint` at the bound count (conservative).
- For a lattice Λ with phase histogram H, the tower at anchor T removes the lattice points inside
  its forbidden-origin region R_T (tower body against every kind, tower brim against body kinds).
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
- Threshold per flavour: B = foot radius 3.75 + xy 0.75 + 1 mm (the bed-outside ring is an
  obstacle in routing); O/R/S = 0 (clip only, and the hard rule already keeps envelopes inside).
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
   - Bambu single-object G-code has no object labels (PM §0.1). Accept `; OBJECT_ID: N`. When
     a plate holds exactly one object, attribute every non-tower, non-custom extrusion to it, and
     end a label at tower and custom sections.
   - `prime_tower.rs` reads G2/G3 (I/J, sampled), so Bambu rib-wall towers measure 24.9×24.0, not
     16.3×14.6 (PM §0.4).
3. GUI-equivalent checks (`gcode_collisions/gui_checks.rs`, behaviour spec):
   - centreline bbox of non-custom extrusions, arc points included, inside the printable bbox,
     ε = 3e-4 (error);
   - exclusion test per extrusion segment against the exact polygons (the strictest of B and S;
     error);
   - ConflictChecker rule: centreline crossings more than 0.01 mm from segment ends, or collinear
     overlap > 0.01 mm, between different objects or an object and the tower walls/brim;
     support×support exempt. Support × tower brim = reported, allowed (owner); everything else =
     error;
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
   model; the layout's safety for the target relies on the target-flavour envelope.

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
`--no-final-slice`" (if Q5 is approved). Two invariants are added: canonical-first order and
the clean-room rule.

---

## 11. Tests and harness

**CI tests** (self-contained; generated data or `tests/fixtures/`; no slicer, no network):

| file | covers |
|---|---|
| `tests/mesh_section.rs` | all-parts slab projection on generated meshes; per-part hulls/boxes; the source rotation is kept |
| `tests/support_model.rs` | generated meshes (table with a cantilever, bridge, L-shape, painted enforcer facets written by the test): feet trimmed by own landing ⊕ gap; expansion steps; flavour switches (flat foot vs hemisphere L1 area, bucket rule, critical-only differences); **determinism** (identical envelopes for `RAYON_NUM_THREADS` = 1 and 8, run twice); saturation rule |
| `tests/support_fidelity.rs` | a small decimated goat fixture with paint (`tests/fixtures/support/goat_small.3mf`, copied deliberately, ≤ 300 KB) plus recorded single-object realizations per flavour (`*.realization.json`). Every recording must lie inside the envelope with 0 exceedance beyond δ. |
| `tests/keepouts.rs` | the matrix as a table test; legacy-profile → `bed_rules` mapping; hull rule (touching allowed); soft strip blocks only the landing; restricted band uses part boxes with equality passing; tower brim allows support base |
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

**Acceptance gate for switching the default to `--support-source model`:**
- counts ≥ the targets at the calibrated δ;
- 0 support/body contacts on ≥ 5 audited full plates per printer;
- byte-identical determinism;
- packing < 5 s.

---

## 12. Work packages

Three tracks: **S** (support and mesh geometry), **P** (packing core), **I** (integration). Each WP
owns its files. Only WP0 touches `src/lib.rs` and `Cargo.toml`.

| WP | owner | files (exclusive) | depends on | delivers |
|---|---|---|---|---|
| **0** types and stubs | P | `src/lib.rs`, `Cargo.toml` (`libm`, polygon backend per Q6), `src/object_print.rs`, `src/keepout/{mod,matrix,rules}.rs` (types and the matrix table), `src/footprint.rs` (legacy adapter), `src/options.rs` (`PlateRules` field + legacy conversion; P3 takes it over afterwards), `tests/footprint.rs`, empty module stubs for every new module below | — | `ObjectPrint`, `PrintKind`, `Realization`, `Keepout`, `PlateRules`, `pair_rules()`, legacy `Footprint` adapter; compiles; table test |
| S1 mesh and paint | S | `src/embedded_mesh.rs`, `src/mesh_section.rs`, `src/paint.rs`, `tests/mesh_section.rs` | 0 | all normal parts, source transform, slab bands, hulls/boxes, paint decode |
| S2 support stage A | S | `src/support_model/{mod,settings,flavour,layers,sections,overhangs,clearance}.rs`, `src/polygon_ops.rs` | S1 | behaviour spec (support) for A1-A5; flavour switches |
| S3 support stage B | S | `src/support_model/{tips,descend,merge,place,smooth,draw,base,bands}.rs` | S2 | realizations per member |
| S4 ensemble and envelope | S | `src/support_model/{ensemble,envelope,cache}.rs`, `tests/support_model.rs`, `tests/support_fidelity.rs`, `tests/fixtures/support/` | S3 | envelope, saturation, cache, initial δ |
| P1 clearance and masks | P | `src/span_raster.rs`, `src/raster_mask.rs` → `src/kind_masks.rs`, `src/collision.rs`, `tests/clearance.rs`, `tests/raster.rs`, `tests/support_landing.rs`, `tests/support_overlap.rs` | 0 | Euclidean masks, matrix pairs, feasible map with zones |
| P2 validation split | P | `src/validation.rs`, `src/validation_geometry.rs`, `src/bin/verify_outputs/*`, `tests/validation.rs` | P1 | raster/vector split, cached zones, parallel pairs |
| P3 portfolio skeleton | P | `src/pack/{mod,context,no_fit,free_map,sweep,select}.rs`, `src/packing*.rs`, `src/grid.rs`, `src/layout.rs`, `src/frame.rs` (wrappers), `src/options.rs` (after WP0), `tests/packing.rs`, `tests/constraints.rs` | P1 | Ctx, FreeCounts, sweeps moved, ranking, wrappers |
| P4 lattice + tower | P | `src/pack/lattice/{enumerate,residue,single}.rs`, `src/pack/{edge_fill,tower_scan}.rs`, `tests/pack_portfolio.rs` | P3 | member A, tower anchors in scoring |
| P5 double and two-block | P | `src/pack/lattice/{double,two_block}.rs` | P4 | members B, C |
| P6 improve and robustness | P | `src/pack/improve.rs`, `src/pack/robustness.rs` | P4 | member E, δ*, effort prefix |
| I1 verification fixes | I | `src/gcode_collisions/{parser,gui_checks}.rs`, `src/colored_packer/prime_tower.rs`, `src/gcode_footprint.rs` (`realization_from_gcode`), `tests/gui_checks.rs`, `tests/prime_tower.rs` | — (start now; WP0 only for the `Realization` type) | Bambu label fix, tower arcs, GUI-equivalent checks, G-code → `Realization` |
| I2 profiles and catalog | I | `src/colored_packer/profile.rs`, `src/colored_packer/bed_rules.rs`, `src/bin/printer_catalog/{extract,qualify,annotate}.rs`, `src/purge_detect.rs`, `profiles/*.json`, `tests/colored_profile.rs`, `tests/keepouts.rs` | 0 | schema, legacy mapping, derivation, migration of U1/P1S/A2L, regenerated catalog |
| I3 first layer, brim, tower | I | `src/first_layer.rs`, `src/tower_model.rs`, `src/plate_centre/gui_tower.rs` (clean-room review) | S1 | landing, brim, `TowerPrint` |
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
| Q1 | Calibrated δ_L1 and δ_band per flavour, and whether P1S ≥ 43 and A2L ≥ 70 hold with them (they fail at g + δ ≈ 1.5 mm, RP §5.3). If not: 0/180 double lattices (P1S 47, A2L 79 with real realizations) become the default. | gate |
| Q2 | U1: pack for Snapmaker Orca 2.4.0 (owner's GUI) only, or for the union with the server's vanilla Orca? The measured difference is 5.4 / 8 mm, but mixed with rotation. Needs a same-rotation comparison. | §5.3 |
| Q3 | Tower growth with object count (P1S 18.5 → 23.2 mm) was measured with the arc-blind parser. Re-measure after I1, then model it or drop the 0.3 reserve. | I3 |
| Q4 | Exact landing rule: elephant-foot compensation and thin-tip overshoot (≤ 1.66 mm measured). It binds the support-base pair. | I3 |
| Q5 | May a time-window plate ship without a full slice (small-slice line T = a + bN)? Accuracy against past window plates is still to be measured. | I5 |
| Q6 | Polygon backend: Clipper2 via FFI (closest to upstream offsets, adds C++ to the build) or pure Rust `i_overlay`/`geo` (no FFI, more deviation, so a larger δ). Integer coordinates either way. | WP0 |
| Q7 | U1 `PRINT_START` purges of other toolheads, and the A2L `G130` line geometry: are they real keepouts? | I2 |
| Q8 | STL jobs: which GUI flavour the user will slice with (profile default, or ask in the UI). | I4 |
| Q9 | Envelope cache in the webapp: persistent volume vs SeaweedFS/DB; who invalidates it on `MODEL_VERSION` changes. | I4 |
| Q10 | Keep `--support-source slicer` long-term, or delete it after the gate? | after M3 |
| Q11 | Skirt (`skirt_loops > 0`): model it as a plate-level ring or keep it as an explicit error. | I3 |
| Q12 | S3 time trigger (30 s) on loaded user machines: is a "tripped at layer j" member needed for big meshes even when the island-count trigger does not fire? | S4 |
| Q13 | Rotations beyond {0°, 180°}: each costs one more ensemble and one more draw on the plate. | P5 |
| Q14 | Per-band body/body as the default? It is physically valid, but not binding for the goat (cell 834 vs 1072 mm², CM §0.5). | P1 |

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

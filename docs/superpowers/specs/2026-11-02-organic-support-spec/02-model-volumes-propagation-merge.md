# Organic tree supports, part 2: model volumes, influence-area propagation, merging, node placement

2026-11-02. Functional spec for a slicer-free re-implementation of the middle of the Organic tree-support
generator. It starts after the tips have been placed (part 01) and ends with one centre point per branch
node per layer, before organic smoothing and drawing (part 03).

**Licence rule.** BambuStudio, OrcaSlicer and Snapmaker Orca are AGPL-3.0. This document describes behaviour
in our own words: formulas, constants, data flow, edge cases, order properties and recorded test vectors. It
contains no source code and no transliteration. The `file:line` references are there so that an implementer
can check a behaviour against upstream. They are not material to copy. Where exact reproduction depends on an
order (the AABB reordering of §4.2, the merge rounds of §4.4, the compaction of §5.4, the nearest boundary
point of §5.5), the spec states the resulting order as invariants and pins it with test vectors (§8), not
with a step-by-step procedure. Operations and fields carry our names (glossary §1.4); upstream names appear
only in citations.

**Versions** (sources in `/tmp/goat_src`):

| tag | slicer | used by |
|---|---|---|
| **B** | BambuStudio v02.08.02.61 (`bambu_0208`) | owner's P1S / A2L GUI and CLI |
| **O** | OrcaSlicer v2.4.2 (`orca_242`) | server U1 |
| **S** | Snapmaker Orca v2.4.0 (`snaporca240`) | owner's U1 GUI |
| **R** | OrcaSlicer v2.5.0-rc (`orca_250rc`) | local `/Applications/OrcaSlicer.app` |

Unless noted, paths are relative to `src/libslic3r/Support/`. `TMV` = `TreeModelVolumes.cpp`, `TS3` =
`TreeSupport3D.cpp`, `TSC` = `TreeSupportCommon.hpp`, `TS3H` = `TreeSupport3D.hpp`. Line numbers are given
as `O n / S n / R n / B n` when they differ. The tags B/O/R/S are the same in every part of this spec set.
The flavour tag R always stands alone (a table column or before a citation). R(d, f), with arguments, is
the radius function of §1.2.

---

## 0. Findings that matter for the packer

1. **This stage is deterministic given its inputs.** Once the per-layer tip lists exist (part 01, in their
   order), the volumes, propagation, merging and node placement are a pure function of:
   - the tip lists, *in their order*;
   - the object outlines;
   - the bed polygon in the object's frame (Bambu only);
   - one integer, **T = TBB `max_concurrency()`**, which only selects the merge bucket size (§4.3).

   Every other parallel loop in this stage writes disjoint data or fills a cache of pure functions (§4.7).
   The race S1 (tip insertion order) enters only through the order of the tip lists.
2. **T matters only through a per-layer threshold.** A layer with n merge candidates uses buckets of 4 when
   ⌊(n+2)/4⌋ ≥ T, and buckets of 2 otherwise.
   - The realizations over all T form a small, enumerable set of intervals of T (§4.7). There are at most
     max⌊(n+2)/4⌋ + 1 of them, about 11 for a goat.
   - **R's "fix" pins the rule to the T = 1 class.**
   - The owner's GUIs run on a 14-thread M3 Max (`hw.logicalcpu = 14`). For every layer with n ≤ 53 they
     are in the **bucket-of-2 class**, the same class as the server's many-core Orca 2.4.2 run. Local Orca
     2.5-rc is in a *different* class.
3. **Merging happens on every layer.** The "merge every x layers" throttle was written for micrometre units.
   In the slicers' units (1 nm, or 10 nm in B) its maximum is 0, so the condition is always true (§3.2). The merge-only computations inside a move
   therefore always run.
4. **`min_feature_size` is read in the wrong unit.** The percentage value (25) is used as millimetres, so it
   becomes 25 mm (`TSC:59` and peers). The "safe movement step" is therefore about 25.2 mm, which has two
   consequences:
   - the per-layer move is checked against the wall restriction only in the slow part of the move (two
     half-steps);
   - the fast extension is a plain round offset (§3.4).

   All three goat projects have `min_feature_size = 25%`.
5. **Collision radii are rounded up to coarse cache keys** (0.4, 1.5, 2.6, 3.9, 5.85 mm, …). With the
   regular xy distance, a 1.0 mm branch therefore keeps its centre 2.81 mm from the model instead of 1.75 mm.
   A 3.73 mm foot keeps 6.06 mm instead of 4.48 mm (table in §2.2). This rounding, not the nominal xy
   distance, sets where trunks and feet land. Measured on existing P1S G-code, full feet sit 5.97–6.04 mm
   from the body (§5.1).
6. **The stored influence area (IA) is limited by collision, not by avoidance.** Avoidance only decides
   whether a move is *valid* and which part takes part in a merge (§3.6–3.7). Earlier notes wrote
   `IA_l = (IA_{l+1} ⊕ v) \ Avoid`. That is wrong for the stored IA, and the difference matters for where
   nodes are placed.
7. **Only Bambu treats the bed's outside (and its exclusion zones) as an obstacle** (§2.8). It is a 1 m ring
   around the bed polygon, cut into four pieces with 2 mm slits on the bed's centroid axes.
   O, S and R have no bed obstacle. All four clip the support to the bed later (part 03).
8. **For this stage, O ≡ S** on the goat settings. Their differences (roof bookkeeping, two guards that
   never trigger here) do not change node positions. B differs in:
   - where its settings come from;
   - the bed obstacle;
   - its polygon-simplification flavour;
   - **its coordinate grid**: 10 nm units with 32-bit coordinates, against 1 nm with 64-bit in Orca. This
     changes every integer truncation, e.g. V_f = 0.27999 mm against 0.279999 mm, and the tip seed polygon
     has 15 vertices against 45.

   R differs in:
   - the bucket rule;
   - collision simplification, plus support-blocker volumes now counting as obstacles.

---

## 1. Conventions, constants, interfaces

### 1.1 Geometry conventions

- **Coordinates and units differ by flavour.** "trunc" means truncation toward zero, which is how every
  double→integer conversion in this stage behaves.

  | flavour | unit | coordinates | Clipper | where defined |
  |---|---|---|---|---|
  | **B** | **1 unit = 10 nm** (1 mm = 10⁵, scaling factor 10⁻⁵) | 32-bit | built with 32-bit coordinates (`src/clipper/clipper.hpp:83`) | `libslic3r.h:40,58` |
  | **O/S/R** | **1 unit = 1 nm** (1 mm = 10⁶; Orca switches to 10⁻⁵ only for beds ≥ 2147 mm) | 64-bit | 64-bit coordinates, orientation cross products in *double* (`deps_src/clipper/clipper.hpp:85-97`) | `libslic3r.h:43,60-70` |

  Every constant written in "units" below (rounding guards, the 5-unit extra speed, the integer truncations
  of speeds and radii, the 10-unit boolean safety offset) is therefore 10× coarser in B than in Orca.
  Physical thresholds that upstream writes in mm are the same in both: ε_A, the 100 nm box epsilon, every
  offset and arc tolerance. mm → units conversions truncate the double quotient, so some round values lose
  a unit in B: 2.0 mm → 199 999, 0.03 mm → 2 999, 7.5 mm → 749 999 (01 §0, §4).
- **Offsets** use the semantics of the Clipper 1 library (Angus Johnson, v6.4.x, Boost licence; an
  independent port is fine licence-wise), with one Slic3r addition. Our `polygon_ops` facade reproduces the
  rules of this list on top of `i_overlay` (design §5.2):
  - **Input decimation.** Before offsetting by distance d, a ring drops every vertex closer than
    0.005·|d| to the previously kept vertex, and trailing vertices closer than that to the first vertex
    (O `ClipperUtils.cpp:277`, `deps_src/clipper/clipper.cpp:3380`).
  - **⊕m(d)** is a miter-join offset. Upstream passes miter limit 1.2, but Clipper raises any limit below 2
    to 2 (`clipper.cpp:3542`). So corners are mitred out to 2·d and squared off beyond that.
  - **⊕M3(d)** is a miter offset with limit 3 (the library default).
  - **⊕r(d; τ)** is a round-join offset with arc tolerance τ. The effective tolerance is
    τ' = min(τ, |d|/4). A full turn uses π / acos(1 − τ'/|d|) segments, capped at π·|d|
    (`clipper.cpp:3547-3557`). ⊖ means a negative offset.
  - **Offsetting a set of rings** offsets each ring on its own (holes in the opposite sense) and then unions
    the result with the non-zero rule (shrinking adds the usual bounding frame with the negative rule).
  - **ex(X)** resolves a ring set X to polygons-with-holes (non-zero rule). **ex(X) ⊕ d** then offsets each
    outer contour by +d and each hole by −d, and subtracts the holes.
- **Booleans** (∪, ∩, ∖) use the non-zero fill rule. A difference whose clip set is first cropped to the
  subject's bounding box (an upstream speed-up) gives the same result.
- **S(τ) = simplify.** For each ring: Douglas–Peucker with tolerance τ on the closed ring (first vertex
  repeated, then dropped), then a self-union of that ring with the non-zero rule, with holes reoriented back.
  The mode of that union differs by flavour:

  | flavour | union mode |
  |---|---|
  | B | "strictly simple" (`Polygon.cpp:658`) |
  | O, S | not strictly simple (`TSC:591` constant `false`) |
  | R | plain union (`Polygon.cpp:685`), equivalent to O |

  The difference is vertex-level only: touching vertices are split or not, and the area is the same.
- **ε_A** = 10⁻⁶ mm² = (0.001 mm)². "Area > ε_A" is the test for "non-empty".
- **U\*(A, B)** is the "safe union": A ∪ B. If that comes out empty although an input was non-empty, the
  result is instead the union of every input ring's boundary thickened by ⊕m(0.002 mm) (open butt ends)
  (`TS3:749`, B `SupportCommon.cpp:2129`).

### 1.2 Derived settings (evaluated for the goat projects)

Formulas follow `TreeSupportSettings` (`TSC:267` / S 268 / R 285 / B 258). Setting names are the
project keys.

| symbol | meaning, formula | B (P1S, A2L) | O/S/R (U1) |
|---|---|---|---|
| lh | `layer_height` (the first layer is not used here) | 0.28 mm | 0.28 mm |
| w | support line width | 0.42 mm | 0.42 mm |
| θ_f | max branch angle. B: `tree_support_branch_angle`; Orca: `tree_support_branch_angle_organic`. Clamped below 90° | 45° | 40° |
| θ_s | preferred ("slow") angle. B: hard-coded 25° (`TSC:80`); Orca: `tree_support_angle_slow`. Clamped below θ_f | 25° | 25° |
| V_f | trunc(tan θ_f · lh), in units | **27 999 units of 10 nm = 0.27999 mm** (tan 45° evaluates to 0.9999999999999999) | **234 947 nm** |
| V_s | trunc(tan θ_s · lh) | 13 056 units = 0.13056 mm | 130 566 nm |
| V_f′, V_s′ | the volumes' copies, V − 2 units ("rounding guard", `TMV:63`) | 27 997 / 13 054 units | 234 945 / 130 564 nm |
| r_tip | tip radius. B: hard-coded 0.8/2; Orca: `tree_support_tip_diameter`/2, clamped to ≤ branch diameter | 40 000 units = 0.4 mm | 400 000 nm |
| r_b | branch radius = trunc(diameter) div 2. B: `tree_support_branch_diameter`; Orca: `_organic` | **99 999 units = 0.99999 mm** (2.0 mm truncates to 199 999) | 1 000 000 nm |
| N_tip | tip layers = max(⌊(r_b − r_tip)/⌊w/3⌋⌋, ⌊r_b/lh⌋), integer division in units | 4 | 4 |
| g | radius growth per layer = tan(`tree_support_branch_diameter_angle`)·lh | 0.0244968 mm | same |
| R_inc | "increase radius until" radius = r_b | 0.99999 mm | 1.0 mm |
| N_inc | its layer = N_tip · ⌊R_inc/r_b⌋ | 4 | 4 |
| r_bp | trunk foot target, trunc(7.5 mm) div 2 | 374 999 units | 3 750 000 nm |
| g_bp | foot growth per layer = min(tan(0.7 rad)·lh, w/2) | 20 999.5 units | 209 999.5 nm |
| L_bp | layers of foot zone = trunc((r_bp − r_b)/g_bp) (an integer, not 13.1) | **13** | 13 |
| g_ft | foot increment per unit of "foot count" = g_bp − g (double) | 0.1855032 mm | same |
| g_ft′ | the same, truncated to units (used only in §3.6) | 18 550 units = 0.18550 mm | 185 503 nm |
| xy | `support_object_xy_distance` (B: at least 0.01; B lowers it to max(top gap, 0.2) if `top_z_overrides_xy_distance`, which is off here) | 0.75 mm | 0.75 mm |
| xy_min | min(xy, ½·external-perimeter width) | 0.21 mm | 0.21 mm |
| δ | xy − xy_min | 0.54 mm | 0.54 mm |
| z_top, z_bot | round(gap / lh). The gaps are first snapped to multiples of lh (`Slicing.cpp`: O 164-187, B 115-123) | 1, 1 | 1, 1 |
| res | `resolution` | 0.03 mm | 0.03 mm |
| mfs | `min_feature_size` *percentage value read as millimetres* (bug, §0.4) | **25 mm** | **25 mm** |
| to_model_allowed | ¬`support_on_build_plate_only` | **false** | **false** |
| min_depth_model | ⌈1.0 mm / lh⌉ (to-model only) | 4 | 4 |
| Δr_model,max | 1.0 mm / 2 (to-model only) | 0.5 mm | 0.5 mm |

**Radius functions** (`TSC:545-563`, identical in all four):

- **R(d, f)**, the radius of a node with radius height d and foot count f:
  - base(d) = r_tip + ⌊(r_b − r_tip)·d / N_tip⌋ for d ≤ N_tip (integer arithmetic);
  - base(d) = r_b + (d − N_tip)·g for d > N_tip;
  - R(d, f) = trunc(base(d) + f·g_ft), with the sum taken in double.

  Values (Orca): R(0..4) = 0.40, 0.55, 0.70, 0.85, 1.00 mm, R(5) = 1.0245 mm, R(89) = 3.082 mm. B (units):
  R(0..4) = 40 000, 54 999, 69 999, 84 999, 99 999.
- **r_rec(l)**, the recommended minimum radius at layer l: r_b + (L_bp − l)·g_bp for l < L_bp, else 0;
  truncated. r_rec(0) = **3.73 mm** (B 372 992 units, Orca 3 729 993 nm), r_rec(12) = 1.21 mm.
- **effective radius height** d_eff(e) = (e.rdepth < N_inc) ? min(e.depth, N_inc) : e.rdepth.
- **real radius** ρ(e) = R(d_eff(e), e.foot). **Collision radius** c(e) = R(e.rdepth, e.foot) ≤ ρ(e).

### 1.3 Interfaces

**Inputs** (from part 01 and the slicer core):

- **O_l**: object outline per support layer l = 0…N−1, where support layer index = object layer index
  (no raft in our projects). Each O_l is the layer's merged slice, simplified with S(res).
  N = (highest overhang layer) − z_top (`TS3:332` precalculate).
- **TipLists[l]**: an *ordered* list of tip states for l < N (part 01). Each tip carries:
  - position p;
  - initial influence area: a regular polygon of radius 10 µm centred on p, with vertex i at angle 2πi/k
    and coordinates truncated. Upstream asks for "25", but the circle function takes 25 as a chord error in
    *units*, so k = ⌈2π / (2·acos(1 − 25/r_units))⌉. That gives **k = 45 in Orca** (r = 10 000 nm) and
    **k = 15 in B** (r = 1 000 units) (`TS3:866`, B 884-885, `Polygon.cpp:743`);
  - origin_layer = l; origin_xy = hint = centre = p;
  - rdepth = depth = 0, foot = 0;
  - flags to_bed, graceful, past_holes (from the tip's line status), tight_xy
    (= xy > xy_min, true here), under_roof, keep_round;
  - hold_until (3 for an ordinary tip with 3 interface layers);
  - roof-recovery counters (only part 03 uses them).
- **B_obj** (Bambu only): the bed polygon in the frame of this object's first instance (§2.8).

**Output** (to part 03): for each layer l, a list of nodes. Each node has:
- its centre;
- its state (rdepth, depth, foot, flags);
- the indices of its parents in layer l+1;
- its IA.

Deleted nodes have been removed and the indices remapped (§5.4).

### 1.4 Our names (glossary)

Design §1 forbids upstream identifiers in operative text. This part uses the names below. The right-hand
column exists only so that a reviewer can find the behaviour upstream.

| ours | meaning | upstream (citation only) |
|---|---|---|
| `depth` | layers below the element's highest tip | `distance_to_top` (dtt) |
| `rdepth` | radius height, normally `depth`, may lag | `effective_radius_height` |
| `foot` | foot-growth count (double) | `elephant_foot_increases` |
| `centre` | placed node position | `result_on_layer` |
| `hint` | position hint for placement | `next_position` |
| `origin_layer`, `origin_xy` | the tip the branch ultimately serves | `target_height`, `target_position` |
| `to_bed` | must and can reach the bed | `to_buildplate` |
| `graceful` | lands on the model gracefully | `to_model_gracious` |
| `past_holes` | may use the Slow / FastSafe (hole-closed) avoidances | `can_use_safe_radius` |
| `tight_xy` | still allowed the reduced xy distance | `use_min_xy` |
| `under_roof` | supports a roof | `supports_roof` |
| `keep_round` | no ovalisation | `skip_ovalisation` |
| `hold_until` | `depth` before which the branch does not move | `dont_move_until` |
| `model_growth` | radius increase spent on to-model merges | `increased_to_model_radius` |
| `last_setting` | setting that produced the element | `last_area_increase` |
| `lost`, `very_lost` | debug flags | `lost`, `verylost` |
| `to_model_allowed` | support may rest on the model | `support_rests_on_model` |
| `radius_key`, `next_key` | §2.2 cache key and next key | `ceilRadius`, `getRadiusNextCeil` |
| `r_rec(l)`, `d_eff` | recommended minimum radius, effective radius height | `recommendedMinRadius`; the state overload of `getRadius` |
| `step_down` | move an element one layer down | `propagate_down` |
| `radius_valid` | §3.6 radius test | `validWithRadius` |
| `try_merge` | §4.5 pair merge | `merge_influence_areas_two_elements` |
| `nearest_inside`, `nearest_boundary` | §5.5 | `move_inside_if_outside`, `move_inside` |
| `ex(·)` | resolve rings to polygons with holes | `union_ex` |
| `fill_notches` | 03 §3.7 notch filling | `smooth_outward` |

---

## 2. Model volumes (upstream `TreeModelVolumes`)

All volumes are per-object. The generator never sees neighbouring objects.

### 2.1 Layer outlines

O_l = S(res)(slice of layer l) (`TMV:115` / B 133). B then appends the bed-ring pieces (§2.8) to *every*
O_l, after simplification.

### 2.2 Radius keys (cache quantization), our `radius_key` (upstream `ceilRadius`, `TMV:835` / S 834 / R 894 / B 853)

Every request carries a radius r and a flag *min* (use xy_min instead of xy). It is mapped to a **key** K,
and all caches are indexed by (K, layer).

- **Base rounding q(r):**
  - q(0) = 0;
  - q(r) = r_tip for 0 < r ≤ r_tip;
  - above that, one linear step reaches 1.5 mm (r_tip + one step of (1.5 − r_tip)), then the next linear
    step adds another 1.1 mm (2.6 mm);
  - after that, multiply by 1.5 and truncate, until the value is ≥ r;
  - "ignorable radii" are skipped (see below).

  Key sequence: **0, 0.4, 1.5, 2.6, 3.9, 5.85, 8.775, 13.1625 mm …**
- **Key:** K(r, min) = q(r) when min is set. Otherwise K(r, regular) = q(r + δ) for r > 0, and **exactly δ**
  (0.54 mm, not rounded) for r = 0 (`TMV.hpp:149`).
- **Next ceil:** next_key(r, min) = q(r) when min is set, otherwise q(r + δ) − δ. It is used when the radius
  grows (§3.6).
- **Clearance:** the collision for key K offsets the outline by **K + xy_min** (§2.3). So the centre
  clearance actually enforced for a branch of radius r is:
  - min mode: q(r) + 0.21;
  - regular mode: q(r + 0.54) + 0.21 (0.75 for r = 0).

| r (mm) | min key → clearance | nominal r + xy_min | regular key → clearance | nominal r + xy |
|---|---|---|---|---|
| 0 (tip test) | 0 → 0.21 | 0.21 | 0.54 → 0.75 | 0.75 |
| 0.40 | 0.4 → 0.61 | 0.61 | 1.5 → 1.71 | 1.15 |
| 0.55–0.96 | 1.5 → 1.71 | 0.76–1.17 | 1.5 → 1.71 | 1.30–1.71 |
| 1.00 | 1.5 → 1.71 | 1.21 | **2.6 → 2.81** | 1.75 |
| 1.5 | 1.5 → 1.71 | 1.71 | 2.6 → 2.81 | 2.25 |
| 2.06 | 2.6 → 2.81 | 2.27 | 2.6 → 2.81 | 2.81 |
| 2.5 | 2.6 → 2.81 | 2.71 | 3.9 → 4.11 | 3.25 |
| 3.08 (25 mm trunk) | 3.9 → 4.11 | 3.29 | 3.9 → 4.11 | 3.83 |
| **3.73 (full foot)** | 3.9 → 4.11 | 3.94 | **5.85 → 6.06** | 4.48 |

**Ignorable radii.** Before any other computation (`TMV:167-182`), the generator lists every key reachable
from tip radii, i.e. q(R(d)) and q(R(d) + δ) for d = 0…N_tip. Every key between r_tip and r_b that is not in
that list is marked "ignorable", and q skips it. For the goat settings the reachable keys are
{0.4, 1.5, 2.6}, so **no key is ignorable**. A port must still implement the rule for other settings.

### 2.3 Collision Coll(K, l) (`TMV:416-580` / S 416 / R 468 / B 435)

Collision is the set of centre positions where a branch of key K would hit the model on layer l. The rule
uses the z-gap layers on both sides:

```
Coll(K, l) = ∪_{j=max(0, l−z_bot)}^{l}            ( ex(O_j) ⊕m (K + xy_min) )
           ∪ ∪_{i=1}^{z_top}, l+i<N_obj          ( ex(O_{l+i}) ⊕m (K + ρ_i) )
           [R only: ∪ ( ex(Blocker_l) ⊕m K )]
then: union (non-zero); R additionally applies S(res).
```

- **ρ_i**, the reduced clearance toward the layers above:
  ρ_i = trunc(xy_min − (i − h)·xy_min / z_top), with h = ½ when z_top = 1 and h = 0 otherwise. With
  z_top = 1, **ρ_1 = 0.105 mm**.
  - The ½ keeps support from touching the model on the downward diagonal when the gap is a single layer.
  - Upstream's reason for reducing ρ at all: a full xy above would wipe out thin overhangs that the tips
    should sit under.
- **Layers below.** The layer itself and the z_bot layers below it are obstacles at full xy_min.
- When K + xy_min = 0, the offset is skipped and only the union remains. That never happens here.
- **Blockers.** In O, S and B the generator loads support-blocker volumes but never adds them. A
  "last mesh" test is off by one (`TMV:472`, B 490), so the branch that would add them never runs.
  - The same bug also skips the final S(res), so collisions in O/S/B stay **unsimplified** after the
    union.
  - R fixes the test (`TMV:525`). There, blockers enter, offset by K with no xy, and collisions are
    simplified.
  - No goat project has blockers. The only goat-relevant R difference is the S(res) simplification,
    a vertex-level change of up to 0.03 mm.
- **Edges.** The ring set for layer j < 0 is empty. Layers at or above N_obj (the object's layer count, which
  is larger than N) are skipped.

### 2.4 Hole-free collision CollHF(K, l) (`TMV:582-612`)

CollHF is defined only for K < R_inc + δ = 1.54 mm, i.e. keys 0.4 and 1.5:

```
CollHF(K, l) = S(res)( ex( Coll(K_R, l) ) ⊕r( 5 units − (K_R − K) ; res ) ),
               K_R = K(R_inc, regular) = q(1.54 mm) = 2.6 mm
```

- In words: take the 2.6 mm collision (offset 2.81 mm) and shrink it back by 2.6 − K.
- The result is a **morphological closing** of the model outline at radius ≈ 2.81 mm, expressed at key K.
  Gaps and holes too narrow for a 2.81 mm disc become solid.
- "Safe" avoidances use it, so that branches near the tip cannot route through holes smaller than the
  radius they will soon grow to.

### 2.5 Avoidance Avoid_τ,t(K, l) (`TMV:614-714` / S 614 / R 673 / B 632)

Avoidance is the set of centres from which a branch of key K cannot reach the target t ∈ {plate, model}
when it moves at the speed of type τ ∈ {Slow, FastSafe, Fast}.

```
Avoid(K, 0)  = Coll(K, 0)                          (every τ, every t)
Avoid(K, l)  = S(res)( A_n ),  l ≥ 1, where
    A_0      = Avoid(K, l−1)
    A_{i+1}  = C ∪ ( A_i ⊖r( step_i ; res ) ),  i = 0 … n−1
    (t = model: A_n ← A_n ∖ Place(K, l) before S)
    C        = CollHF(K, l)   if τ ∈ {Slow, FastSafe} and K < R_inc + δ
               Coll(K, l)     otherwise
    m        = V_s′ (Slow) or V_f′ (FastSafe, Fast)
    step     = 1.9 · max(K, xy_min)      (float)
    n        = ⌈ trunc(m) / trunc(step) ⌉  (integer)
    last     = m − (n−1)·step; if last < 0.05 mm and n > 1: n ← n−1, every step = m/n
    step_i   = step for i < n−1, last for i = n−1
```

- **Single step here.** For all goat keys (K ≥ 0.4 → step ≥ 0.76 mm > m), n = 1, so each layer the
  avoidance is shrunk once by the full move and unioned with the layer's collision. The splitting rule exists
  so that a large move cannot tunnel through a wall thinner than the step.
- **Type redirect at lookup** (`TMV:330-357`). A request (r, l, τ, t, min) uses K = K(r, min). If
  τ = FastSafe and K ≥ R_inc + δ, it is served as Fast. Slow keeps its own cache, but for K ≥ 1.54 mm it uses
  the plain collision as C.
- **Which variants exist.** The generator computes to-plate avoidances when at least one to-plate request
  exists, and to-model avoidances only when to_model_allowed. **The goat projects use only the three to-plate
  avoidances.**
- **Where they are used:**
  - Fast and FastSafe: the validity tests of fast moves and the error move;
  - Slow: the slow moves;
  - FastSafe or Fast at radius r_tip = R(0) (key 0.4 mm in min mode, 1.5 mm in regular mode): the tip
    status classification (part 01);
  - layer 0: the plain collision for every type.

### 2.6 Placeable areas (to-model only; inactive for the goats) (`TMV:535-562`, `726-763`)

Place_0(l) is computed for l ≥ z_bot + 1, as part of the radius-0 collision pass:

```
Place_0(l) = ( O_{l−z_bot−1} ⊕M3 xy_min ) ∖ ( (O_l ⊕m xy_min) ∪ Blocker_{l−z_bot−1} )
```

Place(K, l) = ex(Place_0(l)) ⊖m (K + xy_min + δ) for l ≥ 1. Place(K, 0) is empty: the plate is not
"model", and the machine-border term upstream is always empty here. In O, S and B, Place_0 is
unsimplified and has no blockers (same off-by-one as §2.3).

### 2.7 Wall restriction WR(K, l) (`TMV:765-833`)

WR(K, l) is defined for l ≥ 1. It approximates the actual walls between layer l and l−1, so that a large
move cannot jump through a wall in one layer:

```
WR_regular(K, l) = S(res)( Coll(δ, l) ∩ Coll(K, l−1) )
WR_min(K, l)     = S(res)( Coll(0, l) ∩ Coll(K, l−1) )
```

Coll(δ, l) is the radius-0 request in regular mode, i.e. the key exactly δ: the outline (with B's ring)
grown by 0.75 mm, plus the z-gap terms of §2.3. Coll(0, l) is the same grown by 0.21 mm.

- **Keying.** The lookup uses K = q(r), with no δ added, whatever the min flag.
- **Variant choice.** WR_min is used when the requester's min flag is set and δ > 0.

### 2.8 Machine border and bed exclusions (`TMV:54-90` B; `TMV:39-52` O/S/R)

**B_obj, the bed in the object frame** (all versions compute it; `TreeSupport.cpp` O 654 / S 627 /
R 698 / B 626):

1. Start from `printable_area`.
2. Subtract the `bed_exclude_area` rectangles (four points each). B also subtracts `wrapping_exclude_area`
   when `enable_wrapping_detection` is on (it is off in the goat projects).
3. Keep **the first polygon** of the difference.
4. Translate by (plate origin − shift of the object's **first instance**). The result is in the frame of
   the "slicing object" (S4: one realization per rotation class, computed in that object's frame).

**B: bed outside as obstacle.**

1. Build the ring E = (B_obj ⊕M3 1000 mm) with B_obj as its hole.
2. Cut E into four pieces, using c = area centroid of B_obj and E's bounding box [min, max]. The pieces are
   E intersected with four rectangles:
   - [min, c − (1,1) mm];
   - [c + (1,1) mm, max];
   - [(min_x, c_y + 1), (c_x − 1, max_y)];
   - [(c_x + 1, min_y), (max_x, c_y − 1)].
3. This leaves a vertical and a horizontal **2 mm slit** through c.
4. If wrapping detection is on, union the wrapping polygon in. That polygon is shifted by −(instance
   shift) only, with no plate origin.
5. Append the outer contours of the pieces to every O_l. Their holes are dropped.

Consequences:

- Every collision, avoidance, hole-free closing and wall restriction treats "outside the bed / inside an
  exclusion" as model, at clearance K + xy_min (regular mode: K + 0.21 with K = q(r + 0.54)).
- **Open slits.** The slits close once K + xy_min ≥ 1 mm. They stay open, with width 2 − 2(K + 0.21), for
  K ∈ {0, 0.4, 0.54}. That means for the radius-0 tests and for thin tips at the midpoints of the bed edges.
  This is a deterministic quirk, not a real escape route for trunks.
- **P1S.** The 18×28 mm corner exclusion is part of B_obj, so branches of the *slicing object* bend away
  from it. Every copy then prints that bent tree (part 03, S4).

**O / S / R:** the border obstacle is explicitly empty (`TMV:39-52`). B_obj is used only to clip
support layers afterwards (part 03).

### 2.9 Caching, laziness, cost

- **Storage.** Each volume kind (Coll, CollHF, Avoid ×3 types ×2 targets, Place, WR, WR_min) is a map
  layer → (key → polygons). Results are pure functions of (K, l) and the inputs.
- **Precalculation and laziness.** Upstream precalculates a key set:
  - avoidance keys = {q(R(d)+δ), q(R(d)), q(r_rec(l)+δ)} over the layers where they can occur;
  - collision keys additionally {0, δ, q(R_inc + δ)} on all layers.

  It computes anything else on demand. Avoidance is a bottom-up recurrence, so computing it lazily in
  increasing l gives identical results. **A port may compute everything lazily with memoization.** The goat
  keys are 0, 0.4, 0.54, 1.5, 2.6, 3.9, 5.85 mm.
- **Cost.** About N ≈ 93 layers × 7 keys × (z_bot + 1 + z_top) offsets for collisions. Avoidance is
  3 types × ~5 keys × 93 *serial* steps, each one shrink + union + simplify. Wall restrictions are about
  5 keys × 93 × 2 intersections. Each offset costs O(V log V) with V ≈ 10²–10³ outline vertices. The whole
  stage takes well under a second, single-threaded, for one goat.

### 2.10 Differences (volumes)

| item | B | O | S | R |
|---|---|---|---|---|
| bed outside + exclusions as obstacle | **yes** (ring, slits) | no | no | no |
| collision simplified after union | no | no | no | **S(res)** |
| support-blocker volumes as obstacle | no (off-by-one) | no | no | **yes, ⊕m K** |
| S() union mode | strictly simple | not strict | not strict | plain |
| early exit if avoidance step ≈ 0 | present | present | removed | present (never triggers) |
| everything else in §2 | same | same | same | same |

---

## 3. Influence-area propagation (upstream `create_layer_pathing`, `increase_areas_one_layer`, `increase_single_area`)

### 3.1 Element state (`TS3H:79-233`)

Each element (branch node) stores the following.

- **Identity and position:** layer l, depth (layers below its highest tip), origin_layer and
  origin_xy (the tip it ultimately serves), hint (a position hint, §4.5), centre (the placed position;
  "unset" is a sentinel).
- **Radius bookkeeping:** rdepth (radius height, normally equal to depth but allowed to lag), foot (double,
  elephant-foot count), model_growth.
- **Flags:**
  - to_bed;
  - graceful;
  - tight_xy (still allowed the reduced xy distance);
  - under_roof;
  - past_holes ("past any holes": may use Slow and FastSafe avoidances);
  - keep_round;
  - deleted, marked (scratch);
  - lost, very_lost (debug only).
- **Movement:** hold_until (a depth threshold), and last_setting (the setting that produced the
  element; geometrically irrelevant here, §3.2).
- **Geometry:** parent indices into layer l+1, and the IA (a polygon set of allowed centres).
- **Merging copy** (only while merging): the IA, the to-plate part P_bp, the to-model part P_m, and the
  bounding box of all three grown by 100 nm.

Moving an element one layer down (`step_down`): depth + 1, layer − 1, centre reset to unset,
keep_round cleared. O/R also decrement a pending roof-recovery counter (part 03 only).

### 3.2 Per-layer loop

Layers are processed from l = N−1 down to 1. Each non-empty layer l produces elements on layer l−1:

1. Build one candidate per element of layer l, in list order (parent index = its position).
2. Move every candidate down (§3.3–3.7). Candidates are independent of each other.
3. Partition the candidates, keeping their order:
   - IA empty → drop (the branch ends on layer l, §3.8);
   - P_bp and P_m both empty → **bypass**: append to layer l−1 now;
   - otherwise → merge input.
4. If there are at least 2 merge inputs, merge them (§4).
5. Append the merge survivors to layer l−1 in their post-merge order. The stored IA is U\*(IA).

Layer l−1 may already hold tips inserted there by part 01. **The final order of layer l−1** is therefore:
[tips inserted at l−1, in insertion order] + [bypass elements] + [merge survivors]. A tip and a branch on the
same layer meet in the merge of the *next* step down.

**Merge every layer** (`TS3:2461` / B 2313). The throttle is
x_max = min(5000/max(V_f, 100), 1000/max(V_s, 20), 3000/lh), designed for micrometres. In the slicers' units (1 nm; 10 nm in B) it
is **0**, and the "layers since last merge ≥ x" test is always true. Every layer merges, and the merge-only
computations inside a move (to-plate area for non-to-plate elements, the taint corrections) always run. The
shortcut that reuses the parent's last setting is disabled when merging, so it is **never** used.

### 3.3 Speeds and extra terms (`TS3:1816-1860`)

P is the parent on layer l; C = step_down(P), with rdepth_C = rdepth_P and foot_C = foot_P. r_P = c(P).
K(·) is the radius key in P's xy mode (§2.2).

The two speeds are v_fast = V_f + e and v_slow = V_s + e + e_s. They start at e = 5 units (50 nm in B,
5 nm in Orca) and e_s = 0. Two growth terms are then added, the radius term first; each goes either to both
speeds or to the slow speed only:

| term | applies when | adds to both speeds (to e) | otherwise adds to the slow speed only (to e_s) |
|---|---|---|---|
| radius growth Δr = R(rdepth_P + 1, foot_P) − r_P | always | Δr, if K(R(rdepth_P + 1, foot_P)) = K(r_P) or R(rdepth_P + 1, foot_P) < R_inc | min(Δr, V_f + e − V_s − e_s), so the slow speed never passes the fast one |
| foot growth g_bp | l < L_bp and r_rec(l − 1) < R(rdepth_C + 1, foot_C) (strict) | g_bp, if K(R(rdepth_P + 1, foot_P + 1)) = K(r_P) | min(g_bp, V_f − V_s − e_s) (without e) |

The **safe step** is s = (C.min ? xy_min : xy) + (min(z_top, z_bot) > 0 ? mfs : 0); O and R replace s = 0 by
0.1 mm.

Typical values (B, θ = 45°):

| zone | v_slow | v_fast |
|---|---|---|
| tip (Δr = 0.15 mm, r⁺ < 1 mm) | 0.281 mm | 0.430 mm |
| trunk (Δr = 0.0245, key unchanged) | 0.155 mm | 0.305 mm |
| foot zone with bonus | +0.21 mm | +0.21 mm |

For U1 (θ = 40°), v_fast is 0.045 mm lower.

- **Foot bonus condition.** The bonus is granted when the branch is *already wider* than r_rec, which is
  exactly when §3.6 plans no foot growth. Reproduce it literally.
- **s with the mfs bug.** s = 25.21 or 25.75 mm.

### 3.4 Stepped safe offset SOI(A, d, W, s, c, n) (`TS3:792` / B `SupportCommon.cpp:2187`)

SOI grows A by d through the free space without jumping across the wall set W in one step (W is first
cropped to bbox(A) ⊕ d, which does not change the result). **Rule:** grow in k equal steps of length
`step` ≤ s, subtracting W after every step; grow the remainder d − k·step without a wall check; simplify;
and subtract W once more when the "final" flag is set. A degenerate input is first made growable with U\*.
With d = 0 the result is A ∖ W if final, else A.

The step count and length, as maths (x⁺ = max(x, 0), [·] = 1 if true else 0):

```text
q      = ⌊(d − c)⁺ / s⌋
k₁     = q + [d − q·s > c ∧ (q + 1)·s ≤ d]
final  = (c = 0) ∨ (d − q·s > c ∧ (q + 1)·s > d)
if n > 1 ∧ k₁ + [d < c ∨ d mod s ≠ 0] < n:      step = min(⌊d/n⌋, s),   k = (⌊d/n⌋ ≥ s) ? n : ⌊d/step⌋
else:                                            step = s,               k = k₁
rest   = d − k·step                              (grown only if > 100 nm)
```

Each step is a round offset ⊕r(step; 0.01 mm) followed by ∖ W. S(0.015 mm) is applied after the 8th, 18th,
28th … step, and once at the end (before the final ∖ W). The result is returned as a union (non-zero).

**Evaluated for the goats (s ≈ 25 mm ≫ every d).**

- **Slow area** A_slow = SOI(IA_P, v_slow, W, s, 0, 2) gives *two half-steps*. Each half-step is a round
  offset by ⌊v_slow/2⌋ followed by subtracting W, and a final S(0.015) and ∖W follow.
- **Fast area.** Upstream's independent fast offset is never selected here. The fast area is therefore
  A_fast = SOI(A_slow, V_f − (V_s + e_s), W, s, s + r, 1), which is **a single round offset of A_slow
  with no W check**, plus S(0.015).
- **Error area** (the error entry of §3.5):
  - base = IA_P ∪ (IA_P's ring boundaries taken as open polylines, ⊕m 0.005 mm with butt ends). The thin
    ribbon keeps a degenerate, line-like IA growable.
  - overspeed o = trunc(1.5·(V_f + e)).
  - The area is SOI(base, o, W, s, s + r, 1), again a single unchecked round offset.

**W** is the wall restriction of the parent: WR(q(c(P)), l), the min variant if P.min.

### 3.5 Candidate settings and their order (`TS3:1868-1929`)

A setting is (avoidance type τ, speed ∈ {slow, fast, 0}, grow-radius?, ok/error, min?, move?). The order is
built as follows; an entry identical to an existing one is not added again.

- **If ¬C.past_holes** (still possibly above a hole). None of the first three entries move: they
  test whether the unmoved IA is already outside the safe avoidance.
  1. (Slow, slow, grow, ok, regular, **no move**);
  2. only if C.depth < ⌈N_tip/2⌉ = 2: (Fast, slow, grow, ok, regular, no move);
  3. (FastSafe, fast, grow, ok, regular, no move);
  4. (FastSafe, fast, keep, ok, regular, move);
  5. (Fast, fast, keep, ok, regular, move).
- **Else:**
  1. (Slow, slow, grow, ok, regular, move);
  2. (Slow, slow, keep, ok, regular, move);
  3. only if C.depth < N_tip: (FastSafe, slow, grow, ok, regular, move);
  4. (FastSafe, fast, grow, ok, regular, move);
  5. (FastSafe, fast, keep, ok, regular, move).

  There is no plain Fast normal move in this branch. For K ≥ 1.54 mm, FastSafe *is* Fast (§2.5).
- **If C.min:** every entry above is followed by its twin with min = true. The regular variant is always
  tried first.
- **Error entry**, appended if C.to_bed, or if C.graceful and IA_P ∩ Place(r, l) = ∅:
  (Fast, fast, keep, **error**, min = C.min, move).
- **Dont-move entry**, prepended if C.depth < C.hold_until and C.past_holes:
  (Slow, 0, grow, ok, regular, no move).

  With 3 top interface layers, ordinary tips have hold_until = 3. A tip column therefore stays
  vertical for 2 layers below the tip, if the Slow test allows it, and starts moving at depth = 3.

The settings are tried in order and **the first valid one wins** (§3.6). If none is valid, the element fails
(§3.8).

### 3.6 Evaluating one setting (upstream `increase_single_area`, `TS3:1588-1724` / B 1464)

Input: the setting σ, the area A (A_slow if σ.speed = slow, A_fast if fast; for the error entry, the error
area), and the parent P. Output: the new state C′ or "invalid".

1. C′ := step_down(P); if σ.grow: C′.rdepth += 1. r := c(C′).
2. X := (σ.move) ? A : IA_P. X is the "increased area without collision" and is kept for §3.7.
3. **To-plate part:** P_bp := U\*(X ∖ Avoid_σ.τ,plate(r, l−1, σ.min)). If C′ is not yet to_bed and
   area(P_bp) > ε_A, set to_bed (a "taint correction").
4. **To-model part** (only if to_model_allowed): P_m := U\*(X ∖ Avoid_σ.τ,model(…)). If C′ is not yet
   graceful, then:
   - if area(P_m) ≥ ε_A, set graceful and keep P_m;
   - otherwise replace it by P_m := U\*(X ∖ Coll(r, l−1, σ.min)), i.e. route down anyway and land on the
     model non-gracefully.
5. Q := to_bed ? P_bp : P_m.
6. **Radius growth** (only if σ.grow and area(Q) > ε_A).
   - **radius_valid(r′)** is true if K(r′, σ.min) ≤ K(r, σ.min). Otherwise it is
     area(X ∖ Avoid_σ.τ,·(r′, l−1, σ.min)) > ε_A, with the target avoidance (plate, or model/collision as
     in step 4).
   - Let K_before := K(r, σ.min).
   - **Catch-up** (radius lagging behind depth). If c(C′) < R_inc and c(C′) < ρ(C′), with ρ₀ = ρ(C′) and
     target = min(ρ₀, R_inc):
     - the key chain ν₀ = next_key(r, σ.min), ν_{i+1} = next_key(ν_i + 1 unit, σ.min) is followed while it
       stays valid: ν = ν_j for the smallest j with ν_j ≥ target or ¬radius_valid(ν_{j+1});
     - C′.rdepth becomes the largest x with rdepth ≤ x ≤ C′.depth − 1 and R(x, foot) ≤ min(ν, ρ₀) (it keeps
       its value if no larger x qualifies; R(x, foot) is non-decreasing in x, and x never passes N_inc because
       ρ₀ ≤ R_inc there).
   - r := c(C′).
   - **Elephant foot.** Let g_ft′ = trunc(g_bp − g) (an integer here, unlike the double g_ft inside R(d, f)).
     - planned := min(1, (r_rec(l−1) − ρ(C′)) / g_ft′).
     - If planned > 0, C′ is to_bed, ρ(C′) ≥ r_b, ρ(C′) ≥ R_inc and
       radius_valid(R(C′.rdepth, C′.foot + planned)), then C′.foot += planned and r := c(C′).
     - So a trunk gains up to g_bp = 0.21 mm of radius per layer: 0.0245 from depth growth plus 0.1855 from
       foot growth. Below layer 13 it tracks r_rec (3.73 mm at layer 0) whenever the bigger avoidance still
       leaves room. Where it does not, the foot simply stays thinner.
   - If K(r, σ.min) ≠ K_before: recompute P_bp (and P_m) with the new r, and set Q accordingly.
7. Valid iff area(Q) > ε_A.

**Bug kept on purpose.** A simplification at 0.025 mm of X in the no-error case is computed and thrown away
upstream (`TS3:1626`). A port must **not** simplify X there.

### 3.7 Result of a move (`TS3:1983-2040`)

On the first valid σ:

- C := C′, and r := c(C).
- **bypass** := ¬σ.move or (σ.min and C.depth < N_tip).
- If σ.move: hold_until := 0. Otherwise: C.centre := P.centre. A tip's own position is copied down, so
  dont-move columns are exactly vertical.
- past_holes := (σ.τ ≠ Fast).
- If σ is regular: C.min := false. A branch never goes back to min mode.
- **Stored IA:**

  ```
  IA_C = U*( (X ∖ Coll(r, l−1, C.min)) , U*(P_bp, P_m) )
  ```

  The IA is bounded by *collision* at the final radius. Avoidance only gated validity and produced P_bp.
- For merging, keep P_bp (if to_bed) and P_m (if to_model_allowed), unless the element is a bypass.
  Set the bbox to extents(IA_C) ± 100 nm.

### 3.8 When a branch is dropped

- **Failure.** If no setting is valid, the error move included, the candidate gets an empty IA and is dropped
  (§3.2 step 3). In addition, the *parent's* centre is reset and its graceful flag cleared. In node placement
  (§5) the parent then has no centre, so it is deleted together with **every element above it in that
  subtree**: the reset cascades up through all parents, up to their tips.
- **Error moves.** An error move ("teleport") only allows 1.5 × fast speed with the Fast avoidance and no
  wall check. A branch that survives one is flagged "lost" (debug only) and loses past_holes.
- **To-model branches** may also be deleted in placement (§5.3). That never happens in the goat projects:
  every tip there is to-plate, and the AND in §4.5 keeps every element to-plate.

### 3.9 Differences (propagation)

| item | B | O | S | R |
|---|---|---|---|---|
| speeds, settings order, SOI, validity, foot | same | same | same | same |
| s = 0 guard (0.1 mm) | absent | present | absent | present (never triggers here) |
| pending roof recovery decremented on propagate | no | yes | no | yes (part 03 only) |
| setting values (θ, θ_s, tip, branch distance) | `tree_support_*`, hard-coded slow 25°, tip 0.8, top rate 30 % | `*_organic` keys | `*_organic` keys | `*_organic` keys |

**Cost.** For each element and layer: up to 2 SOI calls (a few round offsets and diffs), then one
diff + area per tried setting (≤ 11 settings), plus the catch-up and foot checks (one diff each). With
n ≈ 10–40 elements over ~90 layers, that is a few thousand to ~10⁴ polygon operations.

---

## 4. Merging (`merge_influence_areas*`, `TS3:2044-2444` / B 1914-2297)

### 4.1 Input

The merge input is the ordered list M of non-bypass candidates of one layer step l → l−1, with
n = |M| ≥ 2. Each candidate carries its state, parents, IA, P_bp, P_m and bbox. bbox centroid =
(min + max)/2 per axis, **integer division truncating toward zero** (coordinates can be negative in the
object frame).

### 4.2 AABB reordering (`AABBTreeIndirect.hpp:93-205`, identical in all four)

M is reordered in place by building a balanced median-split tree over the candidates' bounding boxes. Only
the resulting order is used. The order has these properties:

- **Recursive median split.** A range [lo, hi] with more than one element is split at mid = ⌊(lo + hi)/2⌋
  into [lo, mid] and [mid + 1, hi], each ordered the same way in turn (left first).
- **Axis.** The split axis is the longer side of the union of the range's bounding boxes (element boxes,
  not centroids); x on a tie.
- **Partition.** After the split, no centroid coordinate on the left exceeds the element at mid, and none on
  the right is below it, along the axis. Centroids are (min + max) div 2 per axis, truncated toward zero.
- **Which permutation.** Among the orders with these properties, the one produced is that of the textbook
  median-of-three quickselect with Hoare partitioning (Sedgewick's variant): the first, middle and last
  elements of the active range are sorted in place by three compare-and-swaps; the middle one becomes the
  pivot and is parked just before the last; both scans stop on keys equal to the pivot; the pivot is swapped
  back to the meeting point; ranges of at most three elements stop after the three-element sort; the search
  narrows to the side that holds mid.

Quickselect is not stable, so **the result depends on the input order of M**. This is how the tip order (S1)
reaches every later merge. The permutation is pinned by the test vectors of §8.1.

### 4.3 Buckets (S2) (`TS3:2380-2403` / B 2236-2259; R 2428-2445)

- **B, O, S.** T = `tbb::this_task_arena::max_concurrency()`.
  - If m := ⌊(n + 2)/4⌋ ≥ T: **m buckets of 4**.
  - Otherwise: **⌊n/2⌋ buckets of 2**.
- **R:** always m buckets of 4. That is exactly the old rule with T = 1.

Buckets are consecutive ranges of the reordered M, starting at 0:
- If the last bucket runs past n, it is cut at n. It then has size 2 or 3 with size-4 buckets, never fewer
  than 2.
- If exactly one element is left over, it becomes an extra bucket of size 1 that is not merged in round 1.

Thread count only decides the bucket size; the buckets themselves are processed independently.

### 4.4 Merge rounds

Notation: `try_merge(dst, src)` is the pair test of §4.5; on success dst takes the merged state and src
is emptied. Two removal disciplines appear, both standard container operations:

- **remove-by-last** (Rust `Vec::swap_remove`): the removed slot receives the range's current last
  element, and the range shrinks by one at the end;
- **remove-by-first** (Rust `VecDeque::swap_remove_front`): the removed slot receives the live range's
  current first element, and the range shrinks by one at the front.

**Round 1, inside each bucket.** The leftmost live element is the destination. It is offered every later
element in current position order. On a success the absorbed element is removed by remove-by-last, and the
offers start again from the element right after the destination, because the merged area is larger now. When
a full pass finds no partner, the next element becomes the destination. The bucket's live range shrinks
accordingly.

**Later rounds, pairwise over neighbouring buckets.** Bucket 2k (set D) absorbs bucket 2k + 1 (set S); an
odd last bucket is carried over unchanged; the bucket list then halves. For two sets:

- the elements of S are offered in their current left-to-right order; each goes to the **first** element of D
  (left to right) that accepts it;
- an absorbed S element is removed by remove-by-first on S's live range;
- right after an absorption, the absorbing D element is offered every **later** element of D (positions after
  it, in order); each one it absorbs is removed from D by remove-by-last, and the same position is offered
  again;
- afterwards the live range of S follows D directly, in its current order.

Invariants:

- **The destination is always the element on the left** (lower position). It keeps its identity, its
  parents come first, and it keeps its `hint` (§4.5).
- **Survivors are permuted.** Remove-by-last moves the tail of a range forward; remove-by-first moves the
  oldest survivor of S into the absorbed slot. The survivors do *not* keep their original relative order.

The merged layer is the final range of bucket 0. **Its order is the order of the next layer.** Positions
past its end hold emptied elements, which the caller skips because their IA is empty. The test vectors of
§8.2 pin this order together with the bucket rule of §4.3.

### 4.5 Pair merge test and result (`TS3:2093-2269` / B 1960)

try_merge(dst, src) on layer step l → l−1:

1. **Rejects.** Refuse the merge if:
   - dst.graceful ≠ src.graceful; or
   - dst.min ≠ src.min (min-xy and regular branches never merge).
2. **Bigger and smaller.** bigger := dst if c(dst) > c(src), else src (**ties → src is "bigger"**).
   smaller := the other. Δ := |ρ(bigger) − ρ(smaller)|, using real radii.
3. **Box test.** smaller.bbox grown by Δ must intersect bigger.bbox (closed boxes: touching counts).
   Otherwise refuse.
4. **to_bp merge** := dst.to_bed ∧ src.to_bed. If false (to-model only):
   - if one is to-plate and one to-model, the to-model one may grow by the radius difference. Refuse if its
     accumulated increase would exceed Δr_model,max;
   - refuse if neither supports a roof and max(depth) < min_depth_model.
5. Refuse if bigger is not past_holes but smaller is.
6. useMin := bigger.min ∧ smaller.min. Wcoll := Coll(c(smaller), l−1, useMin).
   **grow(X) := SOI(X, Δ, Wcoll, s′ = 2·(xy + c(smaller) − 3 units), c = 0, n = 0)**. Here xy is the regular
   0.75 mm even in min mode. Because s′ ≥ 2.3 mm > Δ, this is one round offset ⊕r(Δ; 0.01) when Δ > 100 nm,
   then S(0.015), then ∖ Wcoll. When Δ = 0 it is just U\*(X) ∖ Wcoll.
7. **I := grow(smaller.P) ∩ bigger.P**, with P = P_bp for a to_bp merge and P_m otherwise.
8. Refuse if area(I) ≤ ε_A, or if area(I ⊖m 0.025 mm) ≤ ε_A. The second test means the overlap must be at
   least about 0.05 mm thick.
9. **Hint.** new_next := nearest_inside(I, **dst**.hint) (§5.5). That is the point of the
   merge overlap closest to the destination's previous hint (the original tip position for an unmerged
   branch).
10. **Merged state:**
    - layer l−1, hint = new_next;
    - min, under_roof, past_holes: OR;
    - hold_until, rdepth, depth: max;
    - to_bed, graceful: AND;
    - origin_layer/origin_xy from the one with the higher origin_layer (**ties → src**);
    - foot := |max(c(dst), c(src)) − R(rdepth, 0)| / (g_bp − g). This is the foot count that makes the merged
      collision radius equal the larger input's;
    - model_growth: the value from step 4 if one was set, else the max of both;
    - centre unset, deleted/marked cleared;
    - last_setting: an elementwise min/OR/AND mix, irrelevant here;
    - roof bookkeeping: O/R keep the max of the pending roof-recovery counts and depths; B/S keep the
      min of the missing roof count (part 03).
11. **Merged areas:**
    - IA := U\*(grow(smaller.IA) ∩ bigger.IA, I);
    - for a to_bp merge: P_bp := I, and if to_model_allowed, P_m := (both graceful ? U\*(grow(smaller.P_m)
      ∩ bigger.P_m, I) : IA);
    - for a to-model merge: P_m := I.
12. dst.parents := dst.parents followed by src.parents. dst takes the merged state and areas. The bbox is
    recomputed over IA ∪ P_bp ∪ P_m (± 100 nm). src is emptied.

**Containment.** I ⊆ bigger.P ⊆ bigger.IA, so the merged IA ⊆ the bigger input's IA. Merging never enlarges
reach beyond what the bigger branch could already do. This keeps the analytic envelope of the earlier research sound.

### 4.6 Cost

- AABB reordering: O(n log n) expected.
- Leaves: O(b²) pair tests per bucket.
- Set merges: O(|D|·|S|) per pair in the worst case.

A pair test costs one round offset, one diff, one intersection and one shrink, but the box test rejects most
pairs. With n ≤ ~50 this is cheap.

### 4.7 Nondeterminism and the realization set of this stage

**Sources inside this stage:**

| id | source | effect | realizations |
|---|---|---|---|
| S2 | bucket size from T (B, O, S) | different pairings, so different merges, positions and later n | one per **T-class** (below) |
| S1 (input) | order of TipLists[l] (part 01) | AABB order and dst/src roles in every layer | whatever part 01 enumerates |
| S4 (B) | B_obj in the slicing object's frame | bed obstacles move with the object | one per object placement |
| S7 | libm/FMA differences in Clipper's trigonometry and in tan() | ±1 unit vertex moves; rarely flips an ε_A test | not enumerable; absorb with a margin |

**Not sources** (checked):

- the per-element moves run in parallel, but each writes only its own candidate and its own parent;
- the merge buckets run in parallel on disjoint ranges;
- the volume caches are pure functions: concurrent lazy computation of the same key yields the same polygons,
  and the first insert wins;
- the precalculation keys pass through a hash map, which only changes the order in which caches are
  filled;
- node placement is serial.

Within one slicer process T is constant. It is the default TBB arena size, i.e. the number of logical CPUs
respecting the Linux CPU affinity mask (`taskset`, docker `--cpuset-cpus`). **Assumption, unverified:** a
docker `--cpus` quota does not change T. Whether the server's oneTBB build reads cgroup CPU quotas is open
(§7 Q2); if it does, a `--cpus 2` runner is in the T = 2 class.

The Bambu CLI does not limit it (no global control in `src/BambuStudio.cpp`).

**T-classes.** Run the stage with a given T and record, for every merge call, m_L = ⌊(n_L + 2)/4⌋. The
run's decisions are the predicates [m_L ≥ T]. A different T′ makes exactly the same decisions at every layer
iff T′ ∈ (a, b], where:
- a = max{m_L : m_L < T} (0 if none);
- b = min{m_L : m_L ≥ T} (∞ if none).

By induction over layers, every n_L is then identical too, so the whole run is identical. This gives an
exact, deterministic enumeration:

```
T := 1
loop:
    run with T; record {m_L}
    emit realization for class (a, b]
    if b = ∞: stop                      (this class is "all buckets of 2", every T > a)
    T := b + 1
```

- At most (number of distinct m_L) + 1 runs. For a goat (n ≤ ~40, m ≤ 10) that is about ≤ 11 runs.
- **R = class of T = 1**: every layer uses buckets of 4.
- **The owner's 14-thread Mac** is in the "all buckets of 2" class unless some layer has n ≥ 54. So is any
  server with ≥ 14 logical CPUs.

Empirical support: the P1S single-core runs (4/4 identical) lie *outside* the union of the 20 multi-core runs
(tree-support research §3.2). That is consistent with T = 1 being a different class from T = 14.

**The combined realization set of this stage** is
{ F(flavour, O, B_obj, TipLists_{φ,σ}, T_class) : φ ∈ S9 families, σ ∈ tip orders, T_class from the loop
above }, where the tip lists come from part 01 for connector-threshold family φ (01 §5.5 item 4) and tip
order σ (canonical; seeded interleavings only on the optional S1 axis, 01 §5.7). T_class is fixed to {1} for
R, and φ has one value there. The design's ensemble (design §5.4) runs exactly this set, with the T-class
walk done separately for each φ.

---

## 5. Node placement (`create_nodes_from_area`, `TS3:2550-2805` / B 2402-2690)

### 5.1 Bottom nodes

For every element on layer 0: centre := nearest_inside(IA, hint). Then its parents are placed
(§5.2).

hint is the original tip position for an unmerged branch, or the merge hint from §4.5 step 9. The
foot therefore lands **as close as possible under the last merge point or the tip**, but outside the
collision of its final radius. With regular xy and a full foot, that is at least 6.06 mm from O_0, and
≥ 5.955 mm from O_1 through the z-gap (§2.2–2.3).

**Measured check (B, P1S single-object slices `/private/tmp/goatdiag/det/one1.gcode` and `all1.gcode`).**
At z = 0.48 (layer 1) the isolated full-size trunks print as circles of radius 3.53–3.54 mm (r_rec(1) =
3.52). Their centres sit **5.97–6.04 mm** from the same layer's printed body, and 6.0–6.5 mm from the
first-layer body.
- That matches the regular-mode key clearance 5.85 + 0.21 = 6.06 mm.
- It does not match the nominal r + xy = 4.27 mm.
- Up to z ≈ 1 mm the trunks stay at about the same place (5.7–5.9 mm), which is the vertical lazy path of
  §5.2.
- Other trunks of radius about 3.3 mm sit about 4.4–4.5 mm from the body, consistent with the min-xy class
  (≥ 4.11 mm).

### 5.2 Upward, lazy paths

Layers are processed from l = 1 up. For every element e on layer l, in list order:

1. If e has no centre:
   - if e is to_bed (a dead end below it, §3.8), or e.depth < min_depth_model and it does not support a
     roof: **delete e**;
   - otherwise (to-model only): place a model contact (§5.3).
2. If e is not deleted, was not reached from below (not marked) and origin_layer = l, delete it. This is a
   tip that nothing supports.
3. If e is deleted: reset the centre of every parent of e. Those parents are then deleted when their layer is
   processed, so the cascade climbs to the tips.
4. Otherwise, for every parent p of e: if p.centre is unset, set
   **p.centre := nearest_inside(IA_p, e.centre)**, and mark p.

These are the **lazy paths**: a parent stays exactly above its child when the child's point lies in the
parent's IA, and otherwise moves to the nearest point of the parent's IA. Two kinds of centre are already
set and are never overwritten:

- tips: their position is set at insertion;
- dont-move copies (§3.7): the position is copied down.

No slope limit is enforced at placement. The step between consecutive nodes is bounded only by the IA
construction:
- ≤ v_fast + growth in normal moves;
- + Δ at a merge;
- 1.5× in an error move.

### 5.3 To-model contacts (inactive for the goats)

- **Graceful elements.** Walk upward along the single-parent chain while IA ∩ Place(c, layer) ≠ ∅, and stop at
  a merge point (more than one parent). Put the contact at the highest such layer, at
  nearest_inside(IA, hint), and delete the elements below it. If no layer qualifies, fall back
  to the non-graceful rule.
- **Non-graceful elements:** centre := nearest_inside(IA, hint) on the element's own layer.

### 5.4 Compaction (upstream `remove_deleted_elements`, `TS3:2642`)

Layers are compacted from top to bottom. **Order property:** on each layer, the deleted elements at the end
of the list are truncated first; then every remaining deleted slot, lowest index first, receives the current
last surviving element (remove-by-last, `Vec::swap_remove`, after truncating trailing deleted entries). The
surviving elements' parent indices are rewritten through the old → new map of the layer above. This permutes
each layer's order, so part 03 must not assume the pathing order. Test vectors: §8.3.

### 5.5 `nearest_inside(P, x)` and `nearest_boundary(P, x)` (upstream `move_inside_if_outside`, `move_inside`; `TS3:1458-1586`, same in all four)

- **Inside test.** If x is inside P, `nearest_inside` returns x. Inside uses the even-odd count over all
  rings, and a point on a boundary counts as inside. Empty P: x is unchanged.
- **Otherwise** it returns `nearest_boundary(P, x)`: the nearest boundary point among these candidates, on
  every ring with at least 2 vertices, ignoring zero-length edges:
  - **edge candidates:** for an edge a → b with t = (b − a)·(x − a) and L = |b − a|² in 64-bit integers, the
    projection a + trunc((b − a)·(t/L)) when 0 < t < L (computed in double, **truncated toward zero per
    component**);
  - **vertex candidates:** a vertex v whose incoming edge has t ≥ L (x projects at or past its end) and
    whose outgoing edge has t ≤ 0 (x projects at or before its start).
- **Ties.** Distances are squared integer distances. A later candidate wins only if **strictly** closer, so
  ties go to the first ring, then to the first candidate in visiting order. Each ring is visited starting with
  its **closing edge** (q_{k−1} → q₀), then (q₀ → q₁), …; a vertex candidate takes the position of its
  outgoing edge.
- **Result.** The point lies on the boundary to within one grid unit (10 nm in B, 1 nm in Orca). It is not
  pushed inside and may be one unit outside. Because of the per-component truncation it can differ from the
  exact nearest point by up to one unit per axis.

The `distance ≠ 0` mode (inward push by a distance) is not used in this stage. Test vectors: §8.4.

### 5.6 Differences (placement)

None between B, O, S and R (only whitespace and debug asserts differ).

---

## 6. Recipe for a deterministic port of this stage

1. Compute the constants of §1.2 with the exact integer truncations, in the flavour's own unit (B: 10 nm
   grid; Orca: 1 nm grid). Hard-code the speeds instead of calling tan, to avoid S7 in the settings:
   - B: V_f = 27 999 and V_s = 13 056 units;
   - Orca U1: V_f = 234 947 nm and V_s = 130 566 nm.
2. Use `polygon_ops` (design §5.2, lead decision): `i_overlay` on the flavour's grid, emulating the
   Clipper-1 semantics that change discrete decisions:
   - the 0.005·|d| input decimation;
   - the miter-limit floor of 2;
   - the arc-tolerance rule (§1.1);
   - B's strictly-simple simplification union (§1.1 S(τ));
   - 32-bit coordinates for B; 64-bit for Orca, whose orientation tests use double cross products and so
     are not exact for very large coordinate products. Exact int128 predicates can differ there in
     degenerate cases.

   `i_overlay` will not reproduce Clipper-1 vertex for vertex (ring start vertices, component order, vertex
   splitting). Those differences enter δ. The facade stays swappable, and WP S4 reports whether they dominate
   δ (if they do, a Clipper-1-compatible engine replaces `i_overlay` behind the same facade).
3. Build the volumes lazily, keyed by (K, l), with the flavour switches of §2.10.
4. Run §3–§5 single-threaded. Use ordered containers only (`Vec`, `BTreeMap`). Never iterate a hash map.
5. Run the ensemble (design §5.4): for each S9 family of part 01, the exact T-class walk of §4.7 (B, O
   and S; R has the single class T = 1), with the canonical tip order; seeded tip orders only on the
   optional S1 axis.
6. Hand the node lists to part 03.

**Expected fidelity.** Exact up to S7 (±1 unit per vertex, occasionally a flipped ε_A test) and up to the
backend differences of item 2. The envelope margin δ absorbs these. The research's 0.1-0.2 mm figure is not
a bound: the corpus shows realizations several mm outside the known axes (design §3.2 SU, §5.5).

---

## 7. Open questions and validation items

1. **TBB concurrency in the GUIs.** I found no global thread limit in the CLI entry points or in Print.
   Whether the B and S GUIs ever cap TBB (preferences, energy mode) is unverified. Check: slice the P1S
   probe in the GUI and on the CLI on the same Mac and compare L1/bands.
2. **oneTBB and cgroups.** Does the server's TBB build read cgroup CPU quotas when it sizes the default arena?
   If it does, docker `--cpus 2` would mean T = 2, a different S2 class from the owner's GUIs. §4.7 and design
   §3.2 treat "quotas do not change T" as an **assumption** until this is checked (a one-line probe binary
   or a log of `max_concurrency()` on the runner). The ensemble enumerates every T-class anyway, so the
   answer matters only for `--slicer-threads` and for reading the corpus.
3. **Feet clearance from key rounding: partly confirmed.** In the existing single-object P1S G-code (§5.1)
   the two isolated full feet sit 5.97–6.04 mm from the body, against 6.06 mm predicted. Still open:
   - do the 4.4–4.5 mm trunks belong to the min-xy class (≥ 4.11 mm) or to partially grown feet?
   - how far does smoothing (part 03) move nodes in general? Within this sample, ≤ 0.1 mm at the foot.

   A port-vs-G-code comparison per node would settle both.
4. **The mfs bug.** If a user sets `min_feature_size` to 0 %, s drops to 0.75 / 0.21 mm. The wall
   restriction then applies to every step, and upstream's independent fast offset can be selected. The general SOI
   rule in §3.4 covers this, but it has not been exercised. Pin the setting or flag it in the packer.
5. **B's bed ring centroid** uses B_obj's area centroid. For the P1S L-shaped bed (with the corner exclusion)
   the centroid is at about (128.9, 128.9) instead of (128, 128), so the slits sit about 0.9 mm off the bed
   centre. Harmless, but a port must use the same centroid definition.
6. **R for U1.** R puts U1 into the T = 1 class and simplifies collisions. That differs from the owner's
   S GUI in two independent ways. Matching the GUI argues for O on ≥ 14 cores, or for a port that runs
   the S flavour with the bucket-of-2 class, not R.
7. **S1 coupling: answered negatively by the corpus.** The multi-core realizations seen in practice
   (B P1S: 16 line-level realizations in 7 L1 families in 53 rotation-0 observations, M §5.3) are **not**
   explained by tip-order permutations × one T-class as specified: R, with the same race, is deterministic
   (1 realization in 30 observations), the T-class is constant on one machine, and S9 gives two families.
   This stage contributes no other run-to-run variation on a fixed machine. The B source is open (design
   §3.2 SU, Q15).

---

## 8. Test vectors for the order rules

Generated by `/tmp/vec02/vectors.py` (our own reference code written from §4.2, §4.3-§4.4, §5.4 and §5.5).
They encode this part's reading of upstream (`AABBTreeIndirect.hpp:93-205`, `TS3:2270-2444`, `:2642`,
`:1458-1586`). WP S3 checks them once against a recorded upstream run (the private instrumented build of the
review, §5.3) before relying on them. A mismatch means the text above is wrong, not the vector. Coordinates
are grid units. `tests/support_model.rs` carries them verbatim.

### 8.1 AABB order (§4.2)

Input: one box (min x, min y, max x, max y) per element, listed in input order (index 0, 1, …). Output: the
input indices in final order.

| # | boxes | output |
|---|---|---|
| A1 | (0,0,0,0) (10,1,10,1) (5,2,5,2) (−3,0,−3,0) (7,−1,7,−1) (2,3,2,3) (5,0,5,0) | 3 0 5 2 6 4 1 |
| A2 (equal centroid x on one axis, extents tie → x) | (4,0,4,0) (4,5,4,5) (1,1,1,1) (4,2,4,2) (9,9,9,9) (4,1,4,1) (0,8,0,8) (4,3,4,3) | 2 0 5 6 3 7 1 4 |
| A3 (y longer, negative coordinates) | (−5,−20,−3,−18) (0,10,2,12) (−1,−1,1,1) (3,30,5,31) (−4,5,−2,6) (1,−9,2,−8) | 0 5 2 4 1 3 |
| A4 (square extent) | (0,0,0,0) (4,4,4,4) (4,0,4,0) (0,4,0,4) (2,2,2,2) | 0 4 3 2 1 |

### 8.2 Merge rounds (§4.3-§4.4)

Abstract pair test: elements are labels; `try_merge(dst, src)` succeeds iff some label of dst and some label
of src form a listed pair, and the merged element carries the union of labels. The input is already in AABB
order. "+" joins the labels of a merged element.

| # | input | mergeable pairs | T | bucket size | output order |
|---|---|---|---|---|---|
| M1 | a b c d e | ab, ce | 1 | 4 | a+b, d, c+e |
| M1 | a b c d e | ab, ce | ≥ 2 | 2 | a+b, c+e, d |
| M2 | a b c d e f g h | ab, ah, bh, cf | ≤ 2 | 4 | a+b+h, d, c+f, g, e |
| M2 | a b c d e f g h | ab, ah, bh, cf | ≥ 3 | 2 | a+b+h, c+f, d, g, e |
| M3 | a b c d e f g h i | ab, cg, di | ≤ 2 | 4 | a+b, d+i, c+g, f, e, h |
| M3 | a b c d e f g h i | ab, cg, di | ≥ 3 | 2 | a+b, c+g, d+i, f, e, h |
| M4 | a b c d e f g | ab, cg, ef, be | ≤ 2 | 4 | a+b+e+f, d, c+g |
| M4 | a b c d e f g | ab, cg, ef, be | ≥ 3 | 2 | a+b+e+f, c+g, d |

### 8.3 Compaction (§5.4)

Input: per position, 1 = deleted. Output: the original indices of the survivors in final order.

| deleted flags | output |
|---|---|
| 0 1 0 0 1 0 | 0 5 2 3 |
| 1 0 0 1 1 | 2 1 |
| 0 0 1 1 1 | 0 1 |
| 1 1 0 1 0 0 1 | 5 4 2 |

### 8.4 Nearest boundary point (§5.5)

| polygon (rings) | x | result | why |
|---|---|---|---|
| square (0,0) (10,0) (10,10) (0,10) | (5,5) | (5,5) | inside |
| same | (12,5) | (10,5) | edge projection |
| same | (13,13) | (10,10) | vertex wedge |
| same | (−2,−3) | (0,0) | vertex wedge at the closing edge's end |
| same | (10,12) | (10,10) | vertex |
| triangle (0,0) (9,0) (0,7) | (6,6) | (5,3) | exact projection (4.22, 3.72); the offset from a = (9,0), (−4.78, 3.72), is truncated toward zero to (−4, 3) |
| same | (3,8) | (2,5) | exact projection (1.38, 5.92), offset (−7.62, 5.92) → (−7, 5); vertex (0,7) at the same squared distance is no candidate |
| same | (10,−1) | (9,0) | vertex |
| two squares (0,0)-(4,4) and (10,0)-(14,4) | (7,2) | (4,2) | tie between rings: first ring wins |
| same | (7,5) | (4,4) | tie between rings: first ring wins |
| square with a repeated vertex (0,0) (10,0) (10,0) (10,10) (0,10) | (12,−1) | (10,0) | zero-length edge ignored |

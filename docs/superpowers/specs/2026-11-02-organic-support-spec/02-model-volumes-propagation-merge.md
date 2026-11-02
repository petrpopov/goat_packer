# Organic tree supports, part 2: model volumes, influence-area propagation, merging, node placement

2026-11-02. Functional spec for a slicer-free re-implementation of the middle of the Organic tree-support
generator. It starts after the tips have been placed (part 01) and ends with one centre point per branch
node per layer, before organic smoothing and drawing (part 03).

**Licence rule.** BambuStudio, OrcaSlicer and Snapmaker Orca are AGPL-3.0. This document describes behaviour
in our own words: formulas, constants, step order, data flow and edge cases. It contains no source code and no
transliteration. The `file:line` references are there so that an implementer can check a behaviour against
upstream. They are not material to copy. Where exact reproduction needs a precise procedure (the AABB
reordering in §4.2, the compaction in §4.4), the procedure is written here as prose steps at the algorithm
level.

**Versions** (sources in `/tmp/goat_src`):

| tag | slicer | used by |
|---|---|---|
| **B** | BambuStudio v02.08.02.61 (`bambu_0208`) | owner's P1S / A2L GUI and CLI |
| **O24** | OrcaSlicer v2.4.2 (`orca_242`) | server U1 |
| **S24** | Snapmaker Orca v2.4.0 (`snaporca240`) | owner's U1 GUI |
| **O25** | OrcaSlicer v2.5.0-rc (`orca_250rc`) | local `/Applications/OrcaSlicer.app` |

Unless noted, paths are relative to `src/libslic3r/Support/`. `TMV` = `TreeModelVolumes.cpp`, `TS3` =
`TreeSupport3D.cpp`, `TSC` = `TreeSupportCommon.hpp`, `TS3H` = `TreeSupport3D.hpp`. Line numbers are given
as `O24 n / S24 n / O25 n / B n` when they differ.

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
   - **O25's "fix" pins the rule to the T = 1 class.**
   - The owner's GUIs run on a 14-thread M3 Max (`hw.logicalcpu = 14`). For every layer with n ≤ 53 they
     are in the **bucket-of-2 class**, the same class as the server's many-core Orca 2.4.2 run. Local Orca
     2.5-rc is in a *different* class.
3. **Merging happens on every layer.** The "merge every x layers" throttle was written for micrometre units.
   In the slicers' units (1 nm, or 10 nm in B) its maximum is 0, so the condition is always true (§3.2). The `mergelayer` code paths
   are therefore always the ones taken.
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
   O24, S24 and O25 have no bed obstacle. All four clip the support to the bed later (part 03).
8. **For this stage, O24 ≡ S24** on the goat settings. Their differences (roof bookkeeping, two guards that
   never trigger here) do not change node positions. B differs in:
   - where its settings come from;
   - the bed obstacle;
   - its polygon-simplification flavour;
   - **its coordinate grid**: 10 nm units with 32-bit coordinates, against 1 nm with 64-bit in Orca. This
     changes every integer truncation, e.g. V_f = 0.27999 mm against 0.279999 mm, and the tip seed polygon
     has 15 vertices against 45.

   O25 differs in:
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
  | **O24/S24/O25** | **1 unit = 1 nm** (1 mm = 10⁶; Orca switches to 10⁻⁵ only for beds ≥ 2147 mm) | 64-bit | 64-bit coordinates, orientation cross products in *double* (`deps_src/clipper/clipper.hpp:85-97`) | `libslic3r.h:43,60-70` |

  Every constant written in "units" below (rounding guards, the 5-unit extra speed, the integer truncations
  of speeds and radii) is therefore 10× coarser in B than in Orca. Physical thresholds that upstream writes
  in mm are the same in both: ε_A, the 100 nm box epsilon, every offset and arc tolerance.
- **Offsets** use the semantics of the Clipper 1 library (Angus Johnson, v6.4.x, Boost licence; an
  independent port is fine licence-wise), with one Slic3r addition:
  - **Input decimation.** Before offsetting by distance d, a ring drops every vertex closer than
    0.005·|d| to the previously kept vertex, and trailing vertices closer than that to the first vertex
    (O24 `ClipperUtils.cpp:277`, `deps_src/clipper/clipper.cpp:3380`).
  - **⊕m(d)** is a miter-join offset. Upstream passes miter limit 1.2, but Clipper raises any limit below 2
    to 2 (`clipper.cpp:3542`). So corners are mitred out to 2·d and squared off beyond that.
  - **⊕M3(d)** is a miter offset with limit 3 (the library default).
  - **⊕r(d; τ)** is a round-join offset with arc tolerance τ. The effective tolerance is
    τ' = min(τ, |d|/4). A full turn uses π / acos(1 − τ'/|d|) segments, capped at π·|d|
    (`clipper.cpp:3547-3557`). ⊖ means a negative offset.
  - **Offsetting a set of rings** offsets each ring on its own (holes in the opposite sense) and then unions
    the result with the non-zero rule (shrinking adds the usual bounding frame with the negative rule).
  - **"union_ex then offset"** first resolves the input to polygons-with-holes. It then offsets each outer
    contour by +d and each hole by −d, and subtracts the holes.
- **Booleans** (∪, ∩, ∖) use the non-zero fill rule. `diff_clipped` first crops the clip set to the
  subject's bounding box; the result is the same.
- **S(τ) = simplify.** For each ring: Douglas–Peucker with tolerance τ on the closed ring (first vertex
  repeated, then dropped), then a self-union of that ring with the non-zero rule, with holes reoriented back.
  The mode of that union differs by flavour:

  | flavour | union mode |
  |---|---|
  | B | "strictly simple" (`Polygon.cpp:658`) |
  | O24, S24 | not strictly simple (`TSC:591` constant `false`) |
  | O25 | plain union (`Polygon.cpp:685`), equivalent to O24 |

  The difference is vertex-level only: touching vertices are split or not, and the area is the same.
- **ε_A** = 10⁻⁶ mm² = (0.001 mm)². "Area > ε_A" is the test for "non-empty".
- **U\*(A, B)** is the "safe union": A ∪ B. If that comes out empty although an input was non-empty, the
  result is instead the union of every input ring's boundary thickened by ⊕m(0.002 mm) (open butt ends)
  (`TS3:749`, B `SupportCommon.cpp:2129`).

### 1.2 Derived settings (evaluated for the goat projects)

Formulas follow `TreeSupportSettings` (`TSC:267` / S24 268 / O25 285 / B 258). Setting names are the
project keys.

| symbol | meaning, formula | B (P1S, A2L) | O24/S24/O25 (U1) |
|---|---|---|---|
| lh | `layer_height` (the first layer is not used here) | 0.28 mm | 0.28 mm |
| w | support line width | 0.42 mm | 0.42 mm |
| θ_f | max branch angle. B: `tree_support_branch_angle`; Orca: `tree_support_branch_angle_organic`. Clamped below 90° | 45° | 40° |
| θ_s | preferred ("slow") angle. B: hard-coded 25° (`TSC:80`); Orca: `tree_support_angle_slow`. Clamped below θ_f | 25° | 25° |
| V_f | trunc(tan θ_f · lh), in units | **27 999 units of 10 nm = 0.27999 mm** (tan 45° evaluates to 0.9999999999999999) | **234 947 nm** |
| V_s | trunc(tan θ_s · lh) | 13 056 units = 0.13056 mm | 130 566 nm |
| V_f′, V_s′ | the volumes' copies, V − 2 units ("rounding guard", `TMV:63`) | 27 997 / 13 054 units | 234 945 / 130 564 nm |
| r_tip | tip radius. B: hard-coded 0.8/2; Orca: `tree_support_tip_diameter`/2, clamped to ≤ branch diameter | 0.4 mm | 0.4 mm |
| r_b | branch radius. B: `tree_support_branch_diameter`/2; Orca: `_organic`/2 | 1.0 mm | 1.0 mm |
| N_tip | tip layers = max(⌊(r_b − r_tip)/⌊w/3⌋⌋, ⌊r_b/lh⌋), integer division in units | 4 | 4 |
| g | radius growth per layer = tan(`tree_support_branch_diameter_angle`)·lh | 0.0244968 mm | same |
| R_inc | "increase radius until" radius = r_b | 1.0 mm | same |
| N_inc | its layer = N_tip · ⌊R_inc/r_b⌋ | 4 | 4 |
| r_bp | trunk foot target, hard-coded 7.5/2 | 3.75 mm | same |
| g_bp | foot growth per layer = min(tan(0.7 rad)·lh, w/2) | 0.21 mm | same |
| L_bp | layers of foot zone = trunc((r_bp − r_b)/g_bp) (an integer, not 13.1) | **13** | 13 |
| g_ft | foot increment per unit of "foot count" = g_bp − g (double) | 0.1855032 mm | same |
| g_ft′ | the same, truncated to units (used only in §3.6) | 18 550 units = 0.18550 mm | 185 503 nm |
| xy | `support_object_xy_distance` (B: at least 0.01; B lowers it to max(top gap, 0.2) if `top_z_overrides_xy_distance`, which is off here) | 0.75 mm | 0.75 mm |
| xy_min | min(xy, ½·external-perimeter width) | 0.21 mm | 0.21 mm |
| δ | xy − xy_min | 0.54 mm | 0.54 mm |
| z_top, z_bot | round(gap / lh). The gaps are first snapped to multiples of lh (`Slicing.cpp`: O24 164-187, B 115-123) | 1, 1 | 1, 1 |
| res | `resolution` | 0.03 mm | 0.03 mm |
| mfs | `min_feature_size` *percentage value read as millimetres* (bug, §0.4) | **25 mm** | **25 mm** |
| rests_on_model | ¬`support_on_build_plate_only` | **false** | **false** |
| min_dtt_model | ⌈1.0 mm / lh⌉ (to-model only) | 4 | 4 |
| Δr_model,max | 1.0 mm / 2 (to-model only) | 0.5 mm | 0.5 mm |

**Radius functions** (`TSC:545-563`, identical in all four):

- **R(d, f)**, the radius of a node with radius height d and foot count f:
  - base(d) = r_tip + ⌊(r_b − r_tip)·d / N_tip⌋ for d ≤ N_tip (integer arithmetic);
  - base(d) = r_b + (d − N_tip)·g for d > N_tip;
  - R(d, f) = trunc(base(d) + f·g_ft), with the sum taken in double.

  Values: R(0..4) = 0.40, 0.55, 0.70, 0.85, 1.00 mm. R(5) = 1.0245 mm. R(89) = 3.082 mm.
- **recMin(l)**, the recommended minimum radius at layer l: r_b + (L_bp − l)·g_bp for l < L_bp, else 0;
  truncated. recMin(0) = **3.73 mm**, recMin(12) = 1.21 mm.
- **effective radius height** eDTT(e) = (e.erh < N_inc) ? min(e.dtt, N_inc) : e.erh.
- **real radius** ρ(e) = R(eDTT(e), e.foot). **Collision radius** c(e) = R(e.erh, e.foot) ≤ ρ(e).

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
  - target_height = l; target_position = next_position = result = p;
  - erh = dtt = 0, foot = 0;
  - flags to_buildplate, to_model_gracious, can_use_safe_radius (from the tip's line status), use_min_xy
    (= xy > xy_min, true here), supports_roof, skip_ovalisation;
  - dont_move_until (3 for an ordinary tip with 3 interface layers);
  - roof-recovery counters (only part 03 uses them).
- **B_obj** (Bambu only): the bed polygon in the frame of this object's first instance (§2.8).

**Output** (to part 03): for each layer l, a list of nodes. Each node has:
- a centre `result`;
- its state (erh, dtt, foot, flags);
- the indices of its parents in layer l+1;
- its IA.

Deleted nodes have been removed and the indices remapped (§5.4).

---

## 2. Model volumes (`TreeModelVolumes`)

All volumes are per-object. The generator never sees neighbouring objects.

### 2.1 Layer outlines

O_l = S(res)(slice of layer l) (`TMV:115` / B 133). B then appends the bed-ring pieces (§2.8) to *every*
O_l, after simplification.

### 2.2 Radius keys (cache quantization) — `ceilRadius` (`TMV:835` / S24 834 / O25 894 / B 853)

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
- **Next ceil:** nextCeil(r, min) = q(r) when min is set, otherwise q(r + δ) − δ. It is used when the radius
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

### 2.3 Collision Coll(K, l) (`TMV:416-580` / S24 416 / O25 468 / B 435)

Collision is the set of centre positions where a branch of key K would hit the model on layer l. The rule
uses the z-gap layers on both sides:

```
Coll(K, l) = ∪_{j=max(0, l−z_bot)}^{l}            ( union_ex(O_j) ⊕m (K + xy_min) )
           ∪ ∪_{i=1}^{z_top}, l+i<N_obj          ( union_ex(O_{l+i}) ⊕m (K + ρ_i) )
           [O25 only: ∪ ( union_ex(Blocker_l) ⊕m K )]
then: union (non-zero); O25 additionally applies S(res).
```

- **ρ_i**, the reduced clearance toward the layers above:
  ρ_i = trunc(xy_min − (i − h)·xy_min / z_top), with h = ½ when z_top = 1 and h = 0 otherwise. With
  z_top = 1, **ρ_1 = 0.105 mm**.
  - The ½ keeps support from touching the model on the downward diagonal when the gap is a single layer.
  - Upstream's reason for reducing ρ at all: a full xy above would wipe out thin overhangs that the tips
    should sit under.
- **Layers below.** The layer itself and the z_bot layers below it are obstacles at full xy_min.
- When K + xy_min = 0, the offset is skipped and only the union remains. That never happens here.
- **Blockers.** In O24, S24 and B the generator loads support-blocker volumes but never adds them. A
  "last mesh" test is off by one (`TMV:472`, B 490), so the branch that would add them never runs.
  - The same bug also skips the final S(res), so collisions in O24/S24/B stay **unsimplified** after the
    union.
  - O25 fixes the test (`TMV:525`). There, blockers enter, offset by K with no xy, and collisions are
    simplified.
  - No goat project has blockers. The only goat-relevant O25 difference is the S(res) simplification,
    a vertex-level change of up to 0.03 mm.
- **Edges.** The ring set for layer j < 0 is empty. Layers at or above N_obj (the object's layer count, which
  is larger than N) are skipped.

### 2.4 Hole-free collision CollHF(K, l) (`TMV:582-612`)

CollHF is defined only for K < R_inc + δ = 1.54 mm, i.e. keys 0.4 and 1.5:

```
CollHF(K, l) = S(res)( union_ex( Coll(K_R, l) ) ⊕r( 5 units − (K_R − K) ; res ) ),
               K_R = K(R_inc, regular) = q(1.54 mm) = 2.6 mm
```

- In words: take the 2.6 mm collision (offset 2.81 mm) and shrink it back by 2.6 − K.
- The result is a **morphological closing** of the model outline at radius ≈ 2.81 mm, expressed at key K.
  Gaps and holes too narrow for a 2.81 mm disc become solid.
- "Safe" avoidances use it, so that branches near the tip cannot route through holes smaller than the
  radius they will soon grow to.

### 2.5 Avoidance Avoid_τ,t(K, l) (`TMV:614-714` / S24 614 / O25 673 / B 632)

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
  exists, and to-model avoidances only when rests_on_model. **The goat projects use only the three to-plate
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

Place(K, l) = union_ex(Place_0(l)) ⊖m (K + xy_min + δ) for l ≥ 1. Place(K, 0) is empty: the plate is not
"model", and the machine-border term upstream is always empty here. In O24, S24 and B, Place_0 is
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

### 2.8 Machine border and bed exclusions (`TMV:54-90` B; `TMV:39-52` O24/S24/O25)

**B_obj, the bed in the object frame** (all versions compute it; `TreeSupport.cpp` O24 654 / S24 627 /
O25 698 / B 626):

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

**O24 / S24 / O25:** the border obstacle is explicitly empty (`TMV:39-52`). B_obj is used only to clip
support layers afterwards (part 03).

### 2.9 Caching, laziness, cost

- **Storage.** Each volume kind (Coll, CollHF, Avoid ×3 types ×2 targets, Place, WR, WR_min) is a map
  layer → (key → polygons). Results are pure functions of (K, l) and the inputs.
- **Precalculation and laziness.** Upstream precalculates a key set:
  - avoidance keys = {q(R(d)+δ), q(R(d)), q(recMin(l)+δ)} over the layers where they can occur;
  - collision keys additionally {0, δ, q(R_inc + δ)} on all layers.

  It computes anything else on demand. Avoidance is a bottom-up recurrence, so computing it lazily in
  increasing l gives identical results. **A port may compute everything lazily with memoization.** The goat
  keys are 0, 0.4, 0.54, 1.5, 2.6, 3.9, 5.85 mm.
- **Cost.** About N ≈ 93 layers × 7 keys × (z_bot + 1 + z_top) offsets for collisions. Avoidance is
  3 types × ~5 keys × 93 *serial* steps, each one shrink + union + simplify. Wall restrictions are about
  5 keys × 93 × 2 intersections. Each offset costs O(V log V) with V ≈ 10²–10³ outline vertices. The whole
  stage takes well under a second, single-threaded, for one goat.

### 2.10 Differences (volumes)

| item | B | O24 | S24 | O25 |
|---|---|---|---|---|
| bed outside + exclusions as obstacle | **yes** (ring, slits) | no | no | no |
| collision simplified after union | no | no | no | **S(res)** |
| support-blocker volumes as obstacle | no (off-by-one) | no | no | **yes, ⊕m K** |
| S() union mode | strictly simple | not strict | not strict | plain |
| early exit if avoidance step ≈ 0 | present | present | removed | present (never triggers) |
| everything else in §2 | same | same | same | same |

---

## 3. Influence-area propagation (`create_layer_pathing`, `increase_areas_one_layer`, `increase_single_area`)

### 3.1 Element state (`TS3H:79-233`)

Each element (branch node) stores the following.

- **Identity and position:** layer l, dtt (layers below its highest tip), target_height and
  target_position (the tip it ultimately serves), next_position (a position hint, §4.5), result (centre;
  "unset" is a sentinel).
- **Radius bookkeeping:** erh (radius height, normally equal to dtt but allowed to lag), foot (double,
  elephant-foot count), increased_to_model_radius.
- **Flags:**
  - to_buildplate;
  - to_model_gracious;
  - use_min_xy (still allowed the reduced xy distance);
  - supports_roof;
  - can_use_safe_radius ("past any holes": may use Slow and FastSafe avoidances);
  - skip_ovalisation;
  - deleted, marked (scratch);
  - lost, verylost (debug only).
- **Movement:** dont_move_until (a dtt threshold), and last_area_increase (the setting that produced the
  element; geometrically irrelevant here, §3.2).
- **Geometry:** parent indices into layer l+1, and the IA (a polygon set of allowed centres).
- **Merging copy** (only while merging): the IA, the to-plate part P_bp, the to-model part P_m, and the
  bounding box of all three grown by 100 nm.

Moving an element one layer down ("propagate down"): dtt + 1, layer − 1, result reset to unset,
skip_ovalisation cleared. O24/O25 also decrement a pending roof-recovery counter (part 03 only).

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

P is the parent on layer l; C = propagate_down(P), with erh_C = erh_P and foot_C = foot_P. r_P = c(P).

```
e  = 5 units                 (added to both speeds; 50 nm in B, 5 nm in Orca)
e_s = 0                      (added to the slow speed only)
r⁺ = R(erh_P + 1, foot_P);   Δr = r⁺ − r_P
if K(r⁺, P.min) == K(r_P, P.min)  or  r⁺ < R_inc:   e  += Δr
else:                                                e_s += min(Δr, (V_f + e) − (V_s + e_s))
if l < L_bp  and  recMin(l−1) < R(erh_C + 1, foot_C):        (literally "<", see note)
     if K(R(erh_P+1, foot_P+1), P.min) == K(r_P, P.min):    e  += g_bp
     else:                                                   e_s += min(g_bp, V_f − (V_s + e_s))
v_fast = V_f + e
v_slow = V_s + e + e_s
s      = (C.min ? xy_min : xy) + (min(z_top, z_bot) > 0 ? mfs : 0)   (O24/O25: if s = 0 then 0.1 mm)
```

Typical values (B, θ = 45°):

| zone | v_slow | v_fast |
|---|---|---|
| tip (Δr = 0.15 mm, r⁺ < 1 mm) | 0.281 mm | 0.430 mm |
| trunk (Δr = 0.0245, key unchanged) | 0.155 mm | 0.305 mm |
| foot zone with bonus | +0.21 mm | +0.21 mm |

For U1 (θ = 40°), v_fast is 0.045 mm lower.

- **Foot bonus condition.** The bonus is granted when the branch is *already wider* than recMin, which is
  exactly when §3.6 plans no foot growth. Reproduce it literally.
- **s with the mfs bug.** s = 25.21 or 25.75 mm.

### 3.4 Stepped safe offset SOI(A, d, W, s, c, n) (`TS3:792` / B `SupportCommon.cpp:2187`)

SOI grows A by d without jumping across the wall set W in one step. W is pre-cropped to bbox(A) + d.

1. final := (c = 0); A := U\*(A).
2. If d = 0: return final ? A ∖ W : union(A).
3. k := (d > c) ? ⌊(d − c)/s⌋ : 0.
4. If d − k·s > c: if (k+1)·s ≤ d then k := k+1, else final := true.
5. step := s. If n > 1 and k + [d < c or d mod s ≠ 0] < n:
   step := ⌊d/n⌋; if step ≥ s then step := s, k := n; else k := ⌊d/step⌋.
6. Repeat k times (i = 0…k−1): A := (A ⊕r(step; 0.01 mm)) ∖ W. After steps with i mod 10 = 7, apply
   S(0.015 mm).
7. rest := d − k·step. If rest > 100 nm: A := A ⊕r(rest; 0.01 mm).
8. A := S(0.015 mm)(A). If final: A := A ∖ W. Return union(A).

**Evaluated for the goats (s ≈ 25 mm ≫ every d).**

- **Slow area** A_slow = SOI(IA_P, v_slow, W, s, 0, 2) gives *two half-steps*. Each half-step is a round
  offset by ⌊v_slow/2⌋ followed by subtracting W, and a final S(0.015) and ∖W follow.
- **Fast area.** In upstream "offset_independent_faster" is always false here. The fast area is therefore
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

- **If ¬C.can_use_safe_radius** (still possibly above a hole). None of the first three entries move: they
  test whether the unmoved IA is already outside the safe avoidance.
  1. (Slow, slow, grow, ok, regular, **no move**);
  2. only if C.dtt < ⌈N_tip/2⌉ = 2: (Fast, slow, grow, ok, regular, no move);
  3. (FastSafe, fast, grow, ok, regular, no move);
  4. (FastSafe, fast, keep, ok, regular, move);
  5. (Fast, fast, keep, ok, regular, move).
- **Else:**
  1. (Slow, slow, grow, ok, regular, move);
  2. (Slow, slow, keep, ok, regular, move);
  3. only if C.dtt < N_tip: (FastSafe, slow, grow, ok, regular, move);
  4. (FastSafe, fast, grow, ok, regular, move);
  5. (FastSafe, fast, keep, ok, regular, move).

  There is no plain Fast normal move in this branch. For K ≥ 1.54 mm, FastSafe *is* Fast (§2.5).
- **If C.min:** every entry above is followed by its twin with min = true. The regular variant is always
  tried first.
- **Error entry**, appended if C.to_buildplate, or if C.to_model_gracious and IA_P ∩ Place(r, l) = ∅:
  (Fast, fast, keep, **error**, min = C.min, move).
- **Dont-move entry**, prepended if C.dtt < C.dont_move_until and C.can_use_safe_radius:
  (Slow, 0, grow, ok, regular, no move).

  With 3 top interface layers, ordinary tips have dont_move_until = 3. A tip column therefore stays
  vertical for 2 layers below the tip, if the Slow test allows it, and starts moving at dtt = 3.

The settings are tried in order and **the first valid one wins** (§3.6). If none is valid, the element fails
(§3.8).

### 3.6 Evaluating one setting (`increase_single_area`, `TS3:1588-1724` / B 1464)

Input: the setting σ, the area A (A_slow if σ.speed = slow, A_fast if fast; for the error entry, the error
area), and the parent P. Output: the new state C′ or "invalid".

1. C′ := propagate_down(P); if σ.grow: C′.erh += 1. r := c(C′).
2. X := (σ.move) ? A : IA_P. X is the "increased area without collision" and is kept for §3.7.
3. **To-plate part:** P_bp := U\*(X ∖ Avoid_σ.τ,plate(r, l−1, σ.min)). If C′ is not yet to_buildplate and
   area(P_bp) > ε_A, set to_buildplate (a "taint correction").
4. **To-model part** (only if rests_on_model): P_m := U\*(X ∖ Avoid_σ.τ,model(…)). If C′ is not yet
   gracious, then:
   - if area(P_m) ≥ ε_A, set gracious and keep P_m;
   - otherwise replace it by P_m := U\*(X ∖ Coll(r, l−1, σ.min)), i.e. route down anyway and land on the
     model non-gracefully.
5. Q := to_buildplate ? P_bp : P_m.
6. **Radius growth** (only if σ.grow and area(Q) > ε_A).
   - **validWithRadius(r′)** is true if K(r′, σ.min) ≤ K(r, σ.min). Otherwise it is
     area(X ∖ Avoid_σ.τ,·(r′, l−1, σ.min)) > ε_A, with the target avoidance (plate, or model/collision as
     in step 4).
   - Let K_before := K(r, σ.min).
   - **Catch-up** (radius lagging behind dtt). If c(C′) < R_inc and c(C′) < ρ(C′):
     - target := min(ρ(C′), R_inc) and ν := nextCeil(r, σ.min);
     - while ν < target and validWithRadius(nextCeil(ν + 1 unit, σ.min)): ν := that value;
     - then raise C′.erh one step at a time while erh + 1 < C′.dtt, R(erh+1, foot) ≤ ν and
       R(erh+1, foot) ≤ ρ(C′).
   - r := c(C′).
   - **Elephant foot.** Let g_ft′ = trunc(g_bp − g) (integer here, unlike in R).
     - planned := min(1, (recMin(l−1) − ρ(C′)) / g_ft′).
     - If planned > 0, C′ is to_buildplate, ρ(C′) ≥ r_b, ρ(C′) ≥ R_inc and
       validWithRadius(R(C′.erh, C′.foot + planned)), then C′.foot += planned and r := c(C′).
     - So a trunk gains up to g_bp = 0.21 mm of radius per layer: 0.0245 from dtt growth plus 0.1855 from
       foot growth. Below layer 13 it tracks recMin (3.73 mm at layer 0) whenever the bigger avoidance still
       leaves room. Where it does not, the foot simply stays thinner.
   - If K(r, σ.min) ≠ K_before: recompute P_bp (and P_m) with the new r, and set Q accordingly.
7. Valid iff area(Q) > ε_A.

**Bug kept on purpose.** A simplification at 0.025 mm of X in the no-error case is computed and thrown away
upstream (`TS3:1626`). A port must **not** simplify X there.

### 3.7 Result of a move (`TS3:1983-2040`)

On the first valid σ:

- C := C′, and r := c(C).
- **bypass** := ¬σ.move or (σ.min and C.dtt < N_tip).
- If σ.move: dont_move_until := 0. Otherwise: C.result := P.result. A tip's own position is copied down, so
  dont-move columns are exactly vertical.
- can_use_safe_radius := (σ.τ ≠ Fast).
- If σ is regular: C.min := false. A branch never goes back to min mode.
- **Stored IA:**

  ```
  IA_C = U*( (X ∖ Coll(r, l−1, C.min)) , U*(P_bp, P_m) )
  ```

  The IA is bounded by *collision* at the final radius. Avoidance only gated validity and produced P_bp.
- For merging, keep P_bp (if to_buildplate) and P_m (if rests_on_model), unless the element is a bypass.
  Set the bbox to extents(IA_C) ± 100 nm.

### 3.8 When a branch is dropped

- **Failure.** If no setting is valid, the error move included, the candidate gets an empty IA and is dropped
  (§3.2 step 3). In addition, the *parent's* result is reset and its gracious flag cleared. In node placement
  (§5) the parent then has no centre, so it is deleted together with **every element above it in that
  subtree**: the reset cascades up through all parents, up to their tips.
- **Error moves.** An error move ("teleport") only allows 1.5 × fast speed with the Fast avoidance and no
  wall check. A branch that survives one is flagged "lost" (debug only) and loses can_use_safe_radius.
- **To-model branches** may also be deleted in placement (§5.3). That never happens in the goat projects:
  every tip there is to-plate, and the AND in §4.5 keeps every element to-plate.

### 3.9 Differences (propagation)

| item | B | O24 | S24 | O25 |
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

M is reordered in place by building a balanced median-split tree over the candidate bounding boxes. Only the
resulting order is used; the tree itself is discarded. A port needs exactly this permutation.

`build(lo, hi)` on an index range, inclusive:

1. If lo = hi, stop.
2. Compute the union box of the range. **axis** = the axis with the larger extent, x on a tie.
3. mid := ⌊(lo + hi)/2⌋. Run **select(lo, hi, axis, k = mid)** (below).
4. build(lo, mid), then build(mid + 1, hi).

`select(lo, hi, axis, k)` is a quickselect with a median-of-three pivot and a Hoare-style partition. v(i)
means the centroid of the element at i on that axis. While lo < hi:

1. c := ⌊(lo + hi)/2⌋.
2. Sort the three entries at lo, c and hi in place with three conditional swaps:
   - (lo, c) if v(lo) > v(c);
   - then (lo, hi) if v(lo) > v(hi);
   - then (c, hi) if v(c) > v(hi).
3. pivot := v(c).
4. If hi ≤ lo + 2, stop: the range is already ordered.
5. Swap the entries at c and hi − 1. Set i := lo, j := hi − 1.
6. Repeat:
   - advance i by one at least once, until v(i) ≥ pivot (position hi − 1 stops it);
   - move j back by one at least once, until v(j) ≤ pivot or j ≤ i;
   - if i ≥ j, leave the loop; otherwise swap i and j.
7. Swap the entries at i and hi − 1, which puts the pivot at i.
8. If k < i: hi := i − 1. If k = i: stop. Otherwise: lo := i + 1.

Quickselect is not a stable sort, so **the result depends on the input order of M**. This is how the tip
order (S1) reaches every later merge.

### 4.3 Buckets (S2) (`TS3:2380-2403` / B 2236-2259; O25 2428-2445)

- **B, O24, S24.** T = `tbb::this_task_arena::max_concurrency()`.
  - If m := ⌊(n + 2)/4⌋ ≥ T: **m buckets of 4**.
  - Otherwise: **⌊n/2⌋ buckets of 2**.
- **O25:** always m buckets of 4. That is exactly the old rule with T = 1.

Buckets are consecutive ranges of the reordered M, starting at 0:
- If the last bucket runs past n, it is cut at n. It then has size 2 or 3 with size-4 buckets, never fewer
  than 2.
- If exactly one element is left over, it becomes an extra bucket of size 1 that is not merged in round 1.

Thread count only decides the bucket size; the buckets themselves are processed independently.

### 4.4 Merge rounds

**Round 1: within each bucket ("leaves").** For a bucket range [a, e):

```
i := a
while i + 1 < e:
    for j := i+1 … e−1:
        if pairMerge(dst = M[i], src = M[j]) succeeds:
            move M[e−1] into slot j;  e := e − 1
            restart the scan for the same i from j := i+1      (i absorbs again)
    if no pair succeeded in a full scan: i := i + 1
```

The bucket's end becomes e.

**Later rounds: pairwise over neighbouring buckets.** In each round, bucket 2k (dst set D) absorbs bucket
2k+1 (src set S). An odd last bucket is carried over unchanged. Then the list of buckets is halved. Merging
two sets works like this:

```
for each src position s in S, left to right:
    find the first d in D (left to right) with pairMerge(dst = D[d], src = S[s]);
    if found:
        the absorbed src slot is refilled: the earliest still-unabsorbed src (at S's
        moving start pointer) is moved into slot s, and the start pointer advances
        then the absorbing D[d] tries every LATER element of D (positions d+1 … end) as src:
            on success that element is removed by moving D's last element into its slot
            (D shrinks; the same slot is re-tested)
after all srcs: the unabsorbed srcs, which now occupy [start pointer, end of S) in their
current order, are appended right after the (possibly shrunk) D
```

Two properties follow:

- **The destination is always the element on the left side** (lower position in the AABB order). It keeps
  its identity, its parents first, and its next_position hint (§4.5).
- **Unabsorbed srcs are permuted.** Each absorption moves the oldest survivor into the absorbed slot, so the
  survivors do *not* keep their original relative order. A port must reproduce this.

The merged layer is the final range of bucket 0. **Its order is the order of the next layer.** Positions
past its end hold emptied elements, which the caller skips because their IA is empty.

### 4.5 Pair merge test and result (`TS3:2093-2269` / B 1960)

pairMerge(dst, src) on layer step l → l−1:

1. **Rejects.** Refuse the merge if:
   - dst.gracious ≠ src.gracious; or
   - dst.min ≠ src.min (min-xy and regular branches never merge).
2. **Bigger and smaller.** bigger := dst if c(dst) > c(src), else src (**ties → src is "bigger"**).
   smaller := the other. Δ := |ρ(bigger) − ρ(smaller)|, using real radii.
3. **Box test.** smaller.bbox grown by Δ must intersect bigger.bbox (closed boxes: touching counts).
   Otherwise refuse.
4. **to_bp merge** := dst.to_buildplate ∧ src.to_buildplate. If false (to-model only):
   - if one is to-plate and one to-model, the to-model one may grow by the radius difference. Refuse if its
     accumulated increase would exceed Δr_model,max;
   - refuse if neither supports a roof and max(dtt) < min_dtt_model.
5. Refuse if bigger is not can_use_safe_radius but smaller is.
6. useMin := bigger.min ∧ smaller.min. Wcoll := Coll(c(smaller), l−1, useMin).
   **grow(X) := SOI(X, Δ, Wcoll, s′ = 2·(xy + c(smaller) − 3 units), c = 0, n = 0)**. Here xy is the regular
   0.75 mm even in min mode. Because s′ ≥ 2.3 mm > Δ, this is one round offset ⊕r(Δ; 0.01) when Δ > 100 nm,
   then S(0.015), then ∖ Wcoll. When Δ = 0 it is just U\*(X) ∖ Wcoll.
7. **I := grow(smaller.P) ∩ bigger.P**, with P = P_bp for a to_bp merge and P_m otherwise.
8. Refuse if area(I) ≤ ε_A, or if area(I ⊖m 0.025 mm) ≤ ε_A. The second test means the overlap must be at
   least about 0.05 mm thick.
9. **Hint.** new_next := moveInsideIfOutside(I, **dst**.next_position) (§5.5). That is the point of the
   merge overlap closest to the destination's previous hint (the original tip position for an unmerged
   branch).
10. **Merged state:**
    - layer l−1, next_position = new_next;
    - min, supports_roof, can_use_safe_radius: OR;
    - dont_move_until, erh, dtt: max;
    - to_buildplate, gracious: AND;
    - target_height/target_position from the one with the higher target_height (**ties → src**);
    - foot := |max(c(dst), c(src)) − R(erh, 0)| / (g_bp − g). This is the foot count that makes the merged
      collision radius equal the larger input's;
    - increased_to_model_radius: the value from step 4 if one was set, else the max of both;
    - result unset, deleted/marked cleared;
    - last_area_increase: an elementwise min/OR/AND mix, irrelevant here;
    - roof bookkeeping: O24/O25 keep the max of the pending roof-recovery counts and depths; B/S24 keep the
      min of the missing roof count (part 03).
11. **Merged areas:**
    - IA := U\*(grow(smaller.IA) ∩ bigger.IA, I);
    - for a to_bp merge: P_bp := I, and if rests_on_model, P_m := (both gracious ? U\*(grow(smaller.P_m)
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
| S2 | bucket size from T (B, O24, S24) | different pairings, so different merges, positions and later n | one per **T-class** (below) |
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

Within one slicer process T is constant. It is the default TBB arena size, i.e. the number of logical CPUs:
- respecting the Linux CPU affinity mask (`taskset`, docker `--cpuset-cpus`);
- *not* the docker `--cpus` quota.

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
- **O25 = class of T = 1**: every layer uses buckets of 4.
- **The owner's 14-thread Mac** is in the "all buckets of 2" class unless some layer has n ≥ 54. So is any
  server with ≥ 14 logical CPUs.

Empirical support: the P1S single-core runs (4/4 identical) lie *outside* the union of the 20 multi-core runs
(tree-support research §3.2). That is consistent with T = 1 being a different class from T = 14.

**The combined realization set of this stage** is
{ F(flavour, O, B_obj, TipLists_σ, T_class) : σ ∈ tip orders from part 01, T_class from the loop above },
with T_class fixed to {1} for O25.

---

## 5. Node placement (`create_nodes_from_area`, `TS3:2550-2805` / B 2402-2690)

### 5.1 Bottom nodes

For every element on layer 0: result := moveInsideIfOutside(IA, next_position). Then its parents are placed
(§5.2).

next_position is the original tip position for an unmerged branch, or the merge hint from §4.5 step 9. The
foot therefore lands **as close as possible under the last merge point or the tip**, but outside the
collision of its final radius. With regular xy and a full foot, that is at least 6.06 mm from O_0, and
≥ 5.955 mm from O_1 through the z-gap (§2.2–2.3).

**Measured check (B, P1S single-object slices `/private/tmp/goatdiag/det/one1.gcode` and `all1.gcode`).**
At z = 0.48 (layer 1) the isolated full-size trunks print as circles of radius 3.53–3.54 mm (recMin(1) =
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

1. If e has no result:
   - if e is to_buildplate (a dead end below it, §3.8), or e.dtt < min_dtt_model and it does not support a
     roof: **delete e**;
   - otherwise (to-model only): place a model contact (§5.3).
2. If e is not deleted, was not reached from below (not marked) and target_height = l, delete it. This is a
   tip that nothing supports.
3. If e is deleted: reset the result of every parent of e. Those parents are then deleted when their layer is
   processed, so the cascade climbs to the tips.
4. Otherwise, for every parent p of e: if p.result is unset, set
   **p.result := moveInsideIfOutside(IA_p, e.result)**, and mark p.

These are the **lazy paths**: a parent stays exactly above its child when the child's point lies in the
parent's IA, and otherwise moves to the nearest point of the parent's IA. Two kinds of result are already
set and are never overwritten:

- tips: their position is set at insertion;
- dont-move copies (§3.7): the position is copied down.

No slope limit is enforced at placement. The step between consecutive nodes is bounded only by the IA
construction:
- ≤ v_fast + growth in normal moves;
- + Δ at a merge;
- 1.5× in an error move.

### 5.3 To-model contacts (inactive for the goats)

- **Gracious elements.** Walk upward along the single-parent chain while IA ∩ Place(c, layer) ≠ ∅, and stop at
  a merge point (more than one parent). Put the contact at the highest such layer, at
  moveInsideIfOutside(IA, next_position), and delete the elements below it. If no layer qualifies, fall back
  to the non-gracious rule.
- **Non-gracious elements:** result := moveInsideIfOutside(IA, next_position) on the element's own layer.

### 5.4 Compaction (`remove_deleted_elements`, `TS3:2642`)

Process layers from top to bottom:

1. Drop trailing deleted elements.
2. When a deleted element is found at i, drop the trailing deleted ones again, then move the last element
   into slot i and re-examine slot i.
3. Rewrite the surviving elements' parent indices through the old→new map of the layer above.

This permutes each layer's order. Part 03 must not assume the pathing order.

### 5.5 `moveInsideIfOutside(P, x)` and `moveInside` (`TS3:1458-1586`, same in all four)

- **Inside test.** If x is inside P, return x. Inside uses the even-odd count over all rings, and a point on
  a boundary counts as inside.
- **Otherwise,** return the point of P's boundary nearest to x, computed as follows. For each ring
  q₀…q_{k−1} with k ≥ 2, visit its edges in the order (q_{k−1}→q₀), (q₀→q₁), …, (q_{k−2}→q_{k−1}). Keep a
  flag "x projects beyond the end of the previous edge", initialised from the edge (q_{k−2}→q_{k−1}). For
  each edge a→b, with t = (b−a)·(x−a) and L = |b−a|² in 64-bit integers:
  - L = 0: skip the edge (a advances).
  - t ≤ 0 and the flag is set: candidate = vertex a. Clear the flag.
  - t ≤ 0 and the flag is clear: no candidate.
  - t ≥ L: no candidate; set the flag.
  - Otherwise: candidate = a + trunc((b − a)·(t/L)), computed in double and truncated per component. Clear
    the flag.

  A candidate replaces the best one only if its squared integer distance is **strictly** smaller, so ties
  keep the first ring and the first edge.
- **Result.** The returned point lies on the boundary, within one unit (10 nm in B, 1 nm in Orca). It is
  not pushed inside, and it may be one unit outside.
- **Empty P:** x is unchanged.

The `distance ≠ 0` mode of `moveInside` (inward push by a distance) is not used in this stage.

### 5.6 Differences (placement)

None between B, O24, S24 and O25 (only whitespace and debug asserts differ).

---

## 6. Recipe for a deterministic port of this stage

1. Compute the constants of §1.2 with the exact integer truncations, in the flavour's own unit (B: 10 nm
   grid; Orca: 1 nm grid). Hard-code the speeds instead of calling tan, to avoid S7 in the settings:
   - B: V_f = 27 999 and V_s = 13 056 units;
   - Orca U1: V_f = 234 947 nm and V_s = 130 566 nm.
2. Implement Clipper-1-compatible offsets and booleans on the flavour's grid. Include:
   - the 0.005·|d| input decimation;
   - the miter-limit floor of 2;
   - the arc-tolerance rule;
   - 32-bit coordinates for B; 64-bit for Orca, whose orientation tests use double cross products and so
     are not exact for very large coordinate products. Exact int128 predicates can differ there in
     degenerate cases.

   An i_overlay/geo-based engine is fine for envelopes, but it will not reproduce upstream vertex for
   vertex.
3. Build the volumes lazily, keyed by (K, l), with the flavour switches of §2.10.
4. Run §3–§5 single-threaded. Use ordered containers only (`Vec`, `BTreeMap`). Never iterate a hash map.
5. Run the ensemble: the outer loop is over the tip orders from part 01. The inner loop is over the T-classes
   (§4.7), for B, O24 and S24 only.
6. Hand the node lists to part 03.

**Expected fidelity.** Exact up to S7 (±1 unit per vertex, occasionally a flipped ε_A test) and up to
Clipper-port differences. The envelope margin ε from the research (0.1–0.2 mm) is meant to absorb these.

---

## 7. Open questions and validation items

1. **TBB concurrency in the GUIs.** I found no global thread limit in the CLI entry points or in Print.
   Whether the B and S24 GUIs ever cap TBB (preferences, energy mode) is unverified. Check: slice the P1S
   probe in the GUI and on the CLI on the same Mac and compare L1/bands.
2. **oneTBB and cgroups.** Does the server's TBB build read cgroup CPU quotas when it sizes the default arena?
   If it does, docker `--cpus 2` would mean T = 2, a different S2 class from the owner's GUIs. The critic
   (C4) assumed host cores. This needs a one-line probe binary or a log of `max_concurrency()` on the runner.
3. **Feet clearance from key rounding: partly confirmed.** In the existing single-object P1S G-code (§5.1)
   the two isolated full feet sit 5.97–6.04 mm from the body, against 6.06 mm predicted. Still open:
   - do the 4.4–4.5 mm trunks belong to the min-xy class (≥ 4.11 mm) or to partially grown feet?
   - how far does smoothing (part 03) move nodes in general? Within this sample, ≤ 0.1 mm at the foot.

   A port-vs-G-code comparison per node would settle both.
4. **The mfs bug.** If a user sets `min_feature_size` to 0 %, s drops to 0.75 / 0.21 mm. The wall
   restriction then applies to every step, and "offset independent faster" can become true. The general SOI
   rule in §3.4 covers this, but it has not been exercised. Pin the setting or flag it in the packer.
5. **B's bed ring centroid** uses B_obj's area centroid. For the P1S L-shaped bed (with the corner exclusion)
   the centroid is at about (128.9, 128.9) instead of (128, 128), so the slits sit about 0.9 mm off the bed
   centre. Harmless, but a port must use the same centroid definition.
6. **O25 for U1.** O25 puts U1 into the T = 1 class and simplifies collisions. That differs from the owner's
   S24 GUI in two independent ways. Matching the GUI argues for O24 on ≥ 14 cores, or for a port that runs
   the S24 flavour with the bucket-of-2 class, not O25.
7. **S1 coupling.** Whether the multi-core realizations seen in practice (8 distinct in 20 P1S runs) are
   fully explained by tip-order permutations × one T-class is for part 01 to show. This stage contributes no
   other run-to-run variation on a fixed machine.

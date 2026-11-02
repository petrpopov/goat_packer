# Organic tree supports, part 3: smoothing, drawing, first layer, brims, bed clipping, G-code roles

Date: 2026-11-02. Status: functional spec for the slicer-free envelope predictor. It describes what the
slicers do. It does not prescribe our implementation.

This part covers the **back half** of the Organic pipeline. It takes the placed tree nodes from the
middle stage and turns them into printed support layers. Earlier parts of this spec set cover the front
half: overhang detection, tip and roof placement, influence areas, merging and node placement.

## 0. Conventions

**Versions and citation prefixes.** Paths are relative to `src/libslic3r/` of each checkout in
`/tmp/goat_src`.

| prefix | checkout | who uses it |
|---|---|---|
| B | `bambu_0208` (BambuStudio tag v02.08.02.61) | owner's P1S and A2L GUI, server P1S/A2L |
| O | `orca_242` (OrcaSlicer v2.4.2) | server U1 |
| R | `orca_250rc` (OrcaSlicer v2.5.0-rc) | local `/Applications/OrcaSlicer.app` |
| S | `snaporca240` (Snapmaker Orca v2.4.0) | owner's U1 GUI |

When a statement holds for all four, the citation is given for B and O only. Every difference is marked
**[DIFF]**. Every source of run-to-run variation is marked **[NONDET]**. The tags B/O/R/S are the same in
every part of this spec set; R alone is the flavour, R(e, f) with arguments is the radius function (§1.1).
Operations carry our names (glossary 02 §1.4); upstream names appear only in citations.

**Licensing.** All four slicers are AGPL-3.0. This document describes behaviour in our own words: prose,
formulas, constants, order properties and edge cases. Where upstream iterates (smoothing, §2.4), the spec
gives the update equations and the stop rule, not the loop.
Nothing here is a translation of upstream code. The `file:line` references exist only so that an
implementer can check the behaviour.

**Notation.**

- `⊕d` / `⊖d`: grow / shrink a polygon set by distance d.
- `∖`: set difference.
- `L_l`: the object's own slice (islands, "lslices") at layer l.
- `z_l`: print height of layer l.
- `w`: support line width (0.42). `w0`: first-layer line width (0.5).
- `s`: flow spacing, `s = w − h·(1 − π/4)`.
- All lengths are in mm.

**Goat-project values used in the examples** (from `/tmp/goat_research/cfg/*/Metadata/project_settings.config`):

| | P1S (B) | A2L (B) | U1 (O / S / R) |
|---|---|---|---|
| first layer height h0, layer height h | 0.20, 0.28 | 0.20, 0.28 | 0.25, 0.28 |
| z grid `z_l = h0 + l·h` | 0.20, 0.48, 0.76 … | same | 0.25, 0.53, 0.81 … |
| support line width w; first-layer width w0 | 0.42; 0.50 | 0.42; 0.50 | 0.42; 0.50 |
| external perimeter width | 0.42 | 0.42 | 0.42 |
| `support_object_xy_distance` / `support_object_first_layer_gap` g | 0.75 / **1.2** | 0.75 / 0.75 | 0.75 / 0.75 |
| top / bottom z distance → z-gap layers | 0.28 → 1 | 0.24 → 1 | 0.24 → 1 |
| `raft_first_layer_expansion` | −1 (auto) | −1 (auto) | 2 |
| `raft_first_layer_density` | 90 % | 90 % | 90 % |
| `support_interface_top_layers` / bottom | 3 / 3 | 3 / 3 | 3 / 3 |
| `support_base_pattern` | rectilinear | rectilinear | rectilinear |
| `tree_support_wall_count` | 0 | −1 | 0 |
| `brim_type` / `brim_width` / `brim_object_gap` | auto / 5 / 0.1 | same | same |
| `elefant_foot_compensation` | 0.15 | 0.15 | 0.15 |
| arc fitting | on | **off** | on |
| `resolution` | 0.03 | 0.03 | 0.03 |

## 1. What this stage receives

Each tree node (a "support element") arrives with:

- a layer index `l`;
- an XY position `p` (set by node placement);
- a radius state, from which the radius is derived (§1.1);
- a list of parent nodes on layer l+1, toward the tips;
- implicitly, at most one child node on layer l−1, toward the bed.

Nodes are stored per layer in a list whose order comes from the middle stage. **[NONDET]** That order,
and the topology itself, inherit the upstream sources S1 (tip insertion race) and S2 (merge bucket size
from the core count). This back half adds no new thread-dependent geometry (§8). It also never changes
topology, except that Bambu prunes disconnected fragments (§3.6).

Other inputs:

- **Roof polygons** placed by the interface placer of the front stage. They are already sorted into
  "top contact" (the layer just below the z gap) and "interface" (1…N−1 layers further down), per layer.
- **The object's own collision outlines** per layer (§2.2).
- **The bed polygon** in the frame of the object that computes the supports (§6).

### 1.1 Node radius (used by both smoothing and drawing)

`r(node) = R(e, f)`, where `e` is the node's effective distance-to-top in layers and `f` is its
"foot increase" count:

- tip zone (`e ≤ T`): `R = r_tip + (r_branch − r_tip)·e/T`;
- trunk zone (`e > T`): `R = r_branch + (e − T)·Δr`;
- plus a foot term in both zones: `+ f·max(0, Δr_foot − Δr)`.

Constants:

| symbol | definition | goat value |
|---|---|---|
| r_tip | tip diameter / 2 | 0.4 (B hard-codes 0.8 mm tip diameter) |
| r_branch | branch diameter / 2 | 1.0 |
| T | integer `max((r_branch − r_tip)/(w/3), r_branch/h)` | 4 layers |
| Δr | `tan(branch_diameter_angle)·h` | tan 5°·0.28 = 0.0245 per layer |
| Δr_foot | `min(tan(0.7 rad)·h, w/2)` | 0.21 per layer |
| f | grows by at most 1 per layer near the bed, only while the result stays ≤ `r_rec(l) = r_branch + (l_bp − l)·Δr_foot` and the wider branch still fits | — |
| l_bp | `(r_bp − r_branch)/Δr_foot` | ≈ 13 layers |
| r_bp | half of a hard-coded 7.5 mm "initial layer diameter" | 3.75 |

`f` can be fractional after merges. Sources: B `TreeSupportCommon.hpp:255-296`, `:541-560`,
`TreeSupport3D.hpp:235-247`, `TreeSupport3D.cpp:1559-1572` (foot increase), `:1940-1946` (merge).

**Bound used later:** the radius at layer 0 is at most `max(3.75, trunk radius from e)`. A 25 mm tall
trunk has about 3.08.

## 2. Organic smoothing ("avoid collisions" pass)

B `TreeSupport3D.cpp:3628-3836`, O `:3030-3291`, R `:3074-3335`, S `:3016-3228`. The pass is called
from the drawing routine before any geometry is generated (B `:4754`, O `:3734`). All four builds use the
same "new" line-based variant (`TREE_SUPPORT_ORGANIC_NUDGE_NEW` is defined); the older OpenVDB variant is
compiled out.

### 2.1 Graph

Every node becomes a **sphere** with:

- centre = (node XY, `z_l` of its layer);
- radius = `r(node)`;
- links: its parents (layer l+1) and its child (layer l−1, or none).

**Locked spheres never move:**

- tips (no parents);
- nodes above layer 0 that have no child (lost branches whose lower part was removed).

**Root nodes on layer 0 are not locked:** they have parents but no child, and layer 0 is excluded from
the lock rule.

### 2.2 The obstacle the spheres are pushed away from

For each layer, the obstacle is the boundary of the object's **radius-0 collision area**:
`C_l = (L_{l−1} ∪ L_l) ⊕ xy_min ∪ L_{l+1} ⊕ xy_min/2`, with:

- `xy_min = min(xy, ½·external perimeter width)` = 0.21 mm (goats);
- the `xy_min/2` term = the single top-z-gap layer;
- one bottom-z-gap layer below (`L_{l−1}`);
- miter offsets with requested limit 1.2, which the offset library raises to 2 (02 §1.1): corners with
  an interior angle between 60° and ≈ 113° are mitred, not squared.

Sources: B `TreeModelVolumes.cpp:452-560`; the min-xy collision lookup `:313`; the radius-key rule (our
`radius_key`, 02 §2.2) `TreeModelVolumes.hpp:148-154`.

Note that it uses `xy_min`, not the 0.75 mm `support_object_xy_distance`. The full xy distance only acts
through the influence areas of the middle stage.

**[DIFF] Bambu adds the bed exterior to every layer's obstacle** (B `TreeModelVolumes.cpp:67-88`, `:131-134`):

- The exterior is the bed-plus-1000-mm ring minus the bed polygon (bed = printable area minus
  `bed_exclude_area`, in the computing object's frame, §6).
- It is cut into four axis-aligned pieces by two **2 mm-wide straight corridors** through the centroid
  of the bed polygon: one vertical and one horizontal (`ExPolygon::split_expoly_with_holes`, B
  `ExPolygon.cpp:382-413`, gap 1 mm each side).
- So in Bambu, branches are kept ≥ xy_min + radius away from the bed edge and from the cutter exclusion,
  **except inside the two corridors**.
- **Measured:** in `/tmp/goat_research/exp/outA` (P1S canonical goat at the right edge), the support
  sheath reaches x = 255.80 (the bed clip) only in the band G-code y 126–128 (plate y 128–130, the
  corridor at plate y 127–129 plus the branch radius). Elsewhere along the same edge it stays ≤ 255.5.

Orca 2.4.2, Orca 2.5-rc and Snapmaker 2.4.0 add no exterior obstacle. Their machine-border function
returns nothing (O `TreeModelVolumes.cpp:39-49`) and the bed is only a clip (§3.4, §6).

### 2.3 Z span checked for each sphere

A sphere only tests the layers its own tree occupies near it:

- `zmin` = the larger of (the z of its tree's bottom node, reached by following children down) and
  `z − r`;
- `zmax` = the smaller of (the lowest tip z among all its ancestors, found by propagating the minimum
  down from the tips) and `z + r`;
- the layers tested run from `min(own layer, first layer ≥ zmin)` to `max(own layer, last layer ≤ zmax)`.

Layer spacing in this test uses the nominal h, also for layer 0. The cross-section radius on layer k is
`ρ_k = √(r² − ((k − l)·h)²)`; layers with `ρ_k² ≤ 0` are skipped.

### 2.4 Update rule (Jacobi style)

Let x_i⁽ⁿ⁾ be the XY of sphere i after iteration n, with x⁽⁰⁾ the placed node positions. Locked spheres keep
x⁽⁰⁾. For every unlocked sphere i, iteration n ≥ 1 computes, from x = x_i⁽ⁿ⁻¹⁾ and the previous positions p_j =
x_j⁽ⁿ⁻¹⁾ of its neighbours:

```text
collision depth   D  = max over the tested layers k (§2.3) of  ρ_k − |x − q_k|,
                       q_k = the point of layer k's obstacle boundary (§2.2) nearest to x, within ρ_k
push              y  = x + a² · (x − q)/|x − q|,   a = min(D + 0.1, 0.5),   if D > 0 (q = q_k of the max)
                  y  = x                                                     otherwise
target            t  = mean(p_parents)                              (no child)
                  t  = ½·mean(p_parents) + ½·p_child                (with a child)
smoothing         x_i⁽ⁿ⁾ = y + clamp_len(½·(t − y), 0.2)
```

`clamp_len(v, c)` shortens v to length c if it is longer. **[DIFF]** In O and R, y and x_i⁽ⁿ⁾ are each passed
through the anchor limit below. **Stop rule:** the last iteration is the first n ≥ 1 in which no sphere has
D > 1e-4 mm, or n = 100. The final XY is written back rounded to the flavour grid.

Constants: extra gap 0.1, push cap 0.5, smoothing cap 0.2, factor 0.5, at most 100 iterations
(B `:3749-3752`, `:3811`; O `:3152-3155`, `:3262`).

Properties that matter for a faithful reproduction:

- **The push is the square of the clamped value.** Upstream scales the unit direction by `a` twice, so
  the actual displacement is `a²`: between 0.01 mm (shallow touch) and 0.25 mm (depth ≥ 0.4) per
  iteration, not `a`. B `:3790-3793`; O builds the candidate the same way, `:3239-3241`.
- **Weights:** each parent is weighted by the sphere's **own** radius, so all parents weigh the same. The
  child weighs as much as all parents together. Hence the ½/½ split above (B `:3797-3814`).
- **Inside or outside is not checked.** Only the distance to the obstacle boundary matters. A centre that
  is inside the collision area and nearer than ρ to its boundary is pushed *further in* (the direction is
  centre minus boundary point). A centre deeper than ρ inside is not detected at all. Node placement
  normally keeps centres outside, so this is an edge case.
- The push uses the sphere's own previous position; the target uses the neighbours' previous positions;
  the smoothing step starts from the pushed position y. A sphere that does not collide still gets the
  smoothing step.
- **Termination:** at least one iteration always runs, so every unlocked sphere gets ≥ 1 smoothing step.
  If collisions persist, all 100 iterations run.

**[DIFF] Orca 2.4.2 and 2.5-rc anchor limit** (O `:3157-3196`, R `:3201-3245`; absent in B and S). The
push candidate and the smoothing candidate are each limited, in sequence:

1. against the **child's prev position** with allowed distance
   `max(0, r_child − r_self) + v_slow`, where `v_slow = tan(25°)·h` = 0.131;
2. then against **each parent's prev position** with allowed distance `max(0, r_self − r_parent) + v_slow`.

Each limit is a projection onto a disc around the anchor whose radius is the larger of the allowed
distance and the sphere's current distance to that anchor. So it never pulls a sphere in; it only stops
it from moving further away. This keeps every link's horizontal offset at about one slow step per layer
plus the radius change, so smoothing cannot steepen a branch beyond that.

### 2.5 Determinism

- Smoothing reads only `prev` positions of other spheres and writes only its own. The stop test depends
  only on the count. The result is therefore independent of thread scheduling.
- It is deterministic given the input nodes and their order.
- Remaining variation: float rounding (positions are single precision), libm (S7), and the input's
  upstream variation (S1/S2).

### 2.6 How far smoothing can move a node outside its influence area

Smoothing ignores influence areas completely.

**Bound A, convex hull plus pushes:**

- Let `H` be the XY convex hull of all nodes of the connected tree before smoothing.
- A Laplacian step moves a sphere along the segment towards a convex combination of points that are
  already in the hull, so it stays in `H`. The Orca projection also stays on the segment towards an
  anchor that is in `H`.
- Only pushes can leave `H`, and they point away from the own object.
- So the final XY of every node lies in `H ⊕ P`, where `P` ≤ 0.25 mm × (number of iterations in which
  some sphere still collided). The worst case is 25 mm. In practice it is far smaller, but no measured
  bound exists yet (open question Q1).

**Bound B, per iteration and node:** the displacement is at most 0.25 + 0.2 = 0.45 mm per iteration, so
≤ 45 mm in total for B/S. For O/R a node additionally stays within
`max(current distance, radius difference + 0.131)` of each linked neighbour's previous position, per
iteration.

**What the predictor needs.** Bound A says smoothing pulls trees *inward* toward their own hull. It only
pushes them outward near the own body (overhang undersides, body bulges above a trunk), and never toward
a neighbour more than the push total.

- A predictor that **reproduces node placement exactly should also run this pass.** It is
  deterministic, cheap (O(nodes × iterations)) and needs only the radius-0 collision outlines.
- A predictor that **does not** run it must widen its node discs by an empirical smoothing margin σ.
  For tips the margin is 0 (they are locked).

## 3. Drawing branches into per-layer polygons

B `organic_draw_branches` `TreeSupport3D.cpp:4692-5229`, O `:3673-4178`, R `:3769-4313`,
S `:3596-4009`.

### 3.1 Trees and branches

1. Clear all marks.
2. Scan the layers bottom-up, and the elements of each layer in list order.
3. Every unmarked element that has parents starts a new **tree**.
4. From that element, walk up recursively. At each node with k parents, start k **branches**. Each
   branch is the chain of nodes from that node upwards while every node has exactly one parent. The
   chain ends at a tip (no parents: a *tip branch*) or at the next bifurcation node, which is included and
   from which the walk continues.
5. The first branch of a tree is the **root branch**.
6. A bifurcation node is the last node of the incoming branch and the first node of every outgoing
   branch.
7. Elements with neither parents nor a child (isolated single nodes) belong to no tree and are **never
   drawn**.

**[NONDET]** The order of trees, and the order of branches inside a tree, follow the per-layer element
order, which is S1/S2-dependent. Unions make the final geometry order-independent, except for Bambu's
keep-main pass (§3.6). That pass is per tree, so it depends on topology, not on order.

### 3.2 Tube geometry of one branch

For a branch with nodes `n_0 … n_m`, the positions are 3D: (smoothed XY, `z_layer`).

- **Section circles:**
  - at each interior node: radius = the node radius, plane normal = the normalised sum of the incoming
    and outgoing unit segment directions (bisector);
  - at the two ends: the normal is the adjacent segment's direction.
- **Discretisation:** n segments with `n = ⌈2π / (2·arccos(1 − 0.015/r))⌉`. The vertices lie *on* the
  circle, so the polygon is inscribed: inside the true circle by at most 0.015 mm (chord tolerance
  0.015, B `:3442`, O `:2928`). The first vertex lies along the in-plane axis `normal × (0,−1,0)`, which
  is +X for a vertical normal. Counts:

| r (mm) | 0.4 | 0.6 | 1.0 | 1.5 | 2.0 | 2.5 | 3.0 | 3.5 | 3.75 | 4.0 |
|---|---|---|---|---|---|---|---|---|---|---|
| n | 12 | 15 | 19 | 23 | 26 | 29 | 32 | 34 | 36 | 37 |

- **Surface:** consecutive circles are joined by a zig-zag strip that always takes the shorter diagonal.
- **Caps:** both ends get a hemisphere of the end node's radius, built from `⌈π/(2·step)⌉` rings and
  closed with a fan. The bottom cap points along the reverse of the first segment, the top cap along the
  last segment.
- **[DIFF] Flat foot (O, R; not S 2.4.0, not B):** when a root branch starts on layer 0, the bottom
  hemisphere is replaced by a vertical cylinder of the root radius from z = 0 to `z_0`, with a flat
  bottom disc (O `:2941-2957`, R `:2985-3001`). Snapmaker's main branch has the same change
  ("Orca #14092"); its 2.4.0 tag does not.
- **[DIFF] Self-folding split (B only)** (B `:3521-3624`). Bambu tests every consecutive pair of section
  discs for a true 3D intersection (both discs cross the other's plane and their chords on the common
  line overlap; near-parallel normals, |sin| < 1e-4, count as safe). At every unsafe pair it cuts the
  path. Maximal safe runs become separate tubes; each unsafe edge becomes its own two-node capsule with
  hemispherical caps. Overlaps are resolved by the non-zero fill rule when slicing. O/R/S emit one tube
  per branch, so a self-folding strip can lose slice area there. That only removes area: an envelope
  that includes the full capsule is unaffected.

**Containment:** each piece of tube lies inside the union of the balls swept along each segment with
radius `max(r_a, r_b)`, i.e. a capsule. Every section circle lies in its node's ball, and the strip
between two circles lies in their convex hull. This gives the sound bound in §3.9.

### 3.3 Slice heights and layer range

- Layer `l` is sliced at the **mid-height** `ζ_l = ½(z_{l−1} + z_l)`, with `z_{−1} = 0`. So
  `ζ_0 = h0/2` (0.10 for P1S/A2L, 0.125 for U1) and `ζ_l = z_l − 0.14` above (B `:4919-4923`).
- Fill rule: positive / non-zero, so overlapping parts of one branch mesh merge.
- First layer sliced:
  - root branch: its first node's layer (nothing below layer 0);
  - other branches: the lower of the first node's layer and the first layer whose `z_l ≥` the mesh's
    lowest point, so the bottom cap at a bifurcation is sliced downwards.
- Last layer sliced:
  - tip branch: the tip's layer (**the tip's upper hemisphere is never sliced**);
  - other branches: the higher of the last node's layer and the last layer whose `z_l ≤` the mesh's top.

**Consequence for a predictor.** The polygon on layer l is centred where the branch axis crosses `ζ_l`,
i.e. **between** nodes l−1 and l, not at node l. For a straight segment tilted by φ from vertical, the
section is close to an ellipse with semi-axes `r` (across the tilt) and `r / cos φ` (along the tilt),
with `r ≈ ½(r_{l−1} + r_l)`.

Typical tilts:

| speed | per-layer move | tilt φ | elongation 1/cos φ |
|---|---|---|---|
| Bambu fast (45°) | 0.28 | 45° | 1.41 |
| Orca organic (40°) | 0.235 | 40° | 1.31 |
| slow (25°) | 0.131 | 25° | 1.10 |
| error case (1.5 × fast) | — | up to 56° | 1.80 |

Smoothing changes these tilts. Orca's anchor limit caps the tilt again; B/S do not.

Layer 0 of a root branch:

- B/S: the slice at ζ_0 cuts the bottom hemisphere just h0/2 below the root centre, giving a disc of
  radius ≈ `√(r_0² − (h0/2)²)` (≈ r_0 − 0.002 for r_0 = 3.75), plus the start of the tube. All of it
  lies inside `disc(root XY, r_0)`.
- O/R: exactly the flat-foot cylinder, `disc(root XY, r_0)`.

### 3.4 Trims applied to every branch slice

For each sliced layer l of a branch, in this order:

1. **Subtract the own object:** subtract the radius-0 collision area `C_l` from §2.2
   (`(L_{l−1} ∪ L_l) ⊕ 0.21 ∪ L_{l+1} ⊕ 0.105` for the goats). In Bambu `C_l` also contains the bed
   exterior pieces ⊕ 0.21 (§2.2).
2. **Intersect with the bed polygon** of the computing object (§6).
3. **[DIFF] Orca only (O, R):**
   - both operations use a 10 nm "safety offset";
   - slivers below 1e-6 mm² are dropped after each slice;
   - a branch whose slices are all empty is skipped.

Sources: B `:4929-4930`; S `:3806-3807` (same as B); O `:3891-3895`; R `:3987-3990`.

**Measured consequence:** on layers ≥ 1 the support may come as close as **0.21 mm** (material to
material) to its **own** body, not 0.75. Minimum support-to-body centreline gaps in the single-object
probes (`/tmp/goat_research/l0_gap2.py`) are:

| | layer 2 | layer 3 |
|---|---|---|
| P1S | 0.695 | 0.640 |
| A2L | 0.812 | 0.637 |
| U1 | 0.853 | 0.818 |

0.64 ≈ 0.21 (half body line) + 0.21 + 0.21 (half support line). This matters only for the own object.
Neighbours are invisible to the generator.

### 3.5 Roots, bottom contacts, lost branches

**Bed roots** (root branch starting on layer 0, marked "gracious to plate"):

- With bottom interface layers > 0 (all goat projects: 3), a bottom-contact polygon is formed as
  root slice ∩ "placeable area of layer 0".
- With `support_on_build_plate_only` no placeable areas are computed, and the lookup falls back to the
  radius-0 collision area, which the slice was just trimmed against. **So the intersection is empty (Bambu, Snapmaker) or a sub-micron sliver (Orca's
  safety offsets).** Bed roots produce **no bottom-contact geometry**.
- **Measured:** layer 0 of all three probes carries only role *Support* at width 0.5 and no
  *Support interface* (`/tmp/goat_research/role_width.py`).

**Roots above layer 0 that do not rest gracefully** ("lost" branches whose child chain was deleted
upstream): the root's first slice is dropped straight down layer by layer. Each step subtracts that
layer's collision area. The drop stops when the remaining area falls below
`max(0.2·π·r_root², 0.5·π·r_branch²)`. Every dropped layer becomes base area: a vertical column with no
widening.

- **[DIFF]** How far it may drop:
  - Snapmaker 2.4.0, Orca 2.4.2 and 2.5-rc: at most `5·r_root/h` layers, unless the branch is flagged
    "very lost", which drops it to layer 0 (S `:3830`, O `:3949`, R `:4058`);
  - **Bambu:** drops all the way to the model or the bed, with no layer budget ("nofloat", B
    `:4955-4975`).
- Orca also keeps the lowest dropped slice as a bottom contact and has extra fallbacks (O `:3986-4019`).
  These only re-label area.

**[DIFF] Orca roof recovery:** branch slices are turned back into roofs for tips that carry a
pending roof (O `:3646-3671`). It is inactive for these projects: it needs "force tip to roof", which
needs `π·r_tip² > 1 mm²` (0.50 < 1) or a zero-gap interface.

### 3.6 Per-tree merge, and Bambu's keep-main pruning

All branch slices of a tree are collected per layer and unioned where more than one branch contributes.

**[DIFF] Bambu only, keep-main** (B `:5071-5160`; not in O/R/S). For each tree:

1. Split each layer's union into connected parts.
2. Mark a part as kept if any node of that tree on that layer lies inside it, or within 0.025 mm of it.
3. Repeat to a fixpoint: an upward and then a downward sweep that also keeps every part overlapping a
   kept part on the adjacent layer.
4. Drop all unkept parts. Intersect the tree's bottom contacts with what is kept.

Pruning only removes area, mostly fragments cut off from the axis by the own-object trim. An envelope
that ignores it stays sound.

### 3.7 Global merge and clean-up per layer

1. Gather all trees' slices per layer and union them.
2. **Notch filling** (our `fill_notches`; upstream `smooth_outward`) with clip distance w (0.42): fills
   sharp concave notches and cracks whose mouth is narrower than w. It only adds area, and never outside the polygon's local convex hull (Cura-derived
   routine, `MutablePolygon.cpp:257-330` in the `zones_bambu` checkout).
3. **Simplify** with tolerance `min(0.03, resolution)` = 0.03 mm (Douglas-Peucker, then a self-union, 02
   §1.1 S(τ)). **[DIFF]** Only B's self-union is strictly simple (its two-argument simplify uses the
   library's strictly-simple mode, B `:5201`, `Polygon.cpp:658-675`). O and S pass the constant `false`
   (O `:4151`, `TreeSupportCommon.hpp:591`; S `:3981`), and R uses a plain union (R `Polygon.cpp:685-700`).
   The difference is vertex-level only (touching vertices split or not); the area is the same.
4. **Roles:**
   - subtract this layer's **top-contact** polygons (roofs) from base and from bottom contacts;
   - subtract bottom contacts from base;
   - store the remainder as the layer's **base** ("intermediate") polygons.

Sources: B `:5190-5226`, O `:4139-4176`.

### 3.8 Roofs and interfaces (top/bottom interface, roof areas, roles)

**Roof polygons come from the front stage** (B `sample_overhang_area` `:976-1105`; insertion `:1255-1266`).
With interface layers enabled (3 here), every overhang area of at least `minimum_roof_area` = 1 mm² is
placed as a roof:

- The overhang polygon, minus the forbidden area (radius-r_tip avoidance to the bed ⊕ 5 µm), goes on the
  insertion layer (overhang layer − z-gap layers − 1) as **top contact** (dtt = 0).
- It is then copied downward for dtt = 1, 2 as **interface**. Each copy is reduced by the next layer's
  forbidden area. Stacking stops early when the next copy would fall below 1 mm².
- Roof polygons are **not** trimmed by the branch slices and are not clipped by the tube trims of §3.4.
  They are therefore support area **in addition to** the tubes.
- They lie under the own overhang. Per the front stage, the overhang polygon can exceed the model's own
  outline by up to 0.8 mm (enforcer offset) and by 0.2 mm (sharp-tail offset).
- **[NONDET]** The interface placer appends polygons from parallel tasks under a mutex, so their order
  varies between runs. The set is identical, and all of them are unioned later.

**Interface re-labelling** (upstream `generate_interface_layers`, B `SupportCommon.cpp:59-297`, O `:59-242`)
then takes, for every base
layer:

- the top-contact polygons that lie within the next N = 3 layers above it, and the bottom-contact
  polygons within N layers below it;
- closes them by `1.5·s_if + s_if/density` and smooths them;
- **intersects that with the base layer**;
- unions it with the precomputed interface polygons of the same layer;
- subtracts the result from the base.

So this step only **re-labels** base area as interface. It adds no area beyond the precomputed roofs.

**[DIFF] Bambu** (B `TreeSupport3D.cpp:4585-4610`, `SupportCommon.cpp:286-294`):

- When a support has roofs but no base at all, roofs pass through unchanged.
- When the bed layer is an interface with ≥ 3 stacked roof heights above it, the bed layer is demoted to
  base.

Neither case occurs for the goats (they have trunks).

**Role summary per layer:**

| role (G-code label) | polygons |
|---|---|
| Support (base) | tube slices ∖ contacts ∖ interfaces |
| Support interface | top contacts, interfaces (precomputed roofs and re-labelled base), bottom contacts |
| Support transition | not produced here; see §7 |

**Union of all roles** = tube slices ∪ precomputed roof polygons (∪ dropped columns). This union is
what the predictor needs.

### 3.9 Per-layer envelope models for a predictor

**Tight model** (reproduces the slicer up to discretisation). For layer l:

```
T_l = ∪ over branches, for each axis segment crossing ζ_l:
        ellipse(centre = axis point at ζ_l, minor = r(ζ_l), major = r(ζ_l)/cos φ, major along the tilt)
    ∪ end and bifurcation caps whose ball reaches ζ_l
    ∪ root disc(root XY, r_0)                       (layer 0 only)
S_l = ((T_l ∖ C_l) ∩ Bed) ∪ Roof_l                  # fill_notches/simplify add ≤ about w in notches
```

**Sound model** (contains every drawn tube for the given smoothed nodes):

```
T̂_l = ∪ over all segments (a, b) of the tree whose capsule of radius R = max(r_a, r_b)
        reaches z = ζ_l, of the capsule's horizontal section at ζ_l
```

The capsule section is the union of the discs of radius `√(R² − (z − z_c)²)` around the axis points
`c` within R of ζ_l. Because the polygons are inscribed, `T_l ⊆ T̂_l`.

## 4. First layer

### 4.1 What goes into layer 0

After drawing, layer 0 holds:

- the root slices of every bed root (§3.3), already trimmed against `C_0` and clipped to the bed;
- possibly a top contact or interface at layer 0, when a roof stack reaches the bed. Not the case for
  goats.

Then the first-layer base routine (upstream `generate_raft_base`) runs with `raft_layers = 0` (B
`SupportCommon.cpp:299-469`, O `:244-414`,
R `:404-575`, S `:315-480`).

### 4.2 Expansion of the base flange

Let `F0` = the base polygons of support layer 0.

**1. Expansion distance e** **[DIFF]**:

| build | rule | goat projects |
|---|---|---|
| Bambu | `raft_first_layer_expansion < 0` means **auto = 2 mm** (B `:404`, `:446-449`); otherwise the value | P1S/A2L: −1 → 2.0 |
| Orca 2.4.2, Orca 2.5-rc, Snapmaker | `e = max(0, value)`, so −1 means **no expansion** (O `:288`, S `:359`) | U1: value 2 → 2.0 |

A Bambu project with −1 sliced in Orca would therefore get no flange at all.

Both subtract a negligible ε (scaled 1e-4 mm) before use. Bambu expands only if
`e > ε + ε_grid`, Orca only if `e > ε_grid`.

**2. Trimming set** `X = L_0 ⊕ g`:

- the offset uses **square joins**;
- `L_0` is the object's own first-layer islands **before** elephant-foot compensation (B
  `PrintObjectSlice.cpp:1346-1363`, O `:1368-1386`): the slicer stores the uncompensated islands as
  layer 0's islands;
- `g = support_object_first_layer_gap` (P1S 1.2; A2L, U1 0.75; B `:441`, O `:382`).
- X is the slicer's own trim mask. It is **not** the packer's conservative `landing` kind (part 05, design
  §4.2), which is the section ⊕ 0.05 mm; using the landing here would under-predict the base.

**3. Step count and size:**

- `n = max(5, ⌈e / w0⌉)`; for e = 2, w0 = 0.5 this gives n = 5;
- step `δ = e/n` = 0.4 mm.

**4. Iterate** n times: `F ← (F ⊕ δ) ∖ X`.

- The ⊕ is a **miter** offset with limit 3 (library default). On the polygonised circles of tube
  sections this behaves like a round offset to within µm.
- The repeated trimming makes it a **geodesic dilation**: the flange grows 2 mm through free space but
  cannot jump across the gap-inflated own footprint. It can wrap around a narrow part of it only along
  paths that stay outside X.

**5.** If no expansion applies: `F ← F0 ∖ X`.

**6.** Subtract the (ε-inflated) layer-0 contact and interface polygons from F. Those print as interface.

**7. Brim trim inside the raft-base routine:** F, the contacts and the interfaces are reduced by a "brim"
polygon set. For `brim_type = auto_brim` that set is just `L_0` itself (no `brim_object_gap` offset:
auto is neither "outer" nor "inner" here, B `:309-337`), which X already removed. **So it is a no-op for
the goat projects.** For explicit outer brims it would remove `L_0 ⊕ brim_object_gap`, also inside X
whenever g ≥ brim_object_gap.

**8. Clip to the bed** (§6) together with every other support layer, after this routine.

Pseudo-code:

```
X  = squareOffset(objectFirstLayerIslandsUncompensated, g)
F  = base polygons of support layer 0
if e > tiny:
    n = max(5, ceil(e / w0)); d = e / n
    repeat n times: F = grow(F, d) minus X
else:
    F = F minus X
F = F minus (layer-0 contacts and interfaces, grown by tiny)
F = F intersect BedPolygon_of_computing_object
```

**Measured** (single-object probes, centreline to centreline, arcs sampled):

| | min support-to-body gap on layer 0 | model g + w0/2 + EFC 0.15 + w0/2 |
|---|---|---|
| P1S | 1.822 | 1.85 |
| A2L | 1.373 | 1.40 |
| U1 | 1.352 | 1.40 |

The residual −0.03…−0.05 mm comes from simplification and arc fitting.

Foot shape: research `feet.py` measured feet of radius 5.3–5.6 at z = 0.2 on P1S. That is
`r_0 (≈3.5–3.75) + 2.0` (polygon) minus 0.25 (centreline).

### 4.3 Support brims

**No version prints a brim for organic supports.**

- `tree_support_brim_width` and `tree_support_auto_brim` are read only by the non-organic tree generator
  (O `TreeSupport.cpp:2023`, `:2136`; S `TreeSupport.cpp:1981`, `:2092`).
- Bambu's non-organic generator instead maps `raft_first_layer_expansion = −1` to a moment-based width
  (B `TreeSupport.cpp:2481`).
- In the brim generator the "support brim" code is commented out in O/R/S (O `Brim.cpp:592-603`,
  `:606-630`). Bambu has no support-brim area at all (B `Brim.cpp:1029-1049`).
- Supports only act as **no-brim areas** for object brims (§5.4).
- The wide first-layer patch is the flange of §4.2, printed with role *Support*.

### 4.4 Toolpaths on layer 0

The flange polygon is printed with:

- the first-layer flow (`w0` = 0.5);
- a **sheath**: one loop along the polygon shrunk by `w0/2`, implemented as a closing of +ε then
  −(ε + w0/2);
- inside it, rectilinear fill at `raft_first_layer_density` (90 %) on the polygon shrunk by a further
  0.4·s0;
- fill angle `support_angle + 90°`.

Sources: B `SupportCommon.cpp:1821-1834`, `:799-820`; O `:1778-1790`, `:730-751`.

The outer material edge therefore coincides with the flange polygon. The centreline lies `w0/2` = 0.25
inside it. Measured at the bed edge: 255.75 of 256 (P1S), 270.25 of 270.5 (U1).

## 5. Object brims (`brim_type = auto_brim`)

### 5.1 When an object "has a brim"

The brim routine runs for an object when:

- `brim_type ≠ none` with `brim_width > 0`, **or** `brim_type = auto`, **or** painted or ear brims; and
- the object has no raft.

Sources: B `Print.hpp:425-430`, O `:362-366`.

For auto, the **width** decides whether anything is printed.

### 5.2 Auto width, per volume group

The first-layer slices are grouped by volume. Volumes whose first-layer slices touch (after a 0.05 mm
grow) form one group (O `PrintObjectSlice.cpp:517-590`). For each group the inputs are:

| input | meaning |
|---|---|
| H | z-extent of the group's volumes |
| Ixx, Iyy | second moments of area of the group's first-layer slices about their joint centroid (mm⁴) |
| bx, by | bbox of those slices |
| D | `√(bx² + by²)` |
| v | the object's maximum configured print speed: the largest of inner and outer wall, sparse, solid and top infill, support and small-perimeter speeds; 250 if none (B `Model.cpp:4305-4339`) |
| a | adhesion coefficient: 2 for PETG/PCTG, 0.5 for TPU, 1 otherwise (B `Brim.cpp:575-600`) |

Then:

```
k  = max(H·by/Ixx, H·bx/Iyy) · H / 1920
W  = a · min( max(k·v, Θ), 18, 1.5·D )
if W < 5 and W < 1.5·D: W = 0       # small brims are dropped
W  = min(W, 18)
```

- **[DIFF] Θ (thermal term):**
  - Bambu 2.8.2.61: **Θ = 0** (the term is multiplied by zero, B `Brim.cpp:798`);
  - Orca 2.4.2, 2.5-rc and Snapmaker 2.4.0: `Θ = 8·D/λ · min(H, 30)/30`, where λ is the material's
    "thermal length". λ is 200 unless a per-material value exists; a group uses the shortest of its
    volumes (O `Brim.cpp:298`, `Model.cpp:3258-3282`; S `Brim.cpp:799`).
- **Quantisation:** W is rounded down to an even number of brim line spacings: `⌊W/(2·s_b)⌋·2·s_b`,
  where `s_b` is the spacing of the brim flow (first-layer line width 0.5).
- **Per island cap:** the width is reduced to the island's own larger bbox side, then quantised again.
- **[DIFF] Extra brim for thin islands** (O `Brim.cpp:514-520`, R `:564-566`, S `:972-974`; disabled
  in Bambu by an always-false guard, B `:946`). Applies when all hold:
  - the group's W < 5 mm;
  - H > 10 mm;
  - the island's `area / contour length < 1.1 mm` (a narrow island);
  - the computed W is below one brim line spacing.

  Such an island gets `W + 5 mm` (quantised), capped by its bbox side.

**Goat numbers** (`/tmp/goat_research/l0_moment.py` on the probe first layers):

| | value |
|---|---|
| H | 44.7 |
| Ixx, Iyy | ≈ 9 300–10 100 and ≈ 13 700–13 900 mm⁴ |
| bbox | 26.7 × 25.6 |
| D | 37.0 |
| `k·v` | 0.53–0.86 mm for v = 200–300 mm/s |
| Θ (Orca/Snapmaker, λ = 200) | 1.48 |

- In every version W < 5, so W = 0. The margin to the 5 mm threshold:

  | material | Bambu | Orca / Snapmaker |
  |---|---|---|
  | PLA (a = 1) | ×5.8 | ×3.4 (Θ dominates) |
  | PETG (a = 2) | ×2.9 | ×1.7 |

- The goat's first layer is one island of ≈ 330 mm² with `area/perimeter` ≈ 3, so the extra-brim rule
  does not fire. The G-code shows four small extra loops; that they are separately walled colour regions
  of the same island, not islands, is **inferred**: they touch and overlap the main loop's extent.
- **Measured:** no *Brim* line in any probe, experiment or plate (all `exp/*`, `slices/*` and the
  17-goat Snapmaker GUI plate).

### 5.3 Brim area, gap and trimming (when a brim exists)

**Base ring.** For each island of the group:

```
ring = (contour ⊕ brim_object_gap  ⊕ W) ∖ (contour ⊕ brim_object_gap)
```

The offsets are round (`brim_object_gap` = 0.1). Islands inside a hole of their own object are clipped to
that hole (Bambu). Rings are computed in object coordinates, **translated per instance** into plate
coordinates, and kept per object id.

**Every ring is then reduced by the no-brim set N** (B `Brim.cpp:1011-1133`, O `:577-701`,
S `:913-1124`). N contains:

1. every object instance's first-layer islands ⊕ `brim_object_gap` (round). So a brim keeps 0.1 mm from
   any object's first layer, including foreign ones;
2. every object's **support layer-0 area**:
   - B: the union of all support polygons of support layer 0;
   - O/R/S: the area covered by the extruded support lines of layer 0 (width = spacing);
3. the prime tower's first-layer outer wall polygon, brim loops included (fake-tower outline at its
   position);
4. `bed_exclude_area`:
   - **[DIFF]** B reads it as rectangles of 4 points each;
   - O/S read it as one polygon (S uses the bed-minus-exclusion polygon);
5. outside the printable area: a 30 mm ring around the printable polygon. **[DIFF]** B uses each
   nozzle's printable area; O uses each extruder's printable area or the bed.

**[DIFF] Closing of N:**

- B and S close N by one brim spacing: grow by `s_b`, then shrink by `s_b`. This fills gaps narrower
  than `2·s_b` between no-brim regions, so no brim squeezes into them.
- O 2.4.2 and 2.5-rc have this commented out (O `Brim.cpp:674`).

**Connectivity rule:** after trimming, a brim piece survives only if, grown by `2·s_b`, it touches an
object island or another brim piece (any object). **Brims of different objects may overlap.**

**Loops:**

- the first loop is the brim area shrunk by `½·s_b`;
- each next loop is shrunk by a further `1.3·s_b`, then regrown by `0.3·s_b` (net −s_b);
- a ring narrower than `s_b` yields no loop;
- loops are chained and short gaps are linked (O `Brim.cpp:813-878`).

Printed with role *Brim* on layer 0 only, at the first-layer width.

## 6. Bed clipping and the shared-object rule

### 6.1 One computing object per identical group

During `Print::process` (B `Print.cpp:1957-2043`, O `:2192-2270`, R `:3314-…`, S `:2366-2441`), objects
are scanned in **model order**. An object becomes a copy ("shared object") of an earlier computing object
when all of these hold:

- the same 3×3 part of the transformation (rotation and scale; XY translation is excluded);
- the same number of volumes, each with the same type, the **same mesh pointer** and the same volume
  transform;
- the same extruder overrides;
- the same painted support, seam, fuzzy-skin and colour facets;
- the same per-volume and per-object config;
- the same layer-height ranges and profile.

Copies receive the computing object's **layers and support layers by reference**
(`copy_layers_from_shared_object`, B `Print.cpp:1853-1868`, O `:2093-…`, S `:2267-2278`), together with
its first-layer volume groups (used for the auto-brim width). Each copy prints them translated by its
own instance shift.

- The 3MF loader gives clones that reference the same component the same mesh pointer. The packer
  writes clones that way (`duplicate_mesh_per_object: false`).
- **If clones were written as separate meshes, every clone would run its own generator:** an
  independent S1/S2 draw, clipped in its own frame.

### 6.2 Where the bed enters the support generator

The **bed polygon** = printable area minus `bed_exclude_area`, keeping the first resulting polygon
(B `PrintConfig.cpp:10301-10328`; O `:11561-11571`, which subtracts the polygonal exclusion). It is
shifted by `(plate origin − first instance shift of the computing object)` into that object's frame
(B `TreeSupport.cpp:626-629`, O `:654-657`, S `:627-630`). The support generator uses it:

1. as a **clip** of every branch slice (§3.4);
2. as a **clip of every final support layer**: base, contacts, interfaces and the expanded flange,
   after the first-layer expansion (B `TreeSupport3D.cpp:4643-4646`, O `:3599-3602`, R `:3675-3678`,
   S `:3547-3550`);
3. **[DIFF] Bambu only:** as the source of the exterior **obstacle** with two 2 mm corridors (§2.2).
   It affects routing (avoidance), smoothing and the branch trim; it does not affect the flange
   expansion. Orca and Snapmaker only clip.
4. **[DIFF] R only:** additionally trims all support layers by `bed_exclude_volumes`, Z-limited
   tool-changer keep-outs (R `TreeSupport3D.cpp:3669-3671`). Not configured in any goat project.

**[NONDET] across plate layouts (S4).** Clipping and the Bambu obstacle are evaluated **once**, for the
computing object in its own frame:

- If the computing object stands near an edge, its supports are clipped or rerouted, and every copy
  prints that trimmed version. Measured: −4.4 mm (P1S), −5.5 mm (U1) on an interior copy.
- If it stands in the interior, copies near an edge print **unclipped** and can overhang the bed.
  Measured: x = 260.15 on a 256 bed (P1S, CLI −104); x = 275.76 on 270.5 (U1, CLI −102).

Evidence: `zones-research.md` §5.

Consequences for the packer:

1. Put the most interior placement of each identical group first in the 3MF.
2. Treat every copy's predicted envelope as unclipped.
3. Keep it inside the printable area by the packer's own constraint.

## 7. What lands in the G-code, per role

**Labels and widths:**

- Bambu writes `; FEATURE: <role>` and `; LINE_WIDTH: <w>`.
- Orca and Snapmaker write `;TYPE:<role>` and `;WIDTH:<w>`.

**Object attribution:** support lines are emitted inside their object's label block.

- Bambu: `; OBJECT_ID:` / start/stop printing object.
- Orca/Snapmaker: `; printing object … id:N copy M`, plus `EXCLUDE_OBJECT_START NAME=…` with Klipper.

So per-object attribution of supports works in every version.

| role | where (goat projects) | width | centreline relative to the role polygon P | material vs P |
|---|---|---|---|---|
| **Support**, layer 0 (flange) | base of layer 0 after §4.2 | w0 = 0.5 | outer loop on `P ⊖ 0.25`; 90 % rectilinear fill further inside | outer edge = P |
| **Support**, layers ≥ 1 | base polygons (tube slices minus roles) | 0.42 | outer loop on `P ⊖ 0.21` (sheath); **[DIFF]** inside: B and S print the trunk **hollow** (B adds solid fill only to "newly opened voids", lightning pass `TreeSupport3D.cpp:4165-4405`); O/R print rectilinear fill at the base spacing (2.5 + s) because these projects set `support_base_pattern = rectilinear`, not none/default (O `SupportCommon.cpp:1791-1800`); a second inner wall only for large areas or wall count > 1 | outer edge = P (inner walls never change it) |
| **Support interface** | top contacts, interfaces, (bottom contacts) | 0.42 (interface flow) | rectilinear lines at spacing `support_interface_spacing + s` (0.86, density 0.42) clipped to the polygon shrunk by ½·s (lines end ≈ 0.18 inside P; measured 0.18 at the U1 bed clip); linking moves may run along `P ⊖ 0.05·s`; if a region yields no fill line, one loop on `P ⊖ ½·s` | may exceed P by ≈ (w − s)/2 = 0.03, and by up to ≈ 0.19 on rare links |
| **Support transition** | **not produced for these projects.** B emits it only for "base interface" layers, which exist for trees only when the interface filament differs from the support filament (B `SupportParameters.hpp:35-45`, `SupportCommon.cpp:1789-1805`); O/S organic never emit it | 0.42 | as Support | as Support |
| **Brim** | none for goats (§5.2) | w0 = 0.5 | first loop on `area ⊖ ½·s_b`, then every s_b | edge ≈ area boundary + (w0 − s_b)/2 |

Other effects on the printed geometry:

- **Arcs:**
  - with arc fitting on (P1S, U1), support paths are refit to G2/G3 arcs within **0.0375 mm**
    (`SUPPORT_RESOLUTION`, B `libslic3r.h:68`, `Layer.cpp:536-569`);
  - otherwise (A2L) they are simplified with `resolution` 0.03;
  - comparisons with predicted polygons must sample arcs; `gparse.py` uses a 0.2 mm chord;
  - **measured:** a fitted arc overshot a bed-clipped polygon by 0.01 mm (outA).
- **Frame:** G-code XY = plate XY − `extruder_offset`. P1S (0, 2); U1 and A2L (0, 0).
- **Comparing a predicted envelope P with printed lines:** treat each line as its centreline ⊕ w/2 (the
  repo's `strip_polygon`). The expected material extent is:
  - P exactly, for Support and the flange (up to 0.04 from arcs and simplification);
  - P ⊕ 0.03 for interfaces (P ⊕ 0.2 as a hard bound).

  A printed line outside that is a real prediction miss.

## 8. Nondeterminism in this half

| id | source | where | effect | which realisations |
|---|---|---|---|---|
| (in) | input node positions, topology and list order | front and middle stages: S2 T-classes, S9 families, S1 (optional, unproven) | everything here inherits it | as many as upstream produces (B P1S: 16 line-level realizations in 7 L1 families in 53 observations, M §5.3; the source of most of them is unidentified, design §3.2 SU) |
| N1 | order of roof polygons appended under a mutex; order of brim pieces retained in parallel (B) | `InterfacePlacer::add_roof`, B `Brim.cpp:1154-1166` | vertex order only; the unioned sets are identical | 1 geometric realisation |
| N2 | smoothing in parallel | §2 | none: Jacobi update and a count-only stop test | 1 |
| N3 | per-tree drawing in parallel | §3 | none: per-tree work is independent and the merge loop is sequential | 1 |
| S4 | which copy computes; where it stands relative to the bed and exclusions | §6 | clip, plus (B) obstacle and corridor position in the copy's frame | one per layout of the canonical object |
| S7 | float (`Vec3f` meshes, `float` slice z) and libm `cos`/`acos` | §2.4, §3.2 | sub-µm vertex differences, platform-dependent | x86 vs arm64 may differ at the last bit; Clipper rounding can flip a degenerate case |

## 9. Quantities an envelope predictor must reproduce

**Band rule** (design §4.1; lead, 2026-11-02; one rule for supports, interfaces and bodies):

- the material of layer l occupies `z ∈ (z_{l−1}, z_l]` (z_{−1} = 0);
- band `b` covers `[2b, 2b + 2)`. Layer l belongs to **every** band its interval overlaps with positive
  length: z_{l−1} < 2b + 2 and z_l > 2b. A layer ending exactly on a band boundary belongs only to the lower
  band;
- layer 0 belongs to band 0 **and** to the first-layer kinds (`support_base`, `landing`, §9.1);
- goats: band 0 = layers 0-7, band 1 = layers 7-14, …; the boundary layer 7 is (1.88, 2.16] for P1S/A2L and
  (1.93, 2.21] for U1, and it is in both bands.

This replaces the repo's present `floor(z_l / 2)` binning (`src/gcode_collisions/learning.rs:24`, `:120`,
and the corpus' `derive.py`). That binning tests a support layer (1.88, 2.16] only against `body[1]` and
misses a foreign body top at z = 1.95. Corpus band polygons are re-binned with this rule before
calibration.

### 9.1 First layer (z ∈ (0, h0])

| # | quantity | rule / value |
|---|---|---|
| F1 | XY of every root node after smoothing | §2 (roots are not locked); per rotation class |
| F2 | radius r_0 of every root | §1.1; ≤ max(3.75, trunk radius) |
| F3 | root cross-section at ζ_0 | disc(F1, r_0): exact for O/R (flat foot), within 0.002 for B/S (hemisphere) |
| F4 | other tube parts reaching ζ_0 (low bifurcation caps, steep tubes) | §3.3; sound bound = capsule sections |
| F5 | own-object trim before expansion | `(L_0 ⊕ 0.21) ∪ (L_1 ⊕ 0.105)` (plus the B exterior obstacle) |
| F6 | flange expansion | e = 2.0, n = 5, δ = 0.4, geodesic in the free space `¬(L_0^uncomp ⊕□ g)`; g = 1.2 (P1S) or 0.75 (A2L, U1); square joins on the trim, miter joins on the growth |
| F7 | layer-0 roofs or interfaces | none for goats; would print as interface, untrimmed by F6 |
| F8 | bed clip | only the canonical copy, in its own frame; copies are unclipped |
| F9 | smoothing of the polygon outline | `fill_notches(w)` and simplify 0.03 (adds ≤ notches narrower than w) |
| F10 | printed material vs polygon | edge = polygon; centreline 0.25 inside; arcs ±0.0375 |
| F11 | object brim | none for goats; formula §5.2 if the model or settings change |
| F12 | support brim | never (organic) |
| F13 | uncertainty from upstream variation | δ_L1 from the calibration (design §5.5). Corpus: B plates up to 3.0 mm outside a 5-run single-object union; S GUI 0.7-4.1 mm outside the R union (M §2.3-§2.4) |

### 9.2 Each 2 mm band b (every layer l whose interval (z_{l−1}, z_l] overlaps [2b, 2b + 2); band 0 includes layer 0)

| # | quantity | rule / value |
|---|---|---|
| B1 | smoothed node XY at layers l−1 and l, with links | §2; tips locked |
| B2 | node radii | §1.1 (tips 0.4 → 1.0 over 4 layers, then +0.0245 per layer, foot flare in the lowest ≈ 13 layers) |
| B3 | section at ζ_l = z_l − 0.14 | ellipse at the axis crossing, minor r, major r/cos φ; caps at bifurcations; inscribed polygons with chord tolerance 0.015; tips are not capped above the tip layer |
| B4 | own-object trim | `C_l = (L_{l−1} ∪ L_l) ⊕ 0.21 ∪ L_{l+1} ⊕ 0.105` (plus the B exterior obstacle except its corridors) |
| B5 | roofs (top contact at insertion layer, interfaces 1–2 layers below) | overhang polygons from the front stage, ≥ 1 mm², reduced by the bed-avoidance of radius 0.4 on each lower layer; not trimmed by B4 |
| B6 | lost-branch drop columns | §3.5; B: down to model or bed; O/S: ≤ 5·r/h layers unless very lost |
| B7 | B-only pruning | can only remove; ignore for a sound envelope |
| B8 | clean-up | `fill_notches(w)`, simplify 0.03 |
| B9 | bed clip | canonical copy only (§6) |
| B10 | material vs polygon | Support: edge = polygon; Interface: ≤ +0.03 (hard bound +0.2); arcs ±0.0375 |
| B11 | band union | ∪ over the layers of the band of (B3 ∖ B4 ∪ B5 ∪ B6), material-adjusted by B10 |
| B12 | uncertainty from upstream variation | δ_band from the calibration (design §5.5). Corpus: B plates up to 4.4 mm outside a 5-run union; S GUI 1.5-5.4 mm outside the R union |

### 9.3 Sound fallback (no node-level port)

If only an influence-area bound is available, the per-band sound envelope is
`∪_l (IA_l^{bound} ⊕ (σ + r_max(l)·(1/cos φ_max)))`, where:

- `IA_l^{bound}` is the union of the influence areas of layers l−1 and l;
- σ is the smoothing margin of §2.6;
- the first layer uses `⊕ r_0` followed by the geodesic flange F6.

## 10. Version differences at a glance

| topic | B 2.8.2.61 | O 2.4.2 | R 2.5.0-rc | S 2.4.0 |
|---|---|---|---|---|
| smoothing anchor limit | no | **yes** | **yes** | no |
| exterior-of-bed obstacle (2 mm corridors) | **yes** | no | no | no |
| flat foot on bed roots | no (hemisphere) | **yes** | **yes** | no |
| self-folding tube split | **yes** | no | no | no |
| safety offsets and tiny-area removal in branch trims | no | **yes** | **yes** | no |
| lost-branch drop | to model or bed | ≤ 5r/h unless very lost | same as O | same as O |
| keep-main pruning | **yes** | no | no | no |
| roof recovery from branch slices | no | yes (inactive here) | yes (inactive here) | no |
| `raft_first_layer_expansion = −1` | auto **2 mm** | **0** | **0** | **0** |
| trunks printed | hollow + lightning void fill | sheath + rectilinear (base pattern ≠ none) | same as O | hollow |
| auto-brim thermal term Θ | 0 | on | on | on |
| extra 5 mm brim for narrow islands | off | on | on | on |
| closing of the no-brim set | yes | no | no | yes |
| Z-limited exclusion volumes trim supports | no | no | **yes** (unused here) | no |

## 11. Open questions

- **Q1. Smoothing displacement in practice.** How far, in mm, do smoothed nodes end up from the node
  placement's output on the goats? **Partly settled by the fidelity decision** (design §14 Q17): the
  generator runs this pass (§2.4), so σ is needed only by the sound fallback of §9.3. Still worth measuring
  with an instrumented port (pass on/off on a reproduced P1S tree vs the probe G-code) before WP S3 starts,
  because it bounds how far a smoothing difference can move material.
- **Q2. Bambu corridor obstacle.** The corridor finding rests on source plus one placement (`exp/outA`).
  It needs a single-object check with the canonical goat near an edge midpoint vs near a corner, and
  near the P1S cutter. It matters only for the canonical copy, which the packer should keep interior.
- **Q3. Snapmaker 2.4.0 vs Orca 2.4.2 / 2.5-rc.** The drawing differences (flat foot, no anchor limit,
  drop budget) are unmeasured because the Snapmaker CLI segfaults. They are probably part of the 5.4 mm
  L1 / 8 mm band GUI-vs-CLI difference in the research, which also contains a rotation change.
  Candidate test: a single-goat GUI slice at the probe's rotation.
- **Q4. Interface line inset.** Measured 0.18 mm inside the clip (U1), which matches "lines end at
  P ⊖ ½·s". Whether linking moves along `P ⊖ 0.05·s` ever print on organic roofs is not confirmed.
  Until then, keep a +0.2 mm hard bound for interface material.
- **Q5. Thin-island extra brim (Orca/Snapmaker).** It does not fire for goats. For another model it can
  add a ≈ 5 mm ring that is then trimmed by the support layer-0 area and foreign islands ⊕ 0.1. That
  needs its own predictor branch if non-goat models are packed with auto brim.
- **Q6. Bambu flange expansion with miter limit 3.** On tube polygons this is equivalent to round. On a
  roof that reaches the bed (sharp corners) it could spike up to 3·δ = 1.2 mm per step before trimming.
  Not seen on goats; check before trusting the 2 mm figure for other models.
- **Q7. Lost branches on the goats.** Do any dropped columns occur in the probes? If so, they are
  vertical columns under overhang tips and add band area outside the trees.

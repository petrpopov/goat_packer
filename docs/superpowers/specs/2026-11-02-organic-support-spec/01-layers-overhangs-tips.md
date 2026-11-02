# Organic tree supports, part 1: layers, sections, paint, overhangs, settings, tip seeding

2026-11-02. Behaviour spec for stages A1-A4 and B1 of `2026-11-02-slicer-free-packer-design.md`
(§5.2: `layers`, `sections`, `paint`, `overhangs`, `tips`) plus the derived settings every later
stage reads. Later parts of this spec set cover clearance volumes (collision / avoidance), descent,
merging, placement, smoothing, drawing and the first-layer base.

**Licence rule.** Bambu Studio, OrcaSlicer and Snapmaker Orca are AGPL-3.0. This file describes
*behaviour* in our own words: prose, formulas, constants, order properties, edge cases and recorded
test vectors. It contains no pasted or transliterated source. Citations point at the upstream lines
so a reviewer can check a behaviour; they are not a template for code structure or names. Upstream
identifiers appear only as cross-references next to a citation; our operations and fields use our
own names (design §1, glossary in 02 §1.4). Where an upstream procedure is greedy or order-dependent
(the tip outset rounds, the roof stack, the line connector), this part states the result as
recurrences, predicates and precedence rules, not as the upstream loop.

**Citation prefixes** (all under `/tmp/goat_src/<tree>/src/libslic3r/`):

| prefix | tree | slicer | who uses it |
|---|---|---|---|
| B | `bambu_0208` | Bambu Studio 02.08.02.61 | owner's P1S and A2L GUI, local + server CLI |
| O | `orca_242` | OrcaSlicer 2.4.2 | server U1 CLI |
| R | `orca_250rc` | OrcaSlicer 2.5.0-rc | local `/Applications/OrcaSlicer.app` |
| S | `snaporca240` | Snapmaker Orca 2.4.0 | owner's U1 GUI |

"O/S" means O and S behave identically on that point (checked by diff). R differs from O only where
stated.

---

## 0. Data flow and notation

```text
3MF (mesh per part, part types, item/part transforms, paint_supports, config)
  │
  ├─ A1 layers   : layer grid  [ (bottom_k, top_k, slice_z_k) for k = 0..n-1 ]
  ├─ A2 sections : L_k  (object cross-section)      X_k (extrudable part of L_k)
  ├─ A3 paint    : E_k  (enforcer projection)       K_k (blocker projection)   V (vertical enforcer points)
  ├─ settings    : derived constants (§4)
  ├─ A4 overhangs: H_k  (typed overhang polygons per layer, handed to the tree generator)
  └─ B1 tips     : per layer m: ordered list of tip records + roof (interface) polygons
                   → descent stage (next spec)
```

Notation used throughout:

- All geometry lives in the **object frame** (§1.1) on the **flavour's integer grid** (design §5.2,
  02 §1.1):
  - B: **1 unit = 10 nm**, 32-bit coordinates (scaling factor 10⁻⁵, B `libslic3r.h:40,58`);
  - O/S/R: 1 unit = 1 nm, 64-bit coordinates (10⁻⁶ for every bed smaller than 2147 mm, O/S/R
    `libslic3r.h:60-62`).
  Values in "units" below are per flavour; §4 gives both columns.
- `A ⊕ r` grows region A by distance r, `A ⊖ r` shrinks it. The join type is written as a subscript:
  `⊕sq` square corners, `⊕mi(m)` mitred with limit m, `⊕rd(t)` rounded with arc tolerance t.
  Unless stated, upstream offsets use mitred joins with limit 3. **The offset library raises any mitre
  limit below 2 to 2** (02 §1.1), so every `⊕mi(1.2)` in this spec set acts as `⊕mi(2)`: corners are
  mitred out to 2·r and squared off beyond. Upstream's offset also drops input vertices closer than
  0.005·|r| to the previously kept one before offsetting (a fork-specific change, O
  `ClipperUtils.cpp:274-277`, constant `ClipperUtils.hpp:34`). Our `polygon_ops` emulates both
  (design §5.2).
- `\` difference, `∩`, `∪` the usual set operations on regions (non-zero fill unless stated).
- `trunc(x)` truncates toward zero. Lengths are converted mm → grid units by truncation of the double
  quotient x / scaling factor, not by rounding (O `Point.hpp:663-668`). Because 10⁻⁵ and 10⁻⁶ are not
  exact in binary, some round numbers lose a unit: 2.0 mm is 199 999 units in B (2.0/10⁻⁵ =
  199 999.999…), and 0.03 mm is 2 999 units in B. §4 lists the exact integers that result.
- "f32" marks values computed in single precision upstream; the spec says where that changes a
  discrete decision.

---

## 1. Mesh slicing conventions (A1 `layers`, A2 `sections`)

### 1.1 Object frame

- Every printable instance with the same 3×3 linear part of its transform (rotation, scale,
  mirror) shares one sliced object; the XY translation is stored per instance (O
  `PrintApply.cpp:138-167`). Translation never enters the support computation except through the
  bed clip of later stages.
- The linear part keeps the instance's Z translation, so the object frame Z is world Z. The
  pipeline assumes the object rests on the bed (min z of the model parts = 0); our `layers` stage
  must drop the mesh to z = 0 itself and fail if a part is below 0.
- XY origin: the centre of the XY bounding box of all **normal parts** after the linear transform
  (B `PrintObject.cpp:63`, O `:85-112`). All slices, paint projections, fill-line anchors and the tip
  dedup grid (§5.7) are expressed relative to this centre. The goat projects need the full item
  transform plus every component transform (`3D/3dmodel.model` items and components), not the
  first component only.
- Mesh vertices are transformed in f32 with X and Y pre-multiplied by the grid scale (10⁵ in B, 10⁶ in
  O/S/R) and Z left in mm
  (O `TriangleMeshSlicer.cpp:1827-1862`). A port in f64 differs by a few nm; that is inside every
  margin we use.
- Filament shrink: B scales each region's slices and the paint projections about the origin by
  100/`filament_shrink` (B `PrintObjectSlice.cpp:1205-1223`, `:1404-1429`, `PrintObject.cpp:4099`).
  O/S/R fold XY/Z shrink into the instance transform (O `PrintApply.cpp:148`, `Slicing.cpp:62`
  `object_shrinkage_compensation`). All three projects use 100 %: identity.

### 1.2 Layer grid

Inputs: `layer_height` h, `initial_layer_print_height` h₀ (≤ 0 means h), `min_layer_height` h_min
(clamped below by 0.01, default 0.07, and to ≤ h), object height H, `layer_config_ranges`, a stored
variable-layer profile (`Metadata/layer_heights_profile.config`), `precise_z_height`, raft layers.

Rules (O `Slicing.cpp:62-232` and `:234-307`, `:807-868`, `:870-901`; B `Slicing.cpp:62`, `:172`,
`:732`, `:790`; S identical to O; R identical to O):

1. **Profile.** If the object carries a stored profile whose last height equals H (±1e-3) and whose
   first step equals h₀, it is used as is (B additionally discards it if any step lies outside the
   nozzle's [min, max] layer height, B `PrintObject.cpp:3255-3291`). Otherwise the profile is built
   from the ranges:
   - the first layer [0, h₀] at height h₀ is always the first entry when there is no raft;
   - ranges are sorted, trimmed so they do not overlap (each starts no lower than the previous
     end) and clipped at H (B clips at the shrink-compensated height, O at the uncompensated one);
     ranges narrower than 1e-4 mm are dropped;
   - gaps between ranges are filled with h;
   - consecutive steps with the same height collapse: when a third consecutive entry repeats the
     height of the two before it, the previous entry's Z is moved up instead of adding a point.
   The goat projects collapse to the 8-number profile `[0,h₀, h₀,h₀, h₀,h, H,h]` (P1S has no ranges;
   U1 ranges 14.9-18.24 and 33.14-60, A2L 15.1-18.1 and 33.5-60 all repeat h = 0.28).
2. **Fixed-profile test** (decides the support style, §1.2.1): the profile is "fixed" when it has 4
   or 8 numbers, the first step equals h₀ (and is at least ½h₀ thick), and the remaining step is h,
   or the object is shorter than h₀ + ½h.
3. **Layers.** Walk upward with `top` starting at h₀ (layer 0 = [0, h₀]). For every next layer read
   the height from the profile at the current position (linear interpolation between profile
   points; constant for a fixed profile). A layer [top, top + height] is emitted only if its
   mid-height top + ½·height is below H; the walk stops as soon as top + ½·h_min reaches H.
   Heights accumulate in f64 (0.2 + 0.28 + … is not exactly decimal; keep f64).
4. `slice_z_k = ½(bottom_k + top_k)`, `print_z_k = top_k` (+ raft thickness, 0 here). Exception, O
   2.4.2 and R only: if a region enables Z anti-aliasing (`zaa_enabled`), slice_z of layers k ≥ 1 is
   `bottom_k + zaa_min_z` (O `PrintObjectSlice.cpp:24-48`). Not present in the projects; reject.
5. Slice planes are handed to the mesh slicer as f32 (`zs_from_layers`). Coplanarity tests against
   mesh vertices (§1.3) compare f32 to f32.
6. `precise_z_height = 1` rescales the last 5 layers so the top equals H (O `Slicing.cpp:713-805`).
   All projects: 0. Supported only as "off"; reject otherwise.
7. Layers whose sliced area is empty at the top of the stack are removed after slicing (O
   `PrintObjectSlice.cpp:1188-1196`).

#### 1.2.1 Style resolution that depends on the grid

`support_type = tree(auto|manual)` with `support_style = default` resolves to **Organic**, except:

- B: if `has_variable_layer_heights` (fixed-profile test fails for an object with custom layering) or
  a soluble interface, the style becomes **Tree Hybrid**, a different generator (B
  `Support/SupportParameters.hpp:136-156`, `Print.cpp:1430-1445`). Organic chosen explicitly with a
  variable profile is a slicing error in B.
- O/S/R: always Organic; a variable profile with Organic is refused (O `Print.cpp:1399-1415`).
- Our implementation: a non-fixed profile is an explicit error ("variable layer height: Bambu
  switches to Tree Hybrid, Orca refuses"), as in the design doc.

Snapmaker 2.4.0 adds mixed-filament gradient and Z-dithering modes that rewrite the ranges before
step 1 (S `PrintObject.cpp:3838-3900`). All are off in the U1 project
(`mixed_filament_gradient_mode = 0`, `dithering_z_step_size = 0`, `dithering_local_z_mode = 0`);
treat any of them as unsupported.

#### 1.2.2 Values for the projects (validated against existing single-object G-code)

| | U1 | P1S | A2L |
|---|---|---|---|
| h₀ / h / h_min | 0.25 / 0.28 / 0.08 | 0.20 / 0.28 / 0.08 | 0.20 / 0.28 / 0.08 |
| H (goat, mesh) | 44.7435 | 44.7435 | 44.7435 |
| layers n | 160 | 160 | 160 |
| print_z | 0.25 + 0.28·k | 0.20 + 0.28·k | 0.20 + 0.28·k |
| slice_z | 0.125, then 0.39 + 0.28(k−1) | 0.10, then 0.34 + 0.28(k−1) | as P1S |
| top layer | [44.49, 44.77] (above H, allowed) | [44.44, 44.72] | [44.44, 44.72] |

The G-code of `/tmp/goat_research/slices/{u1,p1s,a2l}_r1/plate_1.gcode` has 160 layers, first Z
0.25 / 0.20, last Z 44.77 / 44.72. Script: `/tmp/spec01/layers.py`.

### 1.3 Cross-section of one volume at slice_z

For each volume that needs slicing (normal part, negative part, modifier) and each layer
(O `PrintObjectSlice.cpp:148-222`, `TriangleMeshSlicer.cpp:156-321`, `:1383-1532`, `:1738-1825`,
`:2003-2050`):

1. **Triangle ∩ plane.** Each triangle crossing z = slice_z contributes one segment. An edge
   crossing strictly yields the point `b + (a − b)·t` (t from the Z values, f64, rounded half-up to
   the grid unit; vertices ordered by index for a stable result). A vertex exactly on the plane is used as is.
   An edge lying in the plane counts only when the third vertex is **below** the plane, so the plane
   catches the top faces of a horizontal surface and misses its bottom faces. Horizontal triangles
   are not sliced.
2. **Loops.** Segments are chained into loops first by triangle connectivity, then by exact
   end-point matches (forward, then allowing reversal), then remaining open chains are closed
   across gaps up to 2 mm (forward, then reversed). Segments from parallel workers arrive in any
   order; the resulting loop set is the same, only start vertices differ.
3. **Fill rule:** `slicing_mode` regular → non-zero; even-odd → even-odd; close holes → positive and
   every loop made counter-clockwise. All projects: regular.
4. **Closing.** With closing radius r_c = `slice_closing_radius` (0.049 mm in all projects): union
   with the fill rule, then grow by r_c and shrink by r_c (mitred, limit 3). B 02.08 first removes
   micro back-tracks: any run of ≤ 8 edges with path length ≤ min(2 µm, 0.05·r_c) whose ends lie
   within min(0.1 µm, max(1 nm, that length/20)) of each other is cut out (B
   `TriangleMeshSlicer.cpp:1739-1798`). O/S/R do not.
5. **Simplification.** If `resolution` > 0.001 mm (0.03 in all projects) every ring is simplified by
   Douglas-Peucker with a fixed 0.0025 mm tolerance (the `resolution` value itself is not used here,
   O `PrintObjectSlice.cpp:170-172`). Douglas-Peucker on a closed ring depends on its start vertex
   (sub-2.5 µm effect).

### 1.4 Multi-part objects: what the support generator sees

Volume types (3MF `model_settings.config` `subtype`): `normal_part`, `negative_part`,
`modifier_part`, `support_enforcer`, `support_blocker`. Processing is in volume id order
(O `PrintObjectSlice.cpp:269-470`, B `:241-485`).

- The **object section** used by supports is `L_k` = union over all regions of their slices. With
  more than one region (the goat: body and pompom use different extruders) the union is done by a
  10-unit outward offset (10 nm in O/S/R, 100 nm in B: `ClipperSafetyOffset` = 10 units), so L_k is
  that much larger than the exact union (O `Layer.cpp:38-64`,
  `ClipperUtils.hpp:362-365`); with one region it is the region slice as is. For geometry this is:

  ```text
  S = ∅
  for each volume v in id order:
      if v is a normal part:   S = S ∪ section(v)
      if v is a negative part: S = S \ section(v)      # only removes parts that come earlier
  L_k = S                                              # modifiers never change S
  ```

  Overlapping normal parts are carved so each area belongs to one region (B has an
  order-independent option, `enable_order_independent_overlap_carving`, 0 in P1S/A2L); the union
  is unaffected.
- **Version difference, source-level, unverified:** when two or more printable volumes share a
  layer and none overlaps another's XY box, O/S/R assign only the first part's slice in their fast
  path (O `PrintObjectSlice.cpp:288-330`); B assigns all of them (B `:272-307`). Irrelevant for the
  goat (body and pompom boxes overlap) but must be pinned before supporting multi-part objects with
  disjoint parts. Open question Q5.
- XY hole / contour compensation (both 0 in the projects), B circle compensation
  (`enable_circle_compensation = 0`), O/S/R `make_overhang_printable` (0 in U1) would all change
  L_k. Reject non-default values.
- Colour painting (MMU segmentation) and fuzzy skin re-partition regions but do not change L_k.

### 1.5 Elephant-foot compensation

- B: layer 0 only. O/S/R: the first `elefant_foot_compensation_layers` layers (U1: 1), linearly
  tapering. Value 0.15 mm in all projects. Disabled when a raft is used.
- The compensated geometry is printed, but `L_k` for those layers is **restored to the
  uncompensated section** afterwards (B `PrintObjectSlice.cpp:1346-1365`, O `:1412-1430`). With one
  region it is the region slice before compensation; with several regions (the goat: body and
  pompom use different extruders) it is the merged regions closed by ε = 1.5·r_c (grow ε, union,
  shrink ε). ε is a process-lifetime constant taken from the first object ever sliced in that
  process (a `static` initialisation; harmless while every object uses 0.049).
- Consequence: the support generator, including the later first-layer trim, sees the uncompensated
  layer 0.

### 1.6 The extrudable filter

Detection (A4) works on

```text
X_k = L_k ∩ ( (L_k ⊖ w/2) ⊕ w )          joins mitred, limit 3
```

with w the configured line width: B uses the raw `line_width` number (a percentage would be
misread), O/S/R resolve it against nozzle 0. All projects: w = 0.42 mm. X_k drops every feature that
an erosion by w/2 deletes (anything thinner than about w) and keeps everything else exactly
(B `Support/TreeSupport.cpp:864`, O `:803`, S `:776`, R `:889`).

Later stages use other versions of the section (listed here so the data contract is explicit):
clearance volumes use L_k simplified at `resolution` = 0.03 mm (Douglas-Peucker, then Clipper
simplification; B asks for strictly simple output, O/S/R do not; O `Support/TreeModelVolumes.cpp:115`,
B `:133`); the first-layer base trims against L_0.

---

## 2. Paint data (A3 `paint`)

### 2.1 Where it lives

- Attribute `paint_supports` on each `<triangle>` of a part mesh in the Bambu-style 3MF (B
  `Format/bbs_3mf.cpp:316`, O `:329`); `slic3rpe:custom_supports` in the Prusa-style 3MF
  (O `Format/3mf.cpp:112`). Missing or empty = unpainted. Separate attributes carry seam, fuzzy-skin
  and colour paint (`paint_color`); they are ignored here.
- The index is the triangle's position in that part's mesh as stored. The loader keeps the order
  (it may flip all windings if the mesh volume is negative, and it re-centres the mesh, neither of
  which changes which world triangle a code belongs to; B `bbs_3mf.cpp:5322-5426`).
- Only **normal parts** contribute paint to supports (O `PrintObject.cpp:4650-4651`).

### 2.2 Bit stream

Per triangle (B `Model.cpp:4605-4650`, O `Model.cpp:3545-3590`, `TriangleSelector.cpp:1692-1876`):

1. Read the hex string from its **last** character to its first. Each character is one nibble; its
   four bits enter the stream least-significant first. The stream is a sequence of 4-bit codes read
   in that order.
2. Decode one node from a code c:
   - `split = c & 3` (0..3 sides split);
   - if split > 0: the node has split + 1 children; `special = c >> 2` selects the special vertex
     (§2.3). The children follow immediately as nodes, **last child first**.
   - if split = 0 (leaf): `state = c >> 2`; if that is 3, the next code n gives state = n + 3
     (O/S/R). B reads a chain instead: every following code equal to 15 adds 15, the first code
     below 15 ends it: state = 3 + 15·(number of 15s) + last. The two only disagree for states ≥ 18
     (colour paint).
3. States: 0 none, 1 **enforcer**, 2 **blocker**, ≥ 3 colour indices (not support-related).

Examples from the goat: `"4"` is an unsplit enforcer triangle; `"4002"` = split into 3 (special
vertex 0), children read as last = none, middle = none, first = enforcer.

### 2.3 Child geometry

A split node with vertices (v0, v1, v2) and special index s is first rotated so that
a = v_s, b = v_{s+1}, c = v_{s+2}. Midpoints are the f32 average of the two end vertices (shared
with the neighbouring triangle, so both sides get the identical vertex).

| split | new points | children in storage order 0, 1, … |
|---|---|---|
| 1 | m = mid(b, c) (edge opposite a) | (a, b, m), (m, c, a) |
| 2 | p = mid(a, b), q = mid(a, c) (both edges at a) | (a, p, q), (p, b, q), (b, c, q) |
| 3 (s = 0) | p = mid(a, b), r = mid(b, c), q = mid(c, a) | (a, p, q), (p, b, r), (r, c, q), (p, r, q) |

Recursion depth is unbounded in principle; the goat uses up to 6
(O `TriangleSelector.cpp:1390-1440`).

### 2.4 Validation against `/tmp/goat_research/mesh3mf.py`

`mesh3mf.decode_paint` reads the stream exactly as §2.2 (reversed characters, LSB-first nibbles,
2-bit split, 2-bit special, child recursion, state 3 → +3) and is correct for structure. It does
**not** build child geometry: it returns "has an enforcer leaf", so a partly painted triangle is
treated as fully painted. A geometric decoder written for this spec (`/tmp/spec01/paint_check.py`)
gives on all three projects (same mesh, same paint):

| quantity | value |
|---|---|
| painted triangles | 7907 (7668 are `"4"`, 239 split) |
| max depth | 6 |
| enforcer leaves / none leaves | 10158 / 2191, no blockers |
| enforcer area / unpainted area inside split triangles | 60.80 mm² / 3.22 mm² (mesh3mf over-counts by 5 %) |
| unconsumed bits | 0 in every string |
| child areas vs parent | equal within 1.1·10⁻⁴ relative (f32 midpoints) |
| exactly vertical painted leaves (§2.6) | 0 |

### 2.5 Strict facet set

For projection, the painted leaves of the requested state are emitted as triangles; where a
neighbour is split more finely, the leaf is cut at the neighbour's midpoints so that the patch has
no T-joints (O `TriangleSelector.cpp:1522-1600`). Geometry is unchanged; only the connectivity used
by the projection (boundary edges of the patch) depends on it.

### 2.6 Projection onto layers

For each normal part, for each state (enforcer, blocker), project its painted facets
(O `PrintObject.cpp:4646-4673`, `TriangleMeshSlicer.cpp:2058-2158`, `:587-905`; S identical; B adds
the shrink scale §1.1; R uses the sliced transform):

1. Transform vertices into the object frame (f32, XY in grid units).
2. Classify each painted triangle by the sign of the cross product of its XY projection computed in
   integer grid units after truncating the coordinates, flipped if the transform mirrors:
   negative → **down-facing**, positive → up-facing, zero with three distinct vertices →
   **vertical**, zero with a repeated vertex → degenerate.
3. Down-facing and degenerate triangles form the patch to project. Two triangles are neighbours in
   the patch only if both are in it; every other edge is a patch boundary.
4. **Layer assignment.** With `z_k = slice_z_k` (f32): the part of the patch with
   `z_{k−1} < z ≤ z_k` (for k = 0: z ≤ z_0) is projected vertically and becomes part of `E_k`
   (enforcers) or `K_k` (blockers). In words: a painted underside at height z belongs to the first
   layer whose slice plane is at or above it, which is the first layer that contains material
   above that underside. A horizontal painted triangle lying exactly on a plane z_j goes to layer
   j + 1. Parts above the last plane are dropped.
5. The per-layer result is a set of counter-clockwise loops; projections of several triangles may
   overlap and are used with non-zero fill (unioned by the later boolean operations). Each part's
   projection is appended to the same per-layer list.
6. **Vertical enforcer points.** For enforcers only, each vertical painted triangle contributes its
   centroid (object frame). Each point becomes a circle of radius 0.5 mm. Upstream passes 25 where a
   chord *error* in grid units is expected (`make_circle(radius, 25)`, B
   `Support/TreeSupport3D.cpp:4459-4473`, O `:3470-3483`), so the vertex count is
   ⌈π / acos(1 − 25/r_units)⌉: a **315-gon** on the 1 nm grid (O/S/R), a **100-gon** on B's 10 nm grid.
   The circle is added to the overhang list of the first
   layer whose slice_z ≥ the point's z, if that layer is neither 0 nor beyond the top. The goat has
   none.
7. **Enforcer / blocker volumes** (`support_enforcer`, `support_blocker` parts) are sliced like
   normal volumes at slice_z (default slicing parameters) and form the start of the same per-layer
   lists; paint projections are appended (O `PrintObjectSlice.cpp:1545-1597`).
8. Order: B projects blockers then enforcers **before** detection and trims trailing empty layers
   from both lists (B `Support/TreeSupport.cpp:878-893`); O/S/R project after the clustering step
   (O `:997-1001`). The resulting E_k, K_k are identical.

Goat values: painted triangles span Z 2.12-26.09 mm, but the down-facing enforcer leaves only
2.12-25.84 mm (10137 down-facing leaves, 21 up-facing or vertical ones that never project). E_k is
non-empty on k = 8-13, 21-26, 46-48, 77-83, 85-93 on the P1S/A2L grid (31 layers: legs and tail
2.3-3.7 mm, 5.9-7.3 mm, belly 12.9-13.5 mm, chin, neck and ears 21.6-26.1 mm) and on the same
ranges ending at 92 on the U1 grid (30 layers). Script: `/tmp/spec01/downz.py`.

---

## 3. Overhang detection (A4 `overhangs`)

Sources: B `Support/TreeSupport.cpp:661-1290`, O `:668-1125`, S `:641-1098` (identical to O),
R `:739-1302` (O plus a bed-tilt / belt shift of the lower layer that is inactive without tilt, and
an equivalent but faster "nothing below" test).

### 3.0 Constants

| symbol | meaning | value |
|---|---|---|
| θ | `support_threshold_angle` | U1 30, P1S 40, A2L 35 |
| θₑ | effective angle: θ + 1°, or 30° if θ ≤ 0; at most 89° | 31° / 41° / 36° |
| d_k | allowed step at layer k: `height(k−1) / tan θₑ` (f32 height); `−0.15·w` for k < `enforce_support_layers` (0 here) | see table below |
| w | line width (§1.6) | 0.42 |
| e_E | enforcer grow ("tip diameter fudge", fixed even when O's tip diameter differs) | 0.8 mm |
| e_K | blocker grow (`g_config_tree_support_collision_resolution`, f32) | 0.2 mm |
| c_max | cantilever distance | 3 mm |
| sharp-tail limits | bbox 6 mm, height 16 mm, growth 5 mm, erosion 5w | — |
| B small-overhang | length 2 mm, radius 2.5w = 1.05 mm, big flat 10 mm (area 100 mm²) | — |
| guard | wall clock 30 s, or > 100 overhang islands on one layer | — |
| ρ | `RESOLUTION` | 0.0125 mm |

| d_k (mm) | U1 | P1S | A2L |
|---|---|---|---|
| k = 1 (below is layer 0) | 0.4161 (h₀ 0.25) | 0.2301 | 0.2753 |
| k ≥ 2 | 0.4660 | 0.3221 | 0.3854 |

### 3.1 Plain overhang (all flavours)

For k ≥ 1:

```text
P_k = X_k \ ( X_{k−1} ⊕sq d_k )
```

Layer 0 never has a plain overhang. Square joins matter: a convex corner of X_{k−1} grows into a
square, not a mitre.

### 3.2 Bambu 02.08.02.61 per-layer pass (parallel over k)

Layers are processed independently except for the shared guard flags (§3.9). For k ≥ 1 with
`support_type` auto (manual: only layers with enforcers):

1. `Eh_k = (((X_k \ X_{k−1}) ∩ E_k) ⊕ e_E) \ X_{k−1}` if E is non-empty (mitred grow).
   This is the **enforced overhang**: the new material this layer adds,
   restricted to painted areas, grown 0.8 mm, then cut back to what is not resting on the layer
   below.
2. `Ov_k = (P_k \ (K_k ⊕ e_K)) ∪ Eh_k` (blockers removed first, enforced area added).
3. Guard check (§3.9). If it trips on this layer, steps 4-5 are skipped for this layer.
4. **Sharp tails, seed pass.** On `C = (X_k \ (K_k ⊕ e_K)) ∪ Eh_k`, each island I is a seed if all hold:
   - `I ⊕ 0.1w` does not touch X_{k−1};
   - area(I) < 100 mm² and (bbox area of I < 100 mm² or `I ⊖ 2.5w` is empty);
   - `I ⊖ 0.1w` is non-empty;
   - if k ≥ 2: `I ⊕ 0.1w` does not touch X_{k−2}.
   Seeds are stored with accumulated height 0.
5. **Cantilevers.** Let `T = X_{k−1} ⊕sq w` and `U = (X_{k−1} ⊕sq d_k) ⊕ (max(w − d_k, 0) + 0.1)`.
   An island J of Ov_k is a cantilever if
   - some ring of J (outer or hole) has no vertex inside T ("floating ring"), or
   - otherwise, with `Q = J ∩ U` non-empty, the largest distance from a vertex of J's outer ring to
     the boundary of Q exceeds c_max = 3 mm.
   (An island with Q empty and no floating ring is not a cantilever.) A ring overhang that fully
   surrounds the layer below (a flange) is caught by the floating-ring rule.

Layer 0: every island of `X_0 \ (K_0 ⊕ e_K)` whose bbox is ≤ 6 mm in **both** X and Y becomes a
seed with accumulated height h₀; layer 0 has no overhang.

### 3.3 Orca 2.4.2 / Snapmaker 2.4.0 / Orca 2.5.0-rc per-layer pass (parallel over k)

1. `Ov_k = P_k` (no blockers, no enforcers yet).
2. Guard check (§3.9); on trip, skip 3-4 for this layer.
3. **Sharp tails, seed pass** on X_k: an island I is a seed if `I ⊕ 0.1w` does not touch X_{k−1}
   and `I ⊖ 0.1w` is non-empty. No area, bbox or k−2 conditions.
4. **Cantilevers:** only the distance rule of §3.2.5 (no floating-ring rule; an island with Q empty
   is not a cantilever).

Layer 0: every island of X_0 with bbox ≤ 6 mm in **at least one** of X, Y becomes a seed with height
h₀.

### 3.4 Sharp-tail growth pass (sequential, bottom-up, both flavours)

Runs only while sharp-tail detection is on (§3.9). For k = 1..n−1, over the islands I of X_k
(B: of `(X_k \ (K_k ⊕ e_K)) ∪ ((X_k \ X_{k−1}) ∩ E_k)`, which equals X_k minus grown blockers),
with T_{k−1} = the tail list of layer k−1 (seeds and grown tails):

1. `I ∩ T_{k−1}` must be non-empty, else not a tail.
2. Not a tail if the bbox of `I ∩ T_{k−1}` exceeds 6 mm in both X and Y (a well-supported base).
3. Accumulated height `h_acc = height(k) + h(first tail of T_{k−1} that intersects I)` (f32 sum);
   `h_acc` must be ≤ 16 mm.
4. With `N = I \ T_{k−1}`: not a tail if `bbox(N).size − bbox(T_{k−1}).size` exceeds 5 mm in both
   axes, or if `N ⊖ 5w` is non-empty.
5. Otherwise I is a grown tail of layer k with height h_acc.

### 3.5 Clusters (sequential, bottom-up)

Every overhang island J of Ov_k (in the order the boolean step produced them) joins the **first**
existing cluster (creation order) for which `J ⊕ w` intersects that cluster's overhangs on layer
k − 1 and the cluster's outline bbox; otherwise it starts a new cluster. A cluster keeps an
**outline** (the union of its first member as is and every later member grown by w: the grow
distance is only set by the join test, not by cluster creation) and its min/max layer. A cluster is
flagged cantilever if any member touches a cantilever of its layer. The un-grown first member
matters for single-layer clusters in §3.6.

- B stores every member per layer (B `:794-847`).
- O/S/R store **one** member per layer: a second island of the same layer that joins the same
  cluster is merged into the cluster outline but is not kept as a member, so it is never emitted
  as an overhang in non-critical mode (O `:721-772`, the per-layer map only accepts the first key).
  Open question Q8 (effect on general models; none in critical-only mode).

### 3.6 Small-overhang removal (`support_remove_small_overhang = 1`, guard not tripped)

For each cluster:

- **Sharp-tail flag:** set if any layer's tail list touches the cluster (B: the cluster's members of
  that layer; O: the cluster outline).
- B (`check_small_overhang`, B `:783-823`), skipped for sharp-tail or cantilever clusters:
  - per member layer: erode members by w/2; `narrow` = largest over eroded islands of min(bbox w,
    bbox h); average narrow width and member area over the layers;
  - small if every layer's narrow ≤ w, average narrow < w/2, average area < 4 mm² and the cluster
    spans < 3 layers;
  - otherwise small if `outline ⊖ 2.5w` is empty or its bbox is < 2 mm in both axes. (The upstream
    condition also compares the layer count with half of 2 mm in grid units, which is always true.)
  - `enforce_add`: the cluster touches E_l on one of its layers.
- O/S/R: not sharp-tail, not cantilever, and `outline ⊖ w` has a bbox narrower than 2w in X **or**
  Y (an empty erosion counts as narrower) → small. Because the first member enters the outline
  un-grown (§3.5), a single-layer cluster is small when the overhang itself is narrower than about
  4w = 1.68 mm in X or Y; a multi-layer cluster is judged on its later layers grown by w.
- Emission: B emits all members of non-small clusters, and for small clusters with `enforce_add`
  only `member ∩ E_l` on layers that have enforcers; each emitted polygon carries the cluster's
  type bits. O/S/R emit the kept member of each non-small cluster.
- B also types clusters **BigFlat** (outline area > 100 mm²) or **ThinPlate** (bbox > 10 mm in some
  axis, starting below layer 20, not touching the eroded union of all lower layers plus the bed
  ring, B `:825-832`, `:4373-4412`). For Organic these types only matter in §3.7.

### 3.7 Final per-layer assembly

**B (sequential, B `:1213-1290`), per layer k:**

1. Critical-only (`support_critical_regions_only = 1`, auto):
   - if k = 0 or E_k is empty: H_k = all cantilever islands of layer k (typed cantilever);
   - else: with `G = E_k ⊕ e_E` (each loop treated as its own polygon, then grown): keep whole every
     emitted polygon whose type has the cantilever or sharp-tail bit (so a whole cantilever
     **cluster's** members, not just the cantilever islands); keep `polygon ∩ G` for the others;
     big-flat pieces are unioned together.
2. Non-critical: H_k = the polygons emitted in §3.6.
3. **Sharp-tail supports** (k ≥ 1): for each tail t of layer k with height h_t, if
   `trunc(10·h_t) mod 5 = 0` (f32 multiply), add `t \ (X_{k−1} ⊕ ρ)` typed sharp tail.
4. If `max_bridge_length > 0`: remove bridged areas (§3.7.1). P1S/A2L: 0.

**O/S/R (sequential, O `:1043-1115`), per layer k:**

1. Sharp-tail supports: on layer 0, all seeds; on k ≥ 1, as B step 3.
2. If K_k exists: remove `K_k ⊕ e_K` from the emitted overhangs, the cantilevers and the sharp-tail
   supports.
3. Critical-only: H_k = cantilevers of layer k (the emitted overhangs are discarded).
4. If `max_bridge_length > 0` (U1: 10): remove bridged areas from H_k (§3.7.1).
5. If E_k exists and k ≥ 1: add `(((X_k \ X_{k−1}) ∩ E_k) ⊕ e_E) \ X_{k−1}` (typed enforced).
6. Add the sharp-tail supports (typed sharp tail).

The sharp-tail emission rule is not "every 0.5 mm": with 0.28 mm layers, h_t takes the f32 values
0, 0.28, 0.56, 0.84, …, and `trunc(10·h_t)` is a multiple of 5 at 0, 0.56, 2.52, 3.08, 5.04, …
Values close to a multiple (7.0 mm) depend on f32 rounding; our code must accumulate in f32 in the
same order.

#### 3.7.1 Bridge removal (only when `max_bridge_length` > 0)

From the region's perimeters of layer k: straight perimeter pieces outside `L_{k−1} ⊕sq w/2`, extended
by w at both ends, whose both ends lie inside L_{k−1}, are bridges (if longer than
`max_bridge_length` + 10 nm they are kept only when "break bridges" is on, which detection never
asks for). They are thickened to the bridging flow width, unioned with the region's bottom-bridge
fill surfaces whose bbox is below `max_bridge_length` in both axes, cut back by the unsupported
bridge edges grown by 1.2 mm, and subtracted from H_k (O `PrintObject.cpp:4306-4420`). This needs
perimeters and bridge classification, which we do not compute.

Evidence that it does not matter for the goat: a U1 single-object slice in R with
`max_bridge_length = 0` produced support identical, layer by layer (symmetric difference 0 on all
160 layers), to the same project with 10 (`/tmp/spec01/exp/cmp.py`). Our implementation treats
`max_bridge_length > 0` as "no effect" and records it in the report; a model where bridges overlap
cantilevers would need this rule (open question Q6).

### 3.8 Hand-off to the tree generator

Per layer the generator receives polygons (B `Support/TreeSupport3D.cpp:4443-4475`, O `:3455-3484`):

- every H_k polygon; polygons whose type has the sharp-tail bit are grown by 0.2 mm (B: any
  polygon with the bit, including members of sharp-tail-typed clusters kept whole in §3.7; O/S/R:
  only the sharp-tail supports of §3.7 step 6);
- the vertical enforcer circles of §2.6.

Overhangs on layers 0, 1, 2 are never used (§5.1).

### 3.9 The guard (nondeterminism S3)

Inside the parallel per-layer pass, after computing the layer's overhang, upstream reads a wall
clock started before the pass. If more than 30 s have elapsed **or this layer's overhang has more
than 100 islands**, it clears two flags shared by all tasks: sharp-tail detection and small-overhang
removal. Effects:

- the tripping layer skips its own sharp-tail seeds and cantilevers;
- layers processed later (by any thread) skip sharp-tail seeds; which layers are "later" depends on
  thread scheduling;
- after the pass, the growth pass (§3.4), small-overhang removal (§3.6) and the cluster sharp-tail
  flag are off globally; all clusters are emitted;
- B's small-overhang flag also gates the sharp-tail typing of clusters.

Realizations: "guard tripped, layers processed before the trip kept their seeds" for any subset
order the scheduler allows. With one thread the order is ascending k, so the trip at layer j keeps
seeds on layers < j. The island-count trigger is deterministic and our code evaluates it exactly;
the clock trigger is not modelled (the goat's detection runs far below 30 s on an idle machine, but
the server has shown load average 44). Q4 (answered provisionally): no goat layer exceeds 100 islands
(at most 40, review appendix A).

### 3.10 What the goat actually depends on (measured)

Single-object slices with all `paint_supports` attributes removed (critical-only kept):

| | painted (reference run) | paint removed |
|---|---|---|
| U1, R 2.5.0-rc: Σ support area over layers / layers with support | 6068 mm² / 91 | 3328 mm² / 81 (z 0.25-22.65) |
| P1S, B 02.08.02.61: same / first-layer support area | 4900 mm² / 570 mm² | 2297 mm² / 480 mm² |

So cantilevers and sharp tails alone produce about half the support and most of the first-layer
foot area. Detection rules §3.2-§3.7 must be reproduced, not only the enforcer rule. Files:
`/tmp/spec01/exp/{u1_nopaint.3mf,p1s_nopaint.3mf,out_nopaint,out_p1snp}`.

---

## 4. Derived settings (inputs of every later stage)

Sources: B `Support/TreeSupportCommon.hpp:28-92` (mesh-group settings), `:257-345` (tree
settings), O `:36-93`, `:265-355`; S as O except bottom-interface handling; R as O. Tip-seeding
locals: O `TreeSupport3D.cpp:1256-1316`, B `:1132-1199`. Support-fill parameters: O
`Support/SupportParameters.hpp:17-238`, B `:16-325`.

Symbols: lh = layer height, w_s = support line width as the **f32 flow width truncated to the flavour
grid** (0.42 → 41 999 units in B, 419 999 nm in O/S/R), D = branch distance, α = branch angle, α_s = slow
angle. **B values are in B's 10 nm units, O/S values in nm** (§0; design §5.2). Physical sizes agree to
within one B unit; the integers differ, and every later truncation inherits the difference.

| quantity | formula | B (P1S, A2L), 10 nm units | O / S (U1), nm |
|---|---|---|---|
| lh | trunc(layer_height) | 28 000 | 280 000 |
| w_s, roof line width | trunc(f32 flow width) | 41 999 | 419 999 |
| r_b branch radius | trunc(branch diameter) div 2; B `tree_support_branch_diameter`, O `…_organic` | **99 999** (2.0 mm → 199 999) | 1 000 000 |
| r_min tip radius | trunc(clamp(tip diameter, 0, branch diameter)) div 2; B fixed 0.8 mm, O `tree_support_tip_diameter` | 40 000 | 400 000 |
| α | clamp(angle, 0, 90° − 1e-4 rad); B `tree_support_branch_angle`, O `tree_support_branch_angle_organic` | 45° | 40° |
| α_s | clamp(slow angle, 0, α − 1e-4 rad); B fixed 25°, O `tree_support_angle_slow` | 25° | 25° |
| move_fast | trunc(tan α · lh) (double tan; tan 45° = 0.9999999999999999) | **27 999** | 234 947 |
| move_slow | trunc(tan α_s · lh) | 13 056 | 130 566 |
| clearance-volume step | move − 2 (fast, slow), used by the avoidance stage | 27 997 / 13 054 | 234 945 / 130 564 |
| tip_layers | max((r_b − r_min) div (w_s div 3), r_b div lh), integer divisions | 4 | 4 |
| radius growth per layer | tan(`tree_support_branch_diameter_angle`) · lh (double) | 2 449.68 (5°) | 24 496.8 |
| radius(dtt) | dtt ≤ tip_layers: r_min + (r_b − r_min)·dtt div tip_layers; else r_b + (dtt − tip_layers)·growth; + foot term (02 §1.2) | r(0) = 40 000, r(4) = 99 999 | r(0) = 0.4 mm, r(4) = 1.0 mm |
| bp_radius (foot target) | trunc(7.5 mm) div 2 | 374 999 | 3 750 000 |
| bp growth per layer | min(tan(0.7 rad)·lh, ½w_s) | 20 999.5 | 209 999.5 |
| layer_start_bp_radius | trunc((bp_radius − r_b) / bp growth) | 13 | 13 |
| recommended min radius at layer l | l < 13: r_b + (13 − l)·bp growth, else 0 | 372 992 at l = 0 | 3 729 993 at l = 0 |
| increase_radius_until | radius r_b, reached at dtt = tip_layers · (r_b div r_b) | 4 | 4 |
| rests on model | not `support_on_build_plate_only` | false | false |
| min_dtt_to_model, max_to_model growth | ceil(1 mm / lh), 1 mm / 2 (unused: build plate only) | 4, ≈ 0.5 mm | 4, 0.5 mm |
| xy | trunc(`support_object_xy_distance`) (B: ≥ 0.01; B clamps to max(top gap, 0.2) if `top_z_overrides_xy_distance`, 0 here) | 75 000 | 750 000 |
| xy_min | min(xy, trunc(½ · max external-perimeter flow width over regions)) | 20 999 | 209 999 |
| use xy_min for tips | xy > xy_min | true | true |
| first-layer gap | `support_object_first_layer_gap` (first-layer base stage) | P1S 1.2, A2L 0.75 mm | 0.75 mm |
| top gap | round(top_z / lh + 1e-4) · lh (`independent_support_layer_height = 0`); zero top gap switches on the "zero gap" variants | 0.28 mm (P1S 0.28, A2L 0.24 → 0.28) | 0.24 → 0.28 mm |
| z_top_layers, z_bottom_layers | round(gap / lh) | 1, 1 | 1, 1 |
| z_delta (tip offset) | z_top_layers + 1 | 2 | 2 |
| roof layers R | `support_interface_top_layers` | 3 | 3 |
| floor layers | B: bottom layers if > 0; O: bottom layers, a negative value meaning "same as top" | 3 | 3 |
| support_bottom_layers | (floor·lh + lh/2) div lh when top and bottom interfaces are on | 3 | 3 |
| D (tip line spacing input) | B `tree_support_branch_distance`, O `tree_support_branch_distance_organic` | **5 mm** | **1 mm** |
| top rate | B fixed 30 %, O `tree_support_top_rate` | 30 | 30 |
| connect_length | trunc(w_s·100/top_rate + max(2·r_min − w_s, 0)) | 177 997 (1.78 mm) | 1 779 997 (1.78 mm) |
| roof line distance (unused by the roof fill, see §5.5) | trunc(`support_interface_spacing`) + w_s | 91 998 | 919 999 |
| base pattern spacing | `support_base_pattern_spacing` (drawing, not tips) | 2.5 mm | 2.5 mm |
| wall count | max(1, `tree_support_wall_count`) | 1 (P1S 0, A2L −1) | 1 (0) |
| minimum roof area | fixed 1 mm² | 1 mm² | 1 mm² |
| minimum support area | fixed 0 (so small-area removal in §5.3 is off) | 0 | 0 |
| support offset | fixed 0 (no config key feeds it) | 0 | 0 |
| outline resolution | trunc(`resolution`) | **2 999** | 30 000 |
| force_tip_to_roof | R > 0 and (soluble/zero-gap interface, or π·r_min² > 1 mm²) | false (0.503 mm²) | false |
| extra_outset (§5.3) | max(0, r_min − w_s div 2) + (use xy_min ? w_s div 2 : 0) | 40 000 | 400 000 |
| outset step (§5.3) | w_s div 8 (because acc + 2·w_s > r_min always here) | 5 249 | 52 499 |
| tip dedup cell | (r_min + 1) div 10 | 4 000 (40 µm) | 40 000 (40 µm) |
| max_overhang_insert_lag | computed but only used by disabled code | — | — |

Support-fill parameters used by the tip line generator (§5.5):

| quantity | formula | value |
|---|---|---|
| flow spacing s | f32: width − height·(1 − π/4) | 0.3599115 mm (all three) |
| base fill density | f32: s / (s + D) | B 0.0671488, O 0.2646580 |
| base line spacing | trunc(s / density) on the flavour grid ≈ s + D | B 535 991 units (5.35991 mm), O 1 359 911 nm |
| interface density | min(1, s / (`support_interface_spacing` + s)) | 0.4185448 |
| roof line spacing | trunc(s / f32 interface density) ≈ s + 0.5 | B 85 991 units, O 859 911 nm |
| base / roof pattern | sparse "support base" lines unless density > 0.95 or sheath (wall count > 0); honeycomb base pattern (and B lightning) select other generators | support base / support base |
| base / interface angle | `support_angle`, `support_angle` + 90° | 0°, 90° |

Unsupported values return `Unsupported(<setting>)` (design §5.7, with the full list): honeycomb or
lightning base pattern, soluble or zero-gap interfaces, raft layers > 0,
`independent_support_layer_height = 1`, support on model (`support_on_build_plate_only = 0`) until the
to-model clearance volumes are specified.

---

## 5. Tip seeding (B1 `tips`)

Sources: O `Support/TreeSupport3D.cpp:332-1460`, B `:329-1340` (identical behaviour except the
tip-state bookkeeping for roof recovery, irrelevant while `force_tip_to_roof` is false), S
`:341-1460`, R `:379-1505` (identical to O).

### 5.1 Which overhangs seed tips

Let `H_k` be the per-layer polygons of §3.8 and n the layer count. Tips for the overhang of layer k
are inserted at layer `m = k − z_delta = k − 2`; only m in [1, n − 3] is used, so overhangs on layers
0, 1, 2 are ignored. Each such k with a non-empty H_k is one **task**; tasks run in parallel
(§5.7). The highest support layer is (highest k with an overhang) − z_top_layers.

### 5.2 Clearance queries used here (defined in the next spec)

- `Coll(r, l)` with xy_min: the section outlines of layers l − z_bottom..l grown by r + xy_min, plus
  the outlines of the z_top layers above grown by r + xy_min·(1 − (i − ½)/z_top) for i = 1
  (½·xy_min with one top layer), mitred limit 1.2.
- `AvoidFast(r, l)`, `AvoidSafe(r, l)`: positions from which a branch of radius r at layer l can no
  longer reach the bed moving at most `move_fast` per layer (the "safe" variant also refuses holes
  in the model it could get trapped in). At l = 0 both equal Coll(r, 0).
- Tip queries use r = radius(0) = r_min and the xy_min variant.

The **forbidden region** of layer l is `F_l = AvoidFast(r_min, l) ⊕mi(1.2) 5 µm` (support on the
bed only; with support on the model it would be `Coll(r_min, l)` instead). The 5 µm keeps points on
the boundary out. The requested mitre limit 1.2 acts as 2 (§0): convex corners with an interior angle
between 60° and ≈ 113° are mitred instead of squared off (a few µm at this offset).

### 5.3 One task (overhang of layer k, insertion layer m = k − 2)

Definitions (P = union of H_k; F_m from §5.2; C0 = Coll(0, m), the bare outlines grown by xy_min;
G(X, r) = (X ⊕rd(0.01 mm) r, Douglas-Peucker at 0.015 mm) ∖ C0, a single round grow for every r used here,
which stays below the 0.63 mm step limit of the stepped offset, O `:792-848`):

```text
reg_0  = P ∖ F_m                                        tips may stand here
rest_0 = (P ∖ (reg_0 ⊕mi(1.2) w_s/2)) ∩ F_m             forbidden overhang farther than half a line from reg_0
a_j    = j · σ,  σ = w_s div 8,  j = 1 … n,  n = max{ j : j·σ < extra_outset }      (n = 7 for the goats, both grids)
rest_j = rest_{j−1} ∖ G(reg_{j−1}, 1.5·a_j)
reg_j  = reg_{j−1} ∪ (G(rest_j, a_j) ∖ F_m)
roofs  = P ∖ F_m                                        (when roof layers R > 0)
fringe = reg_n ∖ (roofs ⊕ 10 units)                      10 nm O/S/R, 100 nm B (safety offset)
```

In words: in round j, forbidden overhang within 1.5·a_j of the region where tips may already stand is
dropped (the branches of those tips will cover it), and the rest is offered back as a fringe grown by a_j
and cut by F_m, so tips can stand at its edge. Once rest is empty the recurrence is stationary (upstream
stops early; the result is the same). After the n rounds, `rest_n` is overhang that gets no tip.

Sampling (§5.4): every connected part of `roofs` as a roof candidate, then every connected part of `fringe`
as a plain candidate. Within each set the parts come in the boolean library's output order. This order
reaches the tip list (§5.7) and therefore the merge order (02 §4.2). `polygon_ops` must return components in
one documented, deterministic order. Whether it matches Clipper-1's output order is part of the backend
question that WP S4 reports on (design §5.2).

With roofs enabled (all projects), almost everything goes through the roof candidate path; the fringe is
the thin band added by the outset rounds.

### 5.4 Sampling one area A at insertion layer m

**Roof stack** (only for roof candidates, R = 3; O `:1133-1186`). Define the shrinking sequence

```text
A_0 = A,   A_{t+1} = A_t ∖ (F_{m−t−1} ⊕mi(1.2) 5 µm)
```

The **stack depth** T is the largest t ≤ min(R, m) such that area(A_s) ≥ 1 mm² for every 1 ≤ s ≤ t
(T = 0 if area(A_1) < 1 mm²). Then:

- **Roofs:** A_s is placed at layer m − s for s = 0 … T − 1: the contact layer at m (s = 0), interface layers
  below (base-interface layers only exist with soluble interfaces).
- **Bed:** if T = m (the stack reaches layer 1 with every area ≥ 1 mm², possible only when m ≤ R), the
  remaining area A_m becomes a roof on layer 0 and no tips are made. This takes precedence over the
  T = R row below.
- **Tip lines:**

  | stack result | line source | classified at | resample spacing |
  |---|---|---|---|
  | T = 0 (the area one layer down is < 1 mm²) | base pattern on A at spacing s + D | layer m | **r_min / 2 = 0.2 mm** ("continuous tips") |
  | 0 < T < R (cut short by the area test) | roof-pattern lines of A_{T−1} (§5.5) | layer m − T + 1; keep only points still outside the avoidance of layer m − T (§5.6) | connect_length |
  | T = R (stack complete) | roof pattern on A_T (§5.5) | layer m − T | connect_length |
  | not a roof candidate | base pattern on A | layer m | connect_length |

- **Truncation:** if T > 0 and the stack yields no line point at all, the stack is shortened to end at the
  first roof A_s whose own roof lines are empty.

**Small-area fallback.** Let `need = clamp(trunc(perimeter(A) / connect_length), 1, 3)`. If the
lines hold `need` points or fewer in total, the outline of the area is sampled instead, with
spacing connect_length and at least `need` points. Upstream intends to sample an inset of the
outline (A ⊖ w_s/2.2, mitred 1.2) but its test almost never selects the inset, so in practice the
outline of A itself is sampled (O `Support/TreeSupport3D.cpp:1196-1214`; a flag in our code). Areas
thinner than s (0.36 mm) produce no fill lines at all and always fall back.

**Classification and insertion** (§5.6 for the point status): points are classified at their layer
(table above); a point inside both avoidances splits its line and is dropped. Each surviving point
becomes a tip record on that layer `m − t` (t = the roof depth it serves) with:

| field (ours) | value |
|---|---|
| position | the point |
| initial area | circle of radius 10 µm around it: a **45-gon** on the 1 nm grid (O/S/R), a **15-gon** on B's 10 nm grid (02 §1.3) |
| reaches bed safely | status "outside AvoidSafe" |
| supports a roof | t > 0 |
| hold still for | R − t layers (0 if R = 0) |
| keep round (no ovalisation) | r_min < 3·w_s, no roof tip layers, and the point's line has more than 5 points |
| distance to top, radius steps, foot growth | 0 |

So the overhang of layer k inserts tips on layers k − 2, k − 3, k − 4 or k − 5 (t = 0..3).

### 5.5 Line generator (support-base pattern)

Behaviour of the sparse support fill as used for tips (O `Fill/FillRectilinear.cpp:3610-3631`,
`:2920-2956`, `Fill/FillBase.cpp:275-321`, `:1352-1580`, `:1834-1953`, `:2173-2640`; B and S
identical; R identical except §5.5.4):

1. **Direction.** Base lines run parallel to the direction at angle `support_angle` from +X (goat:
   along X). Roof lines always run at 135° (lines `x + y = const`) whatever the layer parity:
   upstream's alternation expression is parsed so that the condition is "interface angle + parity",
   which is non-zero for any `support_angle` ≠ −90° (O `Support/TreeSupport3D.cpp:707-709`, B `:700-702`).
2. **Positions.** Each connected part is filled separately. Lines are spaced `spacing` (§4) apart
   perpendicular to their direction and pass through the centre of the part's outer-contour bbox
   (in the object frame); i.e. the perpendicular coordinate of every line is congruent to that of
   the centre modulo the spacing. The part is first shrunk by s/2 (mitred, limit 3), and each line
   contributes its chords inside the shrunk part (holes split chords). Coordinates are rotated in
   f64 and rounded to the grid unit.
3. **Connection of chords into polylines** (our `join_chords`; upstream `connect_base_support`,
   O `Fill/FillBase.cpp:2173-2640`). Stated as the result, not as upstream's step order.

   *Objects.* `ls` = line spacing, s = flow spacing. Every chord end is attached to its nearest point on
   the shrunk boundary. An **arc** is the boundary stretch between two consecutive chord ends on one
   ring. Its **score** is its bulge: the largest distance of the arc from the straight line through its
   two ends, or outside the band between them. An arc is **trimmed** where it passes within 0.8·s of any
   chord (ignoring the last 1.7·s at each chord end), and where a short stretch lies entirely within
   s/2 of its own chord. A **chain** is a maximal sequence of chords joined by selected arcs; selecting
   an arc never closes a cycle of chords.

   *Output.* A set of polylines of four kinds:

   | kind | definition |
   |---|---|
   | free ring | a boundary ring with no chord end, longer than 0.15·s + ls/2, shortened by 0.15·s at its end |
   | excursion piece | for an arc trimmed at both ends that leaves a chord and returns to it or to its neighbour: the parts of the untrimmed arc inside the band between the two chord lines (each side narrowed by s/2), pieces shorter than ls/2 dropped |
   | bulge piece | every arc with score > **3·ls**, and every cap (an arc from a chord back to its own other end) with score > 0.5·ls, even if its ends are used elsewhere; each shortened by s/2 at its start |
   | chain | chords joined by the selected arcs (below), each chord end first extended along the boundary where the boundary stays within 0.33·ls of the chord's line for more than 0.5·ls beyond the end (the longer direction wins) |

   *Arc selection.* Each chord end takes at most one arc. Arcs are selected class by class in the
   precedence order below. Inside a class the stated order applies, and an arc is taken only if both
   ends are still free and it closes no cycle:

   | precedence | class | condition | order inside the class |
   |---|---|---|---|
   | 1 | same-line arcs | both ends on one chord line; a trimmed one only if its untrimmed part is longer than 1.3·ls | boundary order |
   | 2 | contrast arcs | at a chord end whose two arcs have scores with the larger ≥ **1.3·ls**, the smaller ≤ **2·ls**, and a difference ≥ 25 % of the larger: each of the two arcs scoring above 1.3·ls | decreasing score |
   | 3 | zig-zag | the arc joining each chord to its successor along the boundary | chords left to right |
   | 4 | caps | arcs from a chord back to its own other end | boundary order |
   | 5 | T-joints | any remaining arc that joins two different chains, or scores above **2·ls** | decreasing score |

   The bold thresholds (1.3·ls and 2·ls in class 2, 2·ls in class 5, 3·ls for bulge pieces) are the
   three process-static values of item 4. The 1.3·ls of class 1 and the 0.5·ls of caps are evaluated per call.

   *Output order* (it becomes the tip order inside one part, §5.6): free rings, excursion pieces and bulge
   pieces in creation order, then the chains in chord order. All vertices lie on chords or on the boundary.

   *Test vectors.* The order and selection must be pinned by recorded (polygon, spacing, angle) →
   polylines vectors. WP S3 records them from a source where the connector's output is printed directly:
   the base layers of a synthetic normal-support slice in R with density < 0.95 and no sheath (pattern
   "support base"; to be confirmed per flavour), or the private instrumented build of review §5.3. Until
   then the connector's fidelity is part of δ.
4. **Process-static thresholds (nondeterminism S9).** In B 02.08.02.61, O 2.4.2 and S 2.4.0 the
   three arc thresholds of steps 6, 9 and 10 (1.3, 2 and 3 × ls) are initialised **once per
   process** from the line spacing of the first call ever made (the 1.3·ls of step 5 and the
   0.5·ls of step 10 are not static). In a slicer CLI run that first call is whichever task
   reaches the fill first: a roof fill (spacing 0.86 mm) or a base fill (5.36 mm B, 1.36 mm O/S).
   In a GUI session it is the first support fill of the session, possibly another project. R
   2.5.0-rc removed the `static` with exactly this comment (R `Fill/FillBase.cpp:2696-2701`; static in O `:2464-2466`, B `:2367-2369`, S `:2456-2458`).
   Realizations: two families per flavour (thresholds from roof spacing or from base spacing), plus
   the session-history family in the GUI. The ensemble enumerates both families as members (axis
   `arc_thresholds ∈ {roof, base}`, design §5.4); R has one. Which family a given slice lands in is
   unknown (open question Q2); the GUI session effect is design Q20.

### 5.6 Resampling a polyline and classifying points

**Resample(polyline, d, need)** (O `:498-650`):

- d is raised to at least 0.1 mm.
- If the polyline is shorter than 2d and `need` ≤ 1: one point at half its length, measured from
  its first vertex (for a closed outline this depends on the ring's start vertex; Q3).
- Otherwise: for a closed polyline, rotate it so that it starts at the first vertex of the farthest
  vertex pair (exhaustive search, first strict maximum) and remember the other vertex. Start the
  result with the first vertex; add the remembered vertex (last vertex for an open polyline) if
  `need` > 1 or it is farther than d from the start. Then walk: from the current point, find the
  first later vertex at least d away (Euclidean) and the intersection of the circle of radius d
  around the current point with the segment ending at that vertex (on the far side of the foot
  point; segments shorter than 0.01 mm use their midpoint if it is within 0.01 mm of the circle).
  Accept the candidate if it is at least d from every chosen point; otherwise move the walk to the
  candidate and look again at distance max(d − nearest chosen distance, 0.1 mm). If the walk gets
  stuck on the same point it enlarges the distance by d, at most twice.
- If fewer than `need` points result, repeat with d × 0.9 while d ≥ 0.1 mm.
- Output order (it becomes the tip order): the start vertex, then the far vertex if it was added,
  then the walk points in walk order. Each input polyline gives one output polyline, in input
  order.

**Point status** at layer l (O `:382-470`): outside AvoidSafe(r_min, l) → "bed, safe"; else outside
AvoidFast(r_min, l) → "bed"; else invalid (dropped; with support on the model, further classes
exist). Membership uses even-odd over the polygon list with points on a boundary counted as inside.
"Still valid one layer down" re-tests the point against the avoidance of the layer below of the kind
matching its status.

### 5.7 Insertion order and dedup (nondeterminism S1)

- **List content.** The tip list of layer m is the tips of every task writing it, in one interleaving
  that keeps each task's own order. A tip is dropped when its **dedup cell** already holds an earlier
  tip of the same layer list. Cell = (trunc(x / c), trunc(y / c)) in object-frame grid units with
  c = (r_min + 1) div 10: 4 000 units in B, 40 000 nm in O/S/R, i.e. 40 µm in both. Truncation toward
  zero makes the cells that straddle an axis twice as wide. The list order is the input of the merge stage
  (02 §4.2), so order is part of the result.
- **Order within a task** is deterministic: roof parts first, then fringe parts, each in the component
  order of §5.3; within a part, line by line, point by point (§5.6).
- **Writers.** A layer list m receives tips from the tasks k = m + 2 … m + 5 (roof depths 0..3). Upstream
  runs the tasks in a parallel loop with dynamic splitting and appends under a lock, so with several
  threads the writers interleave at **single-tip granularity** in an order set by thread timing; a dedup
  collision across tasks keeps whichever tip came first.
- **Canonical order** (ours): tasks in ascending k, each task's tips in full before the next, which is the
  one-thread order. It is the order of every core ensemble member (design §5.4).
- **Realizations:** any interleaving of the ≤ 4 writer sequences per layer list, plus the choice of
  survivor when two writers hit the same cell. Without roofs (R = 0) every list has one writer and S1
  disappears.
- **Status: unproven as a source of printed variation.** R contains the same race (R
  `Support/TreeSupport3D.cpp:1023-1026` inside the parallel loop at `:1371`) yet produced one realization
  in 30 rotation-0 observations, full plates included (M §2.4, §5.3). Seeded interleavings stay an
  optional ensemble axis (design §3.2, §5.4) until the discriminating experiment (design §11, C5) shows
  that they matter.

### 5.8 Output contract to the descent stage

Per support layer m (0 ≤ m < highest support layer): an ordered list of tip records (§5.4 fields)
and the roof polygons per layer and role (contact, interface). Everything is in the object frame,
integers on the flavour grid (B 10 nm, O/S/R 1 nm).

---

## 6. Nondeterminism in this front part

| id | source | flavours | realizations | our handling |
|---|---|---|---|---|
| S1 | tip insertion order and first-come dedup (§5.7) | B O S R | interleavings of ≤ 4 writer tasks per layer | canonical order in every core member; seeded interleavings an optional axis, unproven (R deterministic, design §3.2) |
| S3 | detection guard: clock or > 100 islands, racy shared flags (§3.9) | B O S R | "tripped from layer j" with any processed-before set | island trigger computed exactly (goat ≤ 40 islands per layer, review appendix A); if it fires, `Unsupported` until the switch-layer axis exists (design §5.7); clock trigger reported as a risk |
| S7 | f32 / libm (tan, sqrt, f32 heights, f32 midpoints, f32 densities) | all | rare flips of discrete tests (sharp-tail emission, dedup cell, line count) | emulate f32 where §1-§5 say so; `libm` for transcendental functions |
| S9 | process-static arc thresholds in the support-base line connector (§5.5.4) | B O S (fixed in R) | thresholds from roof spacing, from base spacing, or from an earlier slice in the GUI session | core ensemble axis `arc_thresholds ∈ {roof, base}` (design §5.4); GUI session history not modelled (design Q20) |
| S10 (negligible) | process-static closing ε for multi-region elephant-foot restore (§1.5) | B O S R | only differs if objects with different closing radii were sliced earlier in the process | none (all projects 0.049) |
| — | slicer segment order (§1.3), slab segment order (§2.6) | all | different loop start vertices, same geometry | start-dependent steps (Douglas-Peucker, half-length point §5.6) inherit it: below 3 µm; Q3 |

Not sources here: the per-layer parallel loops of §1, §2 and §3.2-§3.3 (each writes its own layer),
the sequential §3.4-§3.7, the cluster order (sequential), the boolean results themselves.

## 7. Version differences in this front part

| behaviour | B 02.08.02.61 | O 2.4.2 | S 2.4.0 | R 2.5.0-rc |
|---|---|---|---|---|
| variable layer height with tree(auto)/default | Tree Hybrid | error | error | error |
| ZAA slice_z offset | no | yes (off) | no | yes (off) |
| micro back-track removal before closing | yes | no | no | no |
| multi-part fast path keeps all disjoint parts | yes | first part only (Q5) | as O | as O |
| elephant-foot layers | layer 0 | `elefant_foot_compensation_layers` | as O | as O |
| extrudable width w | raw `line_width` | absolute vs nozzle 0 | as O | as O |
| paint state ≥ 18 encoding | 15-chain | direct | direct | direct |
| paint projection scale | 100/shrink | via transform | via transform | sliced transform |
| enforcer/blocker projection timing | before detection, tails trimmed | after clustering | as O | as O |
| plain overhang | P minus grown blockers ∪ enforced | P | P | P (+ tilt shift if tilted) |
| layer-0 seed bbox rule | ≤ 6 mm in both | ≤ 6 mm in one | as O | as O |
| seed conditions | + area, bbox, k−2 tests | basic | basic | basic, faster equivalent |
| cantilever | floating ring or > 3 mm | > 3 mm only | as O | as O |
| clusters | all members per layer | first member per layer | as O | as O |
| small overhang | multi-criterion, enforcer rescue | erosion-bbox rule | as O | as O |
| BigFlat / ThinPlate typing | yes | no | no | no |
| critical-only | cantilevers; at enforcer layers whole cantilever/sharp clusters + rest ∩ (E ⊕ 0.8), big flats unioned | cantilevers + enforced + sharp tails | as O | as O |
| bridge removal applies to | everything incl. sharp tails and enforced | cantilevers / overhangs only | as O | as O |
| sharp-tail hand-off grow 0.2 mm | any polygon with the bit | sharp-tail supports only | as O | as O |
| tip seeding | as O | — | as O | as O |
| S9 static thresholds | yes | yes | yes | fixed |

## 8. Open questions

- **Q1. Fidelity target for the line connector (§5.5.3). Answered** (lead, 2026-11-02, design §14
  Q17): a behaviourally faithful connector, because the S9 families exist only through its
  process-static thresholds. A "chords only" generator (tip positions off by up to one line spacing)
  remains a diagnostic. Remaining work: the recorded test vectors of §5.5 item 3.
- **Q2. Which family S9 lands in.** In a single-object CLI run the first fill call is a race; in the
  owner's GUI it depends on session history (normal supports sliced earlier set other values).
  Needs a measurement: slice the same project in R (fixed) and B with one thread and compare tip
  layers' interface polylines, or instrument by comparing roof-only vs base-only projects.
- **Q3. Ring start vertices and component order. Partly answered** (design §5.2, §14 Q6): the backend
  is `i_overlay` behind `polygon_ops`, emulating Clipper-1 semantics where they change discrete
  decisions (mitre floor, arc tolerance, decimation, grid). Ring start vertices (Douglas-Peucker, the
  half-length point) and the component order of boolean results (§5.3) are not guaranteed to match
  Clipper-1. Their effect goes into δ, and WP S4 reports whether it dominates.
- **Q4. Island count guard on the goat. Answered provisionally** (review appendix A, approximate
  shapely reconstruction): at most 40 islands on one layer (P1S, 0° and 225°), 33 (A2L), 17 (U1); no layer
  exceeds 100, so S3 explains neither B's spread nor the 225° supports. A4 must log the exact count; above
  100 the model returns `Unsupported` (design §5.7).
- **Q5. Orca fast path for disjoint parts** (§1.4): verify with a two-cube object in R before
  supporting such models; until then reject objects whose parts have disjoint XY boxes on a layer.
- **Q6. Bridge removal** (§3.7.1): treated as no-op (evidence: U1). Needs a rule or a rejection for
  models with straight bridged perimeters inside cantilevers.
- **Q7. B critical-only asymmetry** (§3.7): at enforcer layers whole cantilever clusters survive, at
  other layers only cantilever islands. Confirm with one B slice of a model with a cantilever cluster
  crossing the painted Z range (the goat's belly band, z ≈ 13 mm, may be such a case).
- **Q8. O first-member-per-layer cluster quirk** (§3.5): affects only `support_critical_regions_only
  = 0`; confirm with one R slice (two islands of one layer under one cluster).
- **Q9. B supports at z 26-41.6 mm at 225° only** (M §2.2: P1S and A2L, 5 of 5 runs, bands 13-20,
  15-25 mm² per band, object frame x −14..15, y 3..14). Candidate mechanisms in this part: the
  axis-aligned bbox rules of sharp-tail growth (§3.4) and small-overhang removal (§3.6), and B's
  floating-ring cantilever rule (§3.2 step 5). None is shown to be the cause. Needed before
  `--rotations` admits classes beyond {0°, 180°} (design Q21; the 5° sweep is design §11 C9).
- **Q10. Corpus fixtures for §3.** Two corpus findings are predicted by this part and are cheap
  acceptance tests: `arch` (flat 30 mm bridge) gets no support with critical-only in every flavour
  (M §2.7), and R does not support the mushroom's 24.8° cone (single-layer clusters are small, §3.6),
  while B does (floating-ring cantilever, §3.2 step 5) (M §2.8).

## 9. Validation hooks for the implementation

- A1: 160 layers, z values of §1.2.2 for all three projects (fixture: first/last 3 values).
- Settings: every integer of §4 for both grids (B in 10 nm units, O/S in nm) as fixture values; the
  vertex counts 15/45 (tip seed) and 100/315 (vertical enforcer circle).
- A3: the decode statistics of §2.4 (7907 / 7668 / 239 / depth 6 / 10158 / 2191 / 60.80 mm² /
  3.22 mm² / 0 vertical) and the E_k layer sets of §2.6 (P1S/A2L 31 layers, U1 30 layers).
- A4: per-layer counts of plain, enforced, cantilever and sharp-tail polygons logged; total area of
  §3.8 hand-off per layer. Cross-check: the slicer's roof (contact) layer at k − 2 equals the
  printed "support interface" of the G-code on that layer within line-width rounding.
- B1: number of tips per layer, canonical order hash; with roofs off (test fixture, `R = 0`) the
  tip lists must be order-independent of the ensemble axis S1.
- Experiments behind this file (single-object, sequential): `/tmp/spec01/exp/` (U1
  `max_bridge_length = 0`; U1 and P1S without paint); helper scripts `/tmp/spec01/*.py`.

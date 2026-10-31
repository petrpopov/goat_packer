# Support landing rule, final G-code check, tower placement, progress (2026-10-31)

## Why

P1S goat job `73c31e3e` packed 37; the user hand-placed 40 and sliced it
(`40koz_p1s_04_028_28h22m.gcode.3mf`). Measured on that G-code:

- BambuStudio builds tree supports per object and never trims them around neighbours: all 40
  goats carry the same 617 mm² first-layer support, 44.7×42.6 mm.
- In 15 neighbour pairs one goat's support lands on the bed under the other goat's silhouette
  (up to 21 mm²). Printed lines never collide: the closest support-to-foreign-body gap is
  ~0.2 mm on layer 1, and no support comes within 1.5 mm (centre lines) of a foreign body on
  any higher layer.
- The packer forbade exactly that (patch vs whole silhouette), so it spaced goats ~1 mm wider
  than needed.
- The realized support patch differs between slices of the same project: 565 mm² (Mac probe),
  643 mm² (server probe), 657 mm² (Mac 42-plate), 617 mm² (GUI 40-plate). Within one plate all
  objects share one realization. A one-object probe is therefore only an estimate of what the
  final plate prints.
- The effort slider is inert: with locked orientation the search runs `clamp(attempts, 1, 64)`
  variants, i.e. 64 for every level; unlocked, 4800 and 12000 differ by 54 vs 64 variants.

## Rules

1. **Support landing.** `Footprint::support_landing_loops` = the probe's first-layer body (all
   non-support extrusions). Pair rule for objects A, B:
   - full patch∪silhouette of A must not touch the landing zone of B (and vice versa);
   - silhouettes keep `clearance` from each other (3D bodies);
   - support/support overlap stays governed by `support_overlap_mm`.
   Empty landing loops = the old two-zone rule (body = silhouette), byte-identical layouts for
   support-free footprints.
2. **Tower keepout is a body-only exclusion** (`PackOptions::body_exclusions`): the model's
   silhouette bbox (+clearance) must clear it, supports may enter it (they are purged with the
   tower area anyway; the user's slice is fine with it). Fixed exclusions (bed corner, purge
   lines) keep the full-extent bbox rule.
3. **Tower position search** (whenever centre-out does not place the tower): try the source tower position and the four usable-bed corners (shifted keepout, clear of
   fixed keepouts), pack each, keep the max count (ties: source position, then corner order).
   A moved tower uses the existing `wipe_tower_x/y` relocation and final-slice corner check.
   Corner keepouts stay 6 mm from the bed edge (a flush tower failed BambuStudio's "G-code
   outside of the printable area" check) and reserve 30 % of the probe tower's depth above it
   for growth; a final slice that still fails with a corner tower repacks once at the source.
4. **Final G-code check** (`gcode_collisions`): every layer, support extrusions of object A vs
   non-support extrusions of object B ≠ A; a violation is centre-line distance below the sum of
   half line widths (lines touch or overlap). On violations the packer learns the realized
   first-layer support of the final slice (union over objects, placement-relative, plus the
   probe patch), repacks and reslices; a second failure falls back to the strict two-zone rule
   (landing = silhouette). Remaining violations are reported (`support_collisions` in the
   verification report) and logged, not fatal: the strict rule is what shipped before.
   Window plates are checked and reported the same way.
5. **No multi-probe.** Slicing the probe several times would not predict the final plate:
   within one plate every object shares one support realization, which differs from any
   probe. Learning from the final slice (rule 4) replaces it.
6. **Effort slider removed** from the UI; the backend sends the CLI default effort.

## Progress protocol (packer → webapp)

With `GOAT_LOG_FORMAT=json` the packer prints, besides the human lines:

- `{"event":"stage","stage":ID,"message":TEXT,"expected_s":SECONDS|null,"objects":N|null}`
  at the start of each stage. Stage ids: `probe_slice`, `packing`, `assembling`,
  `final_slice`, `gcode_check`, `repacking`, `window_slice`, `writing`.
  `expected_s` for slices is a rough estimate: the first plate ≈ probe × (1 + 0.5·(N−1))
  (server: probe 22 s → 37 objects 317 s; M-series Mac: 4 s → 43 objects 136 s), every later
  plate scales the measured time of the previous one by its object count.
- `{"event":"slicer_progress","elapsed_ms":N}` every 2 s while a slicer runs.

The webapp maps stages to bands of the bar, interpolates inside a slicing stage by
elapsed/expected (never reaching the band's end before the stage ends), shows the stage list
with the current one highlighted, a Russian description, elapsed/remaining time, and never
shows raw `slicer_output` JSON as the status message.

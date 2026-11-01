# Sample slice, layer zones and a deferred final slice

Date: 2026-11-01. Status: agreed with the user, in implementation.

## Problem

The packer reserves supports measured on a one-object probe sliced from the source project, then
slices the whole plate (7–8 min on P1S) to find out what the slicer really printed. On a 43-goat
P1S plate that slice found supports touching neighbours (first-layer bases up to 0.43 mm wider than
the probe's, interfaces under the ears 2–2.5 mm past the silhouette at z ≈ 23 mm). Every contact
meant a repack of the whole plate under a stricter zone plus another full slice: 43 → 42 → 38/39
objects and +8 min per round. A user's hand-made plate holds 40, the packer's first layout 43.

## Evidence (2026-11-01, BambuStudio, P1S goat)

- Within one slice every goat prints identical supports regardless of position and neighbours:
  five goats of a 43-goat plate compared in their own frame, IoU 1.000, max deviation 0.1 mm
  (the comparison raster), 616 mm² first layer, 857 mm² over all layers.
- Three goats cut out of that plate at the same positions and sliced alone (8 s instead of
  ~7 min) print the same supports: IoU 0.997, max deviation 0.5 mm.
- The one-object probe from the source project differs from both by more (different project
  form: source vs assembled plate).

## Design

1. **Sample slice.** After the first pack, the assembled plate is cut to a few objects (the one
   nearest the bed centre and its nearest neighbours, `SAMPLE_OBJECTS = 3`) and sliced. The
   sample's G-code is the support evidence for every object on the plate.
2. **Layer zones.** The sample's lines of one object, moved into the footprint frame, are grouped
   into height bands (`LAYER_BAND_MM`); per band a support zone (support lines) and a body zone
   (every other line, exterior rings). `Footprint::layer_zones` carries them. Each band adds the
   mask pairs `support(A) vs body(B)` and `body(A) vs support(B)` to `zone_pairs`, dilated so the
   pair keeps `SAMPLE_DRIFT_MM` (sample-to-plate deviation, 0.5 mm) apart. The no-fit maps, the
   grid, `validate_result` and the free-spot search all see them through the same predicate.
   The first-layer patch is learned from the sample as well (`learn_support_patch`).
   Empty `layer_zones` = today's behaviour, byte-identical.
3. **Repack once with the sample's zones** (seconds, no slice). The tower contest and the tower
   free-spot move run as before.
4. **Final slice is optional** (`--final-slice` / `--no-final-slice`, CLI default on). Without it
   the packer writes the 3MF and a `pack_state.json` (plan + layout) and stops. The full slice,
   the layer-by-layer G-code check and the tower checks run later in `--finish` mode from that
   state, with the existing nudge/repack fallbacks. The time window needs slices and keeps the
   final slice on.
5. **Webapp.** A checkbox «Сразу нарезать весь стол» on the pack form (off by default). Off: the
   job ends with the 3MF and the layout; the result shows «Нарезать стол», which starts a follow-up
   job (finish mode on the job's state) and replaces the 3MF if the slice changed it.

## Out of scope

- Global relaxation of a layout: with the real supports in the packing rule the layout is free of
  sample-visible contacts by construction; the nudge stays as the fallback after a final slice.
- Multi-object probes for printers without supports (no support zones: nothing to learn).

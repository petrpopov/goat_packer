# Centre-out plates, A2L heat zones, tower next to the objects — design

Date: 2026-10-26
Status: roadmap sub-project 4; details decided autonomously (see "Decisions"). Builds on the
time window (`2026-10-26-time-model-and-window-design.md`).

## Goal

With a time window the plate holds fewer objects than the bed can. Those objects must sit
around the bed centre (A2L heat-soaks only the first-layer bbox; Bambu Studio recommends the
centre) and the prime tower must sit next to them (shorter head travel per change), not in the
source project's corner.

## Key observation

Any subset of a valid layout is valid. So "centre-out" needs no new search: pack the bed as
today, then order placements by distance from the centre and keep the N nearest. The time
window's prefix/shrink logic (sub-project 3) works unchanged on that order and always drops the
farthest object first.

## Design

1. **Profile**: optional `heat_zones: [{ "name": "inner", "rect": [x0,y0,x1,y1] }, ...]`,
   smallest first. A2L (system key `bed_heat_soak_area`): inner `[102, 93.4, 228, 226.6]`,
   middle `[60, 49, 270, 271]`, full `[0, 0, 330, 320]`. Other printers: absent.
   Centre = centre of the first zone, or of the usable bed when no zones.
2. **Activation**: only with `--time-window-hours`; additionally `--no-centre` opts out.
   Without a window nothing changes (corner sweep, tower where the source put it).
3. **Tower relocation** (when the probe found a tower): shift = centre − probe keepout centre.
   Write `wipe_tower_x[0] += dx`, `wipe_tower_y[0] += dy` into the project settings used by the
   slices (the CLI copy and the output 3MF); other array entries untouched. The reserved keepout
   is the probe keepout shifted by the same vector, clamped inside the usable bed (clamp shrinks
   the shift, never the keepout). Position-independent tower size makes a re-probe unnecessary;
   the final slice verifies it: if the measured tower centre is more than 5 mm from the target,
   error `slicer ignored the relocated wipe tower (wipe_tower_x/y)` — this also catches the
   `different_settings_to_system` reset trap.
   This is the one documented exception to "project_settings.config copied byte-for-byte";
   the comment at the edit site says why.
4. **Packing**: unchanged search on the full usable bed with the relocated tower keepout →
   Nmax layout, sliced as the time model's second sample.
5. **Order**: sort placements by the distance from the centre to each placement's world
   footprint bbox centre (ties: original index), before the window's prefix logic.
6. **Zone report**: bbox of the kept placements' world loops ∪ tower keepout; report the name
   of the smallest zone containing it (`heat_zone`, null when none / no zones) and print it.

## Decisions

- Tower goes to the centre itself (objects ring around it), as the user did by hand in
  koza-mini (tower X 150–178, Y 144–172). Shortest travel to every object in the ring.
- No re-pack per zone: a dense full-bed layout's nearest-N subset is already a compact disc;
  re-packing per zone would cost extra search time for a marginal gain.
- Max-count plates (no window) keep today's behaviour: centre tower fragments free space and
  could cost objects.

## Testing

Unit (no slicer): wipe tower settings patch (arrays of strings, only entry 0, other entries
kept), shift + clamp of a keepout, centre-out ordering, smallest-zone selection, profile parse.
Manual: A2L multi-colour source, 12 h window → tower in the centre in the final G-code, kept
objects around it, `heat_zone` reported.

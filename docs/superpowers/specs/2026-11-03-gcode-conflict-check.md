# The slicers' "G-code path conflicts" check (2026-11-03)

Written from reading the builds for behaviour only (BambuStudio 02.08, OrcaSlicer 2.4.2 and 2.5.0-rc,
Snapmaker Orca 2.4.0); nothing here is translated. Trigger: the owner's U1 plate of 56 goats in the
Snapmaker Orca 2.4.0 GUI showed "Serious warning: Conflicts of G-code paths have been found at layer 0,
z = 0.00 mm (WipeTower <-> koza_45mm_150k_v2.obj)", while the CLI slice of the same plate passed our
`gui_checks`. The corner goat's first-layer support base crossed the tower's layer-0 brim.

## The rule, common to all builds

- It runs at the end of slicing a plate (`Print::process`) and the GUI shows the first hit as a serious
  warning in the preview; the printed G-code is not changed. Nothing in the project settings disables
  it. The CLI flag `--no-check` skips it (the GUI never sets it). Snapmaker Orca 2.4.0 also skips it
  when any object has an adaptive layer-height profile; the other two builds do not.
- Each owner (an object, or the tower) is a bucket of layers. Objects bring, per layer, the
  perimeters and fills of their regions (every role) and the `support_fills` of their support layers.
  A tree support's first-layer base and its brim-like expansion are part of `support_fills`, so they
  count. The object's own brim lives on the print, not the object, and the skirt too: neither counts.
  Only the first instance of an object is read (Orca 2.4.2+ reads all; our plates have one).
- Per step the buckets' current layers are compared (layers are matched by their bottom z).
- Two straight centre-line segments of different owners conflict when they intersect and the
  intersection lies farther than 0.01 mm from every end of both. A pair where both are support
  material uses a 100 mm threshold, i.e. never conflicts; the tower's lines have role "wipe tower",
  so tower against support uses 0.01 mm. Orca and Bambu add a rule for collinear segments overlapping
  by more than 0.01 mm (measured along X); Snapmaker Orca 2.4.0 lacks it.
- Line widths play no part: only centre lines are compared.

## Which tower lines are recorded (the difference between builds)

| build | tower lines compared |
|---|---|
| BambuStudio 02.08 | the outer wall of every layer, plus every brim loop (layer 0) and chamfer loop (above) |
| Snapmaker Orca 2.4.0 (tower type 2, the only one on non-Bambu machines) | brim loops only: layer 0 and the chamfer loops of later layers. The wall is not recorded |
| OrcaSlicer 2.4.2 / 2.5.0-rc, tower type 1 (Bambu machines) | like BambuStudio |
| OrcaSlicer 2.4.2 / 2.5.0-rc, tower type 2 (default on other machines, the U1 on the server) | nothing: the fake tower stays empty, the tower never conflicts |

This is why vanilla Orca's CLI slice (the server's U1 verification) cannot see what the Snapmaker GUI
sees. Our G-code reader marks Orca's `WIPE_TOWER_BRIM_START/END` loops as the tower brim, which are
exactly Snapmaker's recorded lines.

## What the packer does with it

- `Flavour::conflict_tower_lines` holds the table above (`TowerLines::{None, BrimLoops, All}`);
  `GuiRules::tower_lines` makes `gui_checks::check_plate` compare the same lines, and a support line
  over a compared tower line is a conflict like any other (the old "support over the tower brim is
  allowed" exemption is gone).
- The tower's layer-0 brim zone is `ZoneClass::TowerBrimChecked` (supports keep out, gap 0: touching
  extents cannot cross centre lines) where the table is not `None`, else `ZoneClass::TowerBrim`
  (supports may overlap). Object brims stay allowed in both.
- The chamfer rings of layers >= 1 are tower body (design 4.2, Q18), z-ranged, in every flavour.
- Evidence: on `/tmp/goat_m2/u1` (U1 56 plate) the Snapmaker rule finds exactly one conflict, the
  corner goat `id:55`, layer 1 (z = 0.00), as the owner saw; the P1S and A2L plates of that run have
  none under the strict Bambu rule either.
- Not modelled: Orca tower type 1 selected by the project (`wipe_tower_type`); it would read as
  Bambu. The owner's earlier remark that supports and tower brims may overlap holds for Orca type 2
  only (nothing is recorded); Bambu and Snapmaker both flag a crossing.

## The Spiral-lift warning is not a setting check (2026-11-03, Snapmaker Orca 2.4.0 and 2.4.1)

"Model too close to bed boundary. Disable spiral lifting or keep at least 3.5mm gap to avoid collision."
(serious warning) is a purely geometric canvas check (`GLVolumeCollection::check_outside_state`): for a
rectangular bed, for every printable object fully inside the plate, the object's world bounding box must be
at least 3.5 mm from each edge of the plate shape's bbox (`printable_area`), strict `<`. It never reads
`z_hop_types`, `filament_z_hop_types` or any per-object key, so setting Normal lift anywhere does not clear it.
Only the margin does: on the owner's U1 56 plate the lowest mesh bbox is 4.18 mm from the y = 1 edge, so a
shift of 0.7 mm or more toward -Y raises it; 0.65 mm is the largest safe shift (3.53 mm).

## Why the conflict survives the Snapmaker tower model

The model matches the GUI's G-code (checked on the 40- and 48-goat Snapmaker GUI plates: brim loops 4.91 mm
per side from the wall centre line, anchor = wall bbox minimum, depth 16.1 at width 20 and 19.7 at width 16,
cone 3.52 mm at layer 1 shrinking 0.44 mm per layer). The GUI clamps the anchor to
`plate max - width - 1 - brim` and `plate max - depth - 1 - brim`; the packer's (244.5, 248.9) is exactly
that corner. What is left is the support realization: vanilla CLI keeps the nearest goat's first-layer
support 1.66 mm (centre line to the outermost brim loop) from the tower, while the GUI's supports exceed
the vanilla union by 0.7 to 1.4 mm (4.1 mm in one 40-goat plate; `delta.md` section 2). A +1.5 mm reserve on
the checked brim zone costs a goat (56 -> 55 and the tower leaves the corner), so the packer does not apply
it; the webapp tells the user instead. The hand-made 48-goat GUI plate that printed fine has 8 support/brim
line crossings of its own at layer 0.

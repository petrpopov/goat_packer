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

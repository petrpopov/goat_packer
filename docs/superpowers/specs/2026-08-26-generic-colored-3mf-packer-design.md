# Generic Colored 3MF Packer Design

## Goal

Pack repeated copies of a painted source 3MF object using the STL only for collision geometry, while retaining the exact colored mesh, printer/process settings, prime tower, brim, purge paths, bed exclusions, and edge margins for a selected printer.

## Input Contract

One run accepts:

- a printer-specific source 3MF containing exactly one painted object;
- a matching STL whose z-min footprint drives contour packing;
- a printer-profile JSON selected by CLI;
- an output 3MF path.

The tool never converts profiles across printers. The source 3MF remains the source of all paint, filament mapping, materials, process, and machine settings.

## Pipeline

1. Read printer geometry and fixed keepouts from the profile: printable rectangle/polygon, edge margin, bed exclusions, purge segments, slicer CLI.
2. Slice a temporary single-object copy of the source 3MF. For CLI-incompatible U1 start G-code, blank only `machine_start_gcode` in the temporary probe copy.
3. Parse extrusion moves between slicer prime/wipe-tower markers to determine the actual tower/brim XY bounds; expand that boundary by half extrusion width and the requested contour clearance.
4. Run the existing STL contour packer with all profile and discovered keepouts.
5. Clone the source 3MF's top-level object/component and associated model settings for every packed transform. This keeps the shared mesh XML unchanged, including per-triangle `paint_color` attributes.
6. Slice the generated project and parse the tower again. Validate all packed STL footprints against the final tower bounds, printer keepouts, and one another. If the final tower moves/grows, add its bounds and rerun once; fail rather than export an unsafe plate if it remains unstable.

## Prime Tower Bounds

The algorithm uses actual slicer output rather than only `prime_tower_width` because the tower's required length depends on painting/tool changes. It detects `WIPE_TOWER_START`/`END` and `WIPE_TOWER_BRIM_START`/`END`, tracks G90/G91 motion modes, keeps extruding XY moves, and expands the final centerline bbox by the configured clearance.

## Profiles

`profiles/p1s.json` and `profiles/snapmaker_u1.json` define printer-specific geometry and slicers. P1S preserves the 18×28 mm blind corner and its purge U-line. U1 preserves the 3.5 mm edge margin and its X85–185/Y1 purge line.

## Verification

Tests cover parsing/writing profile JSON, tower-marker G-code parsing (absolute and relative moves), colored object clone integrity, and re-pack loop decisions. Final validation checks project settings byte identity, resource/model-settings reference integrity, paint-color preservation, physical contour gaps, all keepouts, prime tower bounds, archive CRC, and slicer loading.

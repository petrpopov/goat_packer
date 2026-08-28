# Colored 3MF Packer — Tower Clearance Fix, Validation, Slicer Detection, Goat 3 Completion

## Context

Builds on `2026-08-26-generic-colored-3mf-packer-design.md`, which specified and implemented
`colored_3mf_packer`, `colored_packer.rs`, and the printer-profile JSON files. That implementation
was never successfully run end-to-end: the final slice of a fully packed Goat 3 / P1S plate fails
with a real BambuStudio CLI error:

```
gcode path conflicts found between goat3.obj and goat3.obj
plate 1: found slicing result conflict!
```

This document specifies the fix, verified empirically against the real BambuStudio CLI, plus three
smaller follow-ups needed to finish the original goal: pack painted Goat 3 for both printers.

## Root Cause (empirically confirmed)

Using the real BambuStudio CLI (`/Applications/BambuStudio.app/Contents/MacOS/BambuStudio --slice 1`)
against hand-built test 3MFs, the following were ruled out:

- **Object cloning mode.** The same 19-placement layout was built both with
  `--reuse-source-object-instances` and with the default (distinct-object, shared external mesh
  component) mode. Both fail identically. The cloning strategy is not the cause.
- **General inter-object clearance.** Two plain objects placed 0.7 mm–20 mm apart, away from the
  prime tower, all sliced successfully. The packer's 0.7 mm default clearance is not too tight in
  general.
- **Object footprint accuracy.** The embedded painted mesh (`3D/Objects/object_2.model`) and
  `goat3.stl` have identical bounding boxes and face counts; the STL-driven contour packer sees the
  same geometry the slicer will print.

What actually reproduces the failure: a **single** object placed near the (fixed, from
`wipe_tower_x`/`wipe_tower_y` in `project_settings.config`) prime tower position conflicts with the
tower even though its rasterized, rotation-aware polygon is >0.6 mm outside the keepout rectangle
that `colored_packer::parse_prime_tower_bounds` currently reserves. Bisection against the real CLI
(angle 0°, sweeping distance beyond the current keepout edge) found:

| extra margin beyond current keepout | result |
|---|---|
| 0.6 mm | conflict |
| 0.8 mm | OK |

The current keepout only expands the measured tower centerline by `extrusion_line_width / 2 +
clearance_mm` (≈0.95 mm with defaults). The real minimum needed, measured from the centerline, is
≈1.65–1.75 mm — the current formula under-reserves by roughly 0.7–1.5 mm. This is not explained by
`prime_tower_brim_width` (3 mm) directly; it is simply an empirically measured slicer-side safety
margin around the tower that the current formula doesn't include.

## Changes

### 1. Tower keepout safety margin (`goat_packer/src/colored_packer.rs`)

Add a fixed extra pad to `parse_prime_tower_bounds`'s expansion, independent of the user's
`--clearance` value (which governs plain inter-object spacing and is already known to work at
0.7 mm):

```rust
const TOWER_EXTRA_SAFETY_MM: f64 = 1.5;
...
let expansion = max_width * 0.5 + clearance_mm + TOWER_EXTRA_SAFETY_MM;
```

Total expansion becomes ≈2.45 mm from the measured centerline with default settings — comfortably
above the empirically found ≈1.75 mm minimum, leaving headroom for other rotation angles and minor
per-print variance in tower geometry. This constant is a pragmatic, tested default for this printer
generation (BambuStudio/OrcaSlicer on P1S/U1); it is not derived from a documented slicer parameter,
so it may need revisiting if a future printer profile shows a different threshold.

### 2. Validation safety net (`goat_packer/src/bin/colored_3mf_packer.rs`)

`pack()` currently returns straight from `pack_footprint` without calling
`goat_packer::validate_result`, unlike `main.rs`. Add the same validation call after packing, before
assembly, so a packing regression fails fast with a clear Rust-side error instead of surfacing as an
opaque slicer conflict later.

### 3. Slicer-kind detection from the source 3MF (`goat_packer/src/colored_packer.rs`)

`Application` metadata in the 3MF is not a reliable signal: OrcaSlicer-family tools (including
Snapmaker's) still emit `Application: BambuStudio-x.y.z` because they're forks. Instead, derive the
slicer kind from `printer_model` (already read and checked by `validate_source_printer`) via a small
substring table:

```rust
const SLICER_KIND_BY_PRINTER_SUBSTRING: &[(&str, &str)] = &[
    ("Bambu Lab", "bambu"),
    ("Snapmaker", "orca"),
];
```

Behavior:
- If `printer_model` matches a known substring and the detected kind **differs** from
  `profile.slicer_kind`, fail with a clear error (wrong profile for this source project) rather than
  silently slicing with the wrong CLI.
- If `printer_model` matches no known substring (future printer), fall back to `profile.slicer_kind`
  and print an informational note.
- This check is folded into (or runs alongside) the existing `validate_source_printer` call in
  `colored_3mf_packer.rs`, which already reads `printer_model` — no new file reads needed.

### 4. Goat 3 deliverables

Once 1–3 land, run the real pipeline for both printers and produce the originally planned outputs:

- `Goat 3/p1s/goat3_p1s_colored_contour_packed.3mf` + `Goat 3/p1s/*_verification.json`
- `Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.3mf` + `Goat 3/snap/*_verification.json`

Verification reuses `verify_outputs.py` (geometry/metadata checks) plus a real slicer CLI run
confirming the final 3MF slices without conflict, matching the original plan's Task 5 checklist
(tower/brim, purge line, edge margin, model gap, paint attributes, settings identity, archive XML,
CLI load/slice).

## Testing

- Extend `tests/prime_tower.rs` with a case asserting the new safety margin is present in the
  computed keepout.
- Extend `tests/colored_pack_loop.rs` (or a new test) to assert `validate_result` is invoked (e.g. by
  injecting a deliberately invalid layout and expecting an error rather than proceeding to assembly).
- New unit tests for `detect_slicer_kind_from_printer_model`: Bambu match, Snapmaker match, unknown
  printer falls back, mismatch against profile errors.
- Manual/integration verification: the real BambuStudio and OrcaSlicer CLIs, run against the actual
  Goat 3 source projects for both printers, must complete without a "slicing result conflict" error.

## Out of Scope

- Making the tower safety margin configurable per-profile (YAGNI until a second printer/tower
  combination demonstrates a different threshold).
- Any change to the packing algorithm itself (`pack_footprint`, `drop_attempt`, `greedy_attempt`) —
  the root cause is entirely in the tower-keepout sizing, not the packer's search or collision logic.

## Addendum: what real end-to-end runs actually needed (post-implementation)

Running the fixed tool against the real Goat 3 P1S and U1 projects surfaced three more real issues
beyond the tower margin, found empirically with the real BambuStudio/OrcaSlicer CLIs:

1. **`--reuse-source-object-instances` is unreliable at real density.** Side-by-side on an identical
   20-object Goat 3 P1S layout, the default clone mode (N distinct `<object>` resources sharing one
   external mesh component — the same pattern already proven by the uncolored Goat 4 packer) sliced
   cleanly while `--reuse-source-object-instances` (one shared object, N build items) reproduced
   `gcode path conflicts found between goat3.obj and goat3.obj`. `colored_3mf_packer.rs` no longer
   passes that flag.
2. **BambuStudio's own "gcode path conflicts" check is a false positive on dense multi-color plates
   for this BambuStudio version**, independent of spacing: it persisted at clearances from 0.7mm up
   to 4.0mm, with brim disabled, across 5 different seeds, and never reproduced in any isolated
   pairwise test. BambuStudio's own `--no-check` flag ("Do not run any validity checks, such as
   gcode path conflicts check") skips it; our own `validate_result` and `verify_outputs.py` already
   independently confirm there is no real geometric overlap, so skipping BambuStudio's heuristic is
   safe. `profiles/p1s.json`'s `no_check_for_final_tower_probe` is now `true` (previously only U1
   needed this).
3. **The safety margin needed to be applied to profile keepouts too, not just the tower.** A dense
   pack (16000 attempts) reliably hugs the P1S bed's 18×28mm exclusion corner tightly enough to trip
   BambuStudio's separate "too close to exclusion area" validation (which `--no-check` does *not*
   suppress). `PrinterProfile::fixed_keepouts()` now expands every `exclusion_rectangles` and
   `purge_lines` keepout by the same `SLICER_KEEPOUT_SAFETY_MM` constant used for the tower (renamed
   from `TOWER_EXTRA_SAFETY_MM` since it's no longer tower-specific).
4. **Unrelated CLI bug:** `colored_3mf_packer.rs` passed `--keepout`/`--purge-line` to
   `build_3mf.py` as `--flag value` (two argv tokens). When a keepout rectangle has a negative
   coordinate (true for U1, whose tower sits near bed-local origin), Python's `argparse` misreads
   the negative-looking value as another option and errors with "expected one argument". Fixed by
   switching to the single-token `--flag=value` form.

With all four fixes, both printers pack and slice cleanly at the default `--clearance 0.7`: P1S
packs 20 objects (0.65mm minimum exact gap), U1 packs 22 objects (0.66mm minimum exact gap), both
independently verified by `verify_outputs.py` with all checks passing.

# goat_packer

*[Русская версия](README.ru.md)*

Packs a mesh into a slicer 3MF project, keeping the original print settings byte-for-byte.

Given a 3MF containing one object, it fits as many copies onto the plate as it can — nesting the
model's whole-model XY silhouette (outer contour and holes), not its bounding box — and writes a
new 3MF with the same profile, filaments, and paint data. Fixed bed exclusions, purge lines, and
the prime tower are reserved automatically.

A second tool schedules a day of printing across several printers.

## Build

```bash
cargo build --release
cargo test --release --all
```

Tests are self-contained: real regression contours/STL and small synthetic 3MFs live in
`tests/fixtures/`. No sibling asset directories, symlinks, network, or installed slicer are
needed for the Rust test suite. With dependencies cached, use `cargo test --offline --release --all`.

The packer canonicalizes XY rotation/translation and searches deterministic no-fit maps,
including mixed orientations. Thread count and seed do not change the result. Geometry is
rasterized at `--resolution`; this remains a heuristic, not a proof of globally optimal packing.
Probe footprints with orientation-dependent supports stay locked to their original orientation.
See [algorithm, limitations and reproducible benchmarks](docs/packing-search.md).
The packing core is local-only; the existing colored-3MF workflow still uses the local slicer
configured in the printer profile for probing/final slicing.

## Packing

```bash
./target/release/colored_3mf_packer --source in.3mf --profile profiles/p1s.json --output out.3mf
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--source` | — | source 3MF with exactly one object |
| `--profile` | — | printer profile JSON (`profiles/`) |
| `--output` | — | packed 3MF to write |
| `--work-dir` | `<output>.work` | scratch dir for slicer runs |
| `--clearance` | `0.7` | gap between objects, mm |
| `--resolution` | `0.25` | raster cell size, mm |
| `--angle-step` | `2.0` | rotation step, degrees |
| `--attempts` | `16000` | deterministic search effort; stops when the finite strategy portfolio is exhausted |
| `--candidates` | `2400` | compatibility flag; free positions are now scanned exhaustively |
| `--stall` | `0` | compatibility flag; no scheduling-dependent early stopping |
| `--seed` | `20270831` | compatibility/report field; results no longer depend on randomness |
| `--support-overlap-mm` | unlimited | how far printed supports (patch past the body) of different objects may overlap, mm; `0` = supports may touch but not overlap. Bodies still keep `--clearance` from anything foreign; no-op without detected supports |
| `--nozzle-clean` | off | inject periodic nozzle wipes into Layer Change G-code: `global,every=10` (whole print) or `from-z=6.6,every=1` (only from a height) |
| `--time-window-hours` | off | keep only as many objects as finish within N hours (corrected time, see below) |
| `--ams-correction` | off | correct the slicer estimate by the profile's real filament change duration (`filament_change_seconds`); no-op on tool changers |
| `--change-seconds` | profile | real duration of one filament change, s (0–600); implies `--ams-correction` |
| `--time-margin-pct` | `10` | safety margin on the corrected time, % (0–50) |
| `--prep-min` | `0` | fixed per-plate preparation time (bed swap, loading), minutes |
| `--no-centre` | off | with a time window: keep the corner layout and the source tower position instead of gathering objects and tower around the plate centre |

Writes `<output>.verification.json` with the object count and prime tower bounds, plus
`supports_detected` (the probe patch reaches past the silhouette, orientation locked) and the
echoed `support_overlap_mm`, and a `time` block (measured samples, chosen count, corrected
seconds; with a window also `heat_zone` and the tower relocation).

### Time window

Corrected time = `(slicer estimate + changes × (real change − slicer's assumed change) + prep) ×
(1 + margin)`. With `--time-window-hours` the packer packs the full plate as usual, slices it,
aims at the window along the line through the 1-object probe and the full plate, slices that
prefix, and shrinks (along the line through the measured window plate and the full plate) while
it is over; at most 4 extra slices. The kept objects are the ones closest to the plate centre
(the smallest profile heat zone, A2L), and the prime tower moves there too (`wipe_tower_x/y`);
`--no-centre` turns both off. The run fails if even one object exceeds the window, if the
slicer ignored the tower move, or if the moved tower would land on a printer keep-out area.

Profiles with `blank_start_gcode_for_cli` (Snapmaker U1) slice with the start G-code blanked, so
their estimates and the window time exclude the start sequence (homing, heating, purge); cover it
with `--prep-min`.

An independent exact-geometry check on any output: `./target/release/verify_outputs <stl>
<placements.json> <packed.3mf>` (minimum gap, edge margin, purge-line gap, keepouts, transforms,
ZIP CRC). It measures gaps between the objects' mesh silhouettes, not `--support-overlap-mm`
(printed supports): support overlap is verified only by the packer's own `validate_result`.

## Nozzle clean

`--nozzle-clean from-z=6.6` injects a periodic nozzle-wipe macro into the packed project's
Layer Change G-code: every layer from a given height (`from-z=H[,every=N]`) or every N layers
for the whole print (`global[,every=N]`; `every` defaults to 10 for `global`, 1 for `from-z`).
The wipe moves come from the printer profile (`nozzle_clean_gcode`): raw G1 moves to the back
chute on P1/X1 (no G150.x firmware macros there), native `G150.3` + `G150.1` on A2/P2/H2.
A profile without that field (Snapmaker) fails with an explicit error. Useful wherever white
prints on top of red: on Goat 3 that zone is z 6.6–8.6 mm, so `from-z=6.6` cleans exactly it.

For projects that did not come out of the packer:

```bash
./target/release/nozzle_clean_inject --source in.3mf --output out.3mf \
  --profile profiles/p1s.json --nozzle-clean from-z=6.6
```

Human-pasteable versions for the Bambu Studio / Orca UI live in `gcode/`.

## Scheduling

```bash
./target/release/daily_scheduler \
  --printer "profile=profiles/p1s.json,source=packed.3mf" \
  --output-schedule schedule.csv
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--printer` | — | repeatable; `profile=`, `source=` required, plus optional `name=`, `prep-min=`, `prep-sweep=a:b:c`, `plates-3mf=<path>`, `change-s=<s>` (real filament change duration for this printer, 0–600; implies the AMS correction for it) |
| `--output-schedule` | — | required; path to write the resolved per-printer job sequence as CSV |
| `--output-report` | off | optional path for the full benchmarks/sensitivity/schedule JSON dump |
| `--turnaround-min` | `10` | dead time between jobs |
| `--reuse-benchmarks` | off | reuse prior slice timings from `--output-report` instead of re-slicing |
| `--ams-correction` | off | add `changes × (real − slicer-assumed change time)` to every benchmark, using the profile's `filament_change_seconds` (or `change-s=`); reused rows are re-corrected, but reports written before change counts existed carry none (a note says so; re-benchmark) |

It slices each batch size to measure real print times (in a scratch temp dir that's cleaned up
automatically), then optimizes a 10:00→10:00 cycle: day jobs plus one overnight job.

Give a `--printer` a `plates-3mf=<path>` to also get that printer's resolved schedule as one
multi-plate 3MF: one `<plate>` per distinct batch size in the schedule, each trimmed from `source`
by dropping objects from one edge of the existing dense layout (same convention as the internal
per-batch benchmarking) rather than re-packing from scratch.

## AMS calibration

Measures the real duration of one filament change on your printer, for `--change-seconds` /
`change-s=`.

```bash
./target/release/ams_calibration --template my_project.3mf --profile profiles/bambu_a2l.json \
  --variant normal --out-dir calib/
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--template` | — | any project 3MF for that printer with at least two filaments; the test inherits its printer, process, filaments and AMS type |
| `--profile` | — | printer profile JSON (`profiles/`) |
| `--variant` | — | `normal` (10 changes, about ±12 s) or `perfectionist` (30 changes, about ±4 s) |
| `--out-dir` | — | where to write the results |

Writes `calib_A.3mf` (a 20 × 20 mm cube in filament 1, no changes), `calib_B.3mf` (the same cube
with exactly that many changes), `instructions.md` (Russian, for the operator) and
`calibration.json` (`variant`, `changes_expected`, `changes_sliced`, `estimate_a_s`,
`estimate_b_s`, `slicer_change_s`); `slicing/` keeps the slicer runs. Both projects are sliced
locally to check the change count and that the prime tower clears the cube. Print A and B,
note start/finish clock times, then `extra = ((fact_B − est_B) − (fact_A − est_A)) / changes`
and the real change time is `extra + slicer_change_s`.

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

Writes `<output>.verification.json` with the object count and prime tower bounds, plus
`supports_detected` (the probe patch reaches past the silhouette, orientation locked) and the
echoed `support_overlap_mm`.

An independent exact-geometry check on any output: `./target/release/verify_outputs <stl>
<placements.json> <packed.3mf>` (minimum gap, edge margin, purge-line gap, keepouts, transforms,
ZIP CRC).

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
| `--printer` | — | repeatable; `profile=`, `source=` required, plus optional `name=`, `prep-min=`, `prep-sweep=a:b:c`, `plates-3mf=<path>` |
| `--output-schedule` | — | required; path to write the resolved per-printer job sequence as CSV |
| `--output-report` | off | optional path for the full benchmarks/sensitivity/schedule JSON dump |
| `--turnaround-min` | `10` | dead time between jobs |
| `--reuse-benchmarks` | off | reuse prior slice timings from `--output-report` instead of re-slicing |

It slices each batch size to measure real print times (in a scratch temp dir that's cleaned up
automatically), then optimizes a 10:00→10:00 cycle: day jobs plus one overnight job.

Give a `--printer` a `plates-3mf=<path>` to also get that printer's resolved schedule as one
multi-plate 3MF: one `<plate>` per distinct batch size in the schedule, each trimmed from `source`
by dropping objects from one edge of the existing dense layout (same convention as the internal
per-batch benchmarking) rather than re-packing from scratch.

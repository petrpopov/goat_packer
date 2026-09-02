# goat_packer

*[Русская версия](README.ru.md)*

Packs a mesh into a slicer 3MF project, keeping the original print settings byte-for-byte.

Given a 3MF containing one object, it fits as many copies onto the plate as it can — nesting the
model's real flat-bottom silhouette (outer contour and holes), not its bounding box — and writes a
new 3MF with the same profile, filaments, and paint data. Fixed bed exclusions, purge lines, and
the prime tower are reserved automatically.

A second tool schedules a day of printing across several printers.

## Build

```bash
cargo build --release
cargo test --release --all
```

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
| `--attempts` | `16000` | independent randomized restarts, each packing a full layout |
| `--candidates` | `2400` | positions sampled per placed object within one restart (search cost scales with attempts × candidates) |
| `--stall` | `attempts/4` | stop once this many restarts pass without improvement; `0` runs all attempts |
| `--seed` | `20270831` | makes a run reproducible |

Writes `<output>.verification.json` with the object count and prime tower bounds.

An independent exact-geometry check on any output: `./target/release/verify_outputs <stl>
<placements.json> <packed.3mf>` (minimum gap, edge margin, purge-line gap, keepouts, transforms,
ZIP CRC).

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

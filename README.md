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

## Scheduling

```bash
./target/release/daily_scheduler \
  --printer "profile=profiles/p1s.json,source=packed.3mf" \
  --output-dir schedule_output
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--printer` | — | repeatable; `profile=`, `source=` required, plus optional `name=`, `prep-min=`, `prep-sweep=a:b:c` |
| `--output-dir` | `schedule_output` | where JSON/CSV reports go |
| `--turnaround-min` | `10` | dead time between jobs |
| `--reuse-benchmarks` | off | reuse prior slice timings instead of re-slicing |

It slices each batch size to measure real print times, then optimizes a 10:00→10:00 cycle: day jobs
plus one overnight job.

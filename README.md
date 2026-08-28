# Goat 4 contour packer

*[Русская версия](README.ru.md)*

The optimizer packs the **actual flat-bottom silhouette** of `goat4_v2.stl`, not its bounding box.
It preserves the outer loop and the keyring hole, pre-rasterizes rotated copies, expands each copy by
half the requested clearance, and uses bit-parallel collision tests during parallel randomized
contact/drop searches. Count is the primary objective; compactness only breaks ties.

The search is heuristic (irregular 2D nesting is NP-hard), so a larger restart budget can improve a
result. The deterministic seed makes a given command reproducible.

## Build and test

```bash
cd goat_packer
cargo test --release --all
python3 tests/test_build_3mf.py -v
cargo build --release
```

## Pack, slice, and assemble a 3MF

`colored_3mf_packer` is the single packing tool: it slices the source project once to find the
printer's actual prime/wipe tower footprint (if any), packs copies of the object around every fixed
and tower keepout, and reassembles the plate as a new 3MF. Projects with a single material/color have
no prime tower — the probe slice simply won't find one, and packing proceeds without reserving space
for it.

```bash
./target/release/colored_3mf_packer \
  --source ../Goat\ 4/p1s/goat4_single_pla.3mf \
  --profile profiles/p1s.json \
  --output g4_packed.3mf \
  --work-dir ../Goat\ 4/work \
  --clearance 0.7 --attempts 16000 --candidates 2400 --seed 20270831
```

It writes `OUTPUT.verification.json` alongside the packed 3MF with the object count, both probe and
final tower bounds (`null` when the project has no tower), and whether a repack was needed because the
final slice's tower grew past the reserved keepout.

## Schedule a day of production across N printers

`daily_scheduler` takes one or more already-packed multi-object plates (one per printer) and
benchmarks real slice times per batch size (1..max objects), then optimizes a 24-hour print
schedule per printer. Each `--printer` reuses the same `PrinterProfile` JSON the packer uses
(`profiles/p1s.json`, `profiles/snapmaker_u1.json`, ...), so slicer path/kind live in one place:

```bash
./target/release/daily_scheduler \
  --printer "profile=profiles/p1s.json,source=../Goat 4/p1s/goat4_multi_pla.3mf" \
  --printer "profile=profiles/snapmaker_u1.json,source=../Goat 4/snap/goat4_multi_snap_pla.3mf,prep-min=11,prep-sweep=6:8:10:11:12:15" \
  --turnaround-min 10 \
  --output-dir schedule_output
```

`--printer` fields (comma-separated `key=value`, quote the whole flag since paths may contain
spaces):

- `profile=<path>` (required) — a `PrinterProfile` JSON; provides `printer_name`, `slicer_path`,
  `slicer_kind` (`bambu` uses `result.json`, `orca` parses the `.gcode` comment header), and
  whether `machine_start_gcode` needs blanking for the CLI.
- `source=<path>` (required) — the packed plate to benchmark, at its maximum object count.
- `name=<label>` (optional) — overrides `printer_name` for this entry, e.g. to run two plates
  through the same physical printer model under distinct labels.
- `prep-min=<minutes>` (optional, default 0) — extra per-job dead time the slicer doesn't account
  for (filament switching, bed prep).
- `prep-sweep=<a:b:c:...>` (optional, default `[prep-min]`) — alternate preparation minutes to
  sweep in `sensitivity.csv` when that number is uncertain.

Add `--reuse-benchmarks` to reuse the slice timings already in `schedule_output/planner_report.json`
when only `--turnaround-min` or a `prep-sweep` changes — skips re-slicing every batch size.

## Build and verify 3MF (manual/low-level)

```bash
python3 build_3mf.py TEMPLATE.3mf PLACEMENTS.json OUTPUT.3mf --stl ../Goat\ 4/source/goat4_v2.stl
python3 verify_outputs.py ../Goat\ 4/source/goat4_v2.stl PLACEMENTS.json OUTPUT.3mf --minimum-gap 0.5
```

`build_3mf.py` keeps `Metadata/project_settings.config` byte-for-byte, reuses the template's embedded
mesh, synchronizes object/plate metadata, conditionally updates Bambu-only metadata, and regenerates
all referenced preview PNGs.

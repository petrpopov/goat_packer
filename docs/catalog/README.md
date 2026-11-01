# Printer catalog (OrcaSlicer import)

`profiles/*.json` for consumer printers are generated from OrcaSlicer's JSON machine presets and
qualified by slicing in the runner image. Design: `docs/superpowers/specs/2026-11-01-printer-catalog-and-stl-design.md`
(the "CLI spike" appendix has the verified slicer commands); plan: `docs/superpowers/plans/2026-11-01-printer-catalog-tool.md`.

## When to run

When the pinned OrcaSlicer version changes (`deploy/slicers.env`, `ORCA_VERSION`), or to add vendors.
The preset checkout must be at the **same tag** as the Orca AppImage in the runner image.

## Licensing

OrcaSlicer presets, bed models and start G-code are AGPL-3.0. Only **numbers** (printable area,
excluded areas, height, nozzle, model name) and our own traced bed polygon go into `profiles/*.json`,
each with a `source` block. Never commit Orca files (JSON, STL/SVG, G-code), and never commit the
qualification work dirs (`work/<id>/presets/*.json` are flattened Orca presets).

## Hand-made profiles are protected

A profile without `source`, or with `source.manual: true` (P1S, A2L, U1), is never overwritten:
`extract` skips its id, `qualify` refuses to write over it. Do not hand-edit generated profiles
either: fix the tool and re-run.

## 1. Extract drafts (anywhere, no slicer)

```bash
cargo build --release
./target/release/printer_catalog extract --orca <OrcaSlicer checkout>/resources/profiles --tag v2.4.2 \
  --vendor BBL --vendor Prusa --out /tmp/catalog-drafts
```

One `<id>.json` per instantiable rectangular-bed machine (deltas/circular beds are skipped and
counted). `slicer_kind` is `bambu` for Bambu Lab printers (the packer refuses orca for a
`Bambu Lab …` printer_model), `orca` otherwise. `purge_lines` is empty until qualification.

## 2. Qualify (on the x86-64 host with the runner images)

```bash
deploy/catalog-qualify.sh [-t v0.3.0] [-P 2] [-B target/linux/release] \
  /tmp/catalog-drafts <OrcaSlicer checkout>/resources/profiles /tmp/catalog-out
```

Per draft, in a container `--network none --cpus 2 --memory 4g` (stage timeout 40 min):

1. resolve the machine preset (vendor-scoped `inherits`), pick a process (one whose
   `compatible_printers` lists the machine, else a 0.20 mm one named `@<machine>`, else the
   vendor base) and `Generic PLA @System`; write them flattened for the CLI
   (`work/<id>/presets/`);
2. export `probe.3mf` from a generated 20×20×10 mm cube with the Orca CLI (`--arrange 1`: with
   `--arrange 0` the object lands on no plate), then move its build item to the centre of the
   draft's printable area, off its exclusions (Orca's arrange misreads seam-chained
   `bed_exclude_area` lists and put the cube off the bed);
3. slice it with the draft's slicer (`slicer_cli::slice_command`, `--arrange 0`) and detect the
   start-G-code purge (`purge_detect`);
4. run `colored_3mf_packer` on the probe with the draft as profile (dense plate, final slice), then
   `verify_outputs` (min gap 0.7 mm, purge lines, exclusions). If the packer fails, retry once with
   `no_check_for_final_tower_probe`, then once with `blank_start_gcode_for_cli`;
5. on success write `profiles/<id>.json` with `source.qualified: true`, `qualified_on`, `notes`.
   Every draft gets a row in `report.md` (vendor, model, id, result, reason, seconds).

The probe 3MF is always exported by the Orca CLI, so Bambu drafts run `--stage export` in
`goat-packer-runner-orca` and `--stage pack` in `goat-packer-runner-bambu` (both images map
`/opt/slicers/{orca,bambu}/AppRun` to their own slicer). Single-colour probe only: the prime tower
is not qualified (the packer measures it per job from the user's own project).

`-t` defaults to the newest `v*` tag present locally for both runner images. The images ship
`printer_catalog` from the first release built after this tool; for older images (or to test
unreleased fixes of the packer/verifier) pass `-B` with a Linux build of all three binaries:

```bash
docker run --rm -v "$PWD:/src" -w /src -e CARGO_TARGET_DIR=/src/target/linux rust:1.94.0-bookworm \
  cargo build --locked --release --bins          # → target/linux/release/
```

A single draft by hand inside a runner container (what the script runs):

```bash
printer_catalog qualify --draft d.json --orca /orca --work /out/work/<id> --out /out/profiles \
  --report /out/report.md [--stage export|pack|all] [--orca-slicer …] [--slicer …] [--packer …] [--verify …]
```

Exit code 0 = qualified (or export done), 3 = not qualified (row written), other = tool error.

## 3. Commit

Copy `report.md` to `docs/catalog/report-<date>.md`, review the failures by reason, copy the
qualified `profiles/*.json` into `profiles/` and commit both.

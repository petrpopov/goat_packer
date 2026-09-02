# Nozzle-clean layer G-code injection — design

Date: 2026-09-02
Status: analyzed, validated on a real slice (single-object Goat 3 P1S project)

## Problem

Red→white AMS prints contaminate the white detail with residue dragged off the red
under-surface (Goat 3: white top section is z 6.6–8.6 mm, exactly 10 layers at 0.2 mm).
A periodic nozzle wipe at layer change fixes it, but the community snippet
(`clean_nozzle.txt`, MakerWorld "Auto Nozzle Wipe") is PETG-only and built around the
`G150.x` firmware macros that only newer machines (P2S / H2C / H2S / A2) understand.

## What the original snippet does

Inserted as `layer_change_gcode` (Machine G-Code → Layer Change G-Code in
Bambu Studio / Orca). Two modes:

- **Mode A — print by layer**: on layer 1, every 10th layer, and the last layer:
  `M400` (drain queue) → z-hop +0.4 → retract 2 mm → `G150.3` (head to trash bin) →
  three `G150.1` wipes → prime 2 mm → undo z-hop.
- **Mode B — print by object**: once per new object start, dropping the bed to
  `max_layer_z + 3` before travelling to the bin.

Gated on `filament_type == "PETG"` — the hardcode to remove.

## Per-printer facts (verified against the machines' own firmware G-code)

| Family | `G150.3` / `G150.1` | Native wipe mechanism | Evidence |
| --- | --- | --- | --- |
| P1S / P1P / X1 | **no** | raw `G1` moves to the back chute zone X≈60–105, Y≈255–265 | P1S `machine_start_gcode` + `change_filament_gcode` shipped in the project: `G1 X60 Y265`, `G1 X100 F18000 ; first wipe mouth`, X70↔X100 shake passes, bed dropped to `Z{max_layer_z + 3.0}` first |
| A2L | **yes** | `G150.3` (trash bin), `G150.1 F16000` (wipe), `G150` (full clean w/ brush) | A2L `machine_start_gcode` uses all three natively |
| P2S / H2C / H2S | yes | same G150.x macros | MakerWorld snippet + Bambu forum |

Both firmwares use the idiom `G1 Z{max_layer_z + 3.0} F1200` (drop bed / lift gantry
3 mm over the tallest printed part) before travelling to the chute, and `M83` +
relative `E-2`/`E2` retract/prime around the wipe.

## Design

### 1. Profile is the source of truth (`profiles/*.json`)

New optional field, machine-specific wipe *body* (no conditions — those are composed):

```json
"nozzle_clean_gcode": "<multi-line body with {layer_z}/{max_layer_z} placeholders>"
```

- `p1s.json`: raw-move wipe (validated snippet below).
- `bambu_a2l.json`: `G150.3` + 3× `G150.1 F16000`.
- `snapmaker_u1.json`: **absent** → requesting `--nozzle-clean` with that profile is a
  hard error with an explanation ("profile X has no nozzle_clean_gcode; Snapmaker is
  not supported").

P1S validated body (from the machine's own firmware patterns):

```
M400 ; finish all queued moves
G1 Z{max_layer_z + 3.0} F1200 ; drop bed 3mm below the tallest printed part (firmware toolchange idiom)
M83 ; relative extrusion
G1 E-2.0 F1800 ; retract before travel
G1 X70 Y245 F21000 ; head to the back chute area
G1 Y255 F3000
G1 X100 F5000 ; wipe mouth
G1 X70 F15000
G1 X100 F5000 ; wipe mouth
G1 X70 F15000
G1 X60 F15000 ; shake off debris
G1 X80 F15000
G1 E2.0 F1800 ; prime nozzle back to pressure
G1 Z{layer_z} F1200 ; return bed to the new layer height
```

### 2. Modes and composition (`src/nozzle_clean.rs`)

```
--nozzle-clean global,every=10        # every 10th layer from the start (original cadence)
--nozzle-clean from-z=6.6,every=1     # only at/above a height (targeted zone)
```

`every` defaults to 10 in `global` and 1 in `from-z` (a targeted zone wants active
wiping; a whole-print wipe every layer would be waste). Composed wrapper:

```
{if layer_num > 0}                      <- never at print start (start gcode just wiped)
{if (layer_num + 1) % EVERY == 0}       <- only when EVERY > 1
{if layer_z >= H - 0.01}                <- only in from-z mode (epsilon: float layer_z)
<body>
{endif} x N
```

The wrapper is **prepended** to the project's existing `layer_change_gcode`, so the
printer-native tail (`M73 L{layer_num+1}`, `M991`, A2L's `M201`/`M1007`) survives —
the A2L tail differs from P1S, so it must come from the project, not from code.

Height epsilon 0.01 mm: `layer_z` is accumulated float (`0.2 * 33 = 6.6000001…`),
the stock firmware snippets already use `+ 0.001`-style epsilons for the same reason.

### 3. Where it hooks in

- `colored_3mf_packer`: after `assemble_project` writes `--output` (initial pack and
  the single repack), rewrite `Metadata/project_settings.config` → `layer_change_gcode`.
  Probe/final slices then include the wipes automatically (CLI copies are derived from
  the output file), so timings and the tower state machine see the real G-code.
- New tiny binary `nozzle_clean_inject --source in.3mf --output out.3mf --profile p.json
  --nozzle-clean from-z=6.6` for already-existing projects (e.g. `goat3_single_pla.3mf`,
  which is not a packer output).

### 4. Serialization note

Rewriting `layer_change_gcode` forces re-serializing `project_settings.config` through
`serde_json::Value` (BTreeMap): keys get alphabetized. This is already what
`prepare_cli_copy` does to every CLI-sliced copy and Bambu Studio accepts those, so it
is safe; the entry is otherwise carried byte-for-byte.

## Goat 3 specifics (why from-z=6.6)

Mesh paint analysis (both P1S projects share one painted mesh, world z = mesh z + 4.3):

- red `#C6001A` (extruder 0): z 0 → 6.6 mm
- white `#FFFFFF` (extruder 1): z 6.6 → 8.6 mm — the contaminated 2 mm section, 10 layers

`from-z=6.6, every=2` wipes every 2nd layer through the white section: 5 wipes
(z 6.8–8.4), ~15 s added. Everything below 6.6 mm stays untouched, as requested. (An
`every=1` variant — 11 wipes, +28 s — was validated first, then thinned to every 2nd
layer on request; the first white layer needs no extra wipe because the red→white
toolchange at z 6.8 already runs the machine's own full wipe sequence.)

## Human-pasteable snippets

`gcode/` keeps ready-to-paste versions for the Bambu Studio UI (not used by the packer):
`nozzle_clean_p1s.txt`, `nozzle_clean_a2l.txt`, `nozzle_clean_g150_universal.txt`. All
three share the original snippet's cadence — after the first layer, every 10th layer,
and on the last layer — with the PETG gate, M117 and by-object mode removed; only the
wipe body differs per machine. A height-zone gate (`{if layer_z >= ...}`) can be added
per the header comment, or left to the packer's `from-z` mode instead. The P1S variant
is slice-verified: 43-layer project → 6 wipes at z 0.4/2.0/4.0/6.0/8.0/8.6.

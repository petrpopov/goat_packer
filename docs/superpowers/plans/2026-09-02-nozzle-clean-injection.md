# Nozzle-clean injection — implementation plan

Date: 2026-09-02. Design: `2026-09-02-nozzle-clean-injection-design.md`.
Validation status: P1S body + `{if layer_z >= H}` wrapper sliced clean on
`goat3_single_pla.3mf` with the real BambuStudio CLI (43 layers, 11 wipes at z
6.6–8.6, relative-E stream confirmed — zero `M82` in the output).

## Steps

1. `src/nozzle_clean.rs` (new, wired in `src/lib.rs`)
   - `NozzleCleanSpec { Global { every }, FromZ { z_mm, every } }` + `parse()`
     (`global`, `global,every=10`, `from-z=6.6`, `from-z=6.6,every=2`; defaults
     every: global→10, from-z→1; verbose parse errors listing valid forms).
   - `compose_layer_change_gcode(existing, spec, body) -> String`: guard
     `{if layer_num > 0}`, `{if (layer_num + 1) % EVERY == 0}` only when every > 1,
     `{if layer_z >= H-0.01}` only for FromZ; body verbatim; `{endif}` per open if;
     existing `layer_change_gcode` appended after (native M73/M991/M201 tail survives).
   - `inject_nozzle_clean(profile, project, output, spec)`: errors clearly when the
     profile has no `nozzle_clean_gcode`; rewrites only
     `Metadata/project_settings.config` (serde_json::Value → alphabetized keys, same
     as `prepare_cli_copy` already does), all other entries copied byte-for-byte.
2. `src/colored_packer.rs`: `PrinterProfile` += `#[serde(default)]
   nozzle_clean_gcode: Option<String>`.
3. `src/bin/colored_3mf_packer.rs`: `--nozzle-clean <spec>`; inject in-place on
   `args.output` right after each `assemble_project` (initial + single repack), so
   probe/final CLI copies inherit the wipes automatically.
4. `src/bin/nozzle_clean_inject.rs` (new): standalone `--source/--output/--profile/
   --nozzle-clean` for already-existing projects; prints the composed block on success.
5. `profiles/p1s.json`: validated raw-move body. `profiles/bambu_a2l.json`:
   G150.3 + 3× `G150.1 F16000` body (native macros per A2L firmware gcode).
   `snapmaker_u1.json`: untouched (absence = unsupported).
6. `tests/nozzle_clean.rs`: parse round-trips + error cases, compose wrapper shape
   (every=1 collapses, epsilon formatting, existing tail preserved), injection
   round-trip on a synthetic zip, missing-profile-key error.
7. `gcode/` pasteable snippets: `nozzle_clean_p1s.txt`, `nozzle_clean_a2l.txt`,
   `nozzle_clean_g150_universal.txt` (simplified original: PETG gate, M117 and
   by-object mode removed).
8. README.md / README.ru.md: new flag + new binary. CLAUDE.md: one-paragraph layer note.
9. Apply: inject both Goat 3 P1S files (`goat3_single_pla.3mf`,
   `goat3_multi_pla_v3.3mf`) with `from-z=6.6`; slice both to confirm.

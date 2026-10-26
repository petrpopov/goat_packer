# Time model, AMS change correction, packer time window — design

Date: 2026-10-26
Status: approved direction (roadmap `2026-10-26-time-window-a2l-zones-ams-roadmap-design.md`,
sub-projects 2 and 3); details decided autonomously, listed under "Decisions".

## Facts (measured 2026-10-26 on real slices)

- Bambu `result.json` → `sliced_plates[0]`: `total_predication` (s), `filament_change_times`
  (e.g. 227 on a 3-colour A2L plate, 0 on a single-filament one).
- Orca G-code: one `; toolchange #N` comment per tool change; `; machine_load_filament_time =`,
  `; machine_unload_filament_time =` in the config footer.
- What the slicer assumes per change (`slicer_change_s`), read from the source
  `project_settings.config`:
  - Bambu with `default_ams_type` present and its per-type pair > 0:
    1 → `ams_filament_load_time_ams` + `ams_filament_unload_time_ams` (23 + 21),
    2 → `..._ams_lite` (11 + 12), 3 → `..._n3f_s` (18 + 19).
  - Otherwise `machine_load_filament_time` + `machine_unload_filament_time`
    (P1S project: 29 + 28, `default_ams_type` absent, `ams_filament_load_time_ams` = "0").
  - Neither present → 0.
- Real duration of one change with a gen-1 AMS (koza-mini measurements):
  P1S 57 + 34 ≈ 91 s, A2L 23 + 72 ≈ 95 s. Two printers agree within 5 %.

## Model

```
corrected(N) = (estimate(N) + changes(N) × (real_change_s − slicer_change_s) + prep_s)
               × (1 + margin)
```

- `estimate`, `changes` from the slice; `slicer_change_s` from the project; `real_change_s` from
  the profile (`filament_change_seconds`, new optional JSON field; absent = no correction) or a
  CLI/frontend override; correction only when enabled.
- `changes` barely depends on N (changes happen per layer), so the correction lands in the
  fixed term. A negative difference (project already assumes a slower AMS) is applied as is.
- One library module `src/print_time.rs` holds: `slicer_change_seconds(settings_json)`,
  `parse_change_count` for Bambu result / Orca G-code, `TimeCorrection { real_change_s,
  prep_s, margin }`, `corrected_seconds(estimate, changes, slicer_change_s, &TimeCorrection)`,
  and `count_for_window(line, window_s) -> Option<usize>` (affine fit through two samples).
- `SliceEstimate` gains `filament_changes: u64` (Bambu: `filament_change_times`; Orca: count
  of `; toolchange #` lines).

## Packer time window (`colored_3mf_packer`)

New flags (all off by default; without `--time-window-hours` behaviour is unchanged):

- `--time-window-hours H` (f64 > 0)
- `--ams-correction` (enable `real_change_s`), `--change-seconds S` (override profile value;
  implies `--ams-correction`)
- `--time-margin-pct P` (default 10)
- `--prep-min M` (default 0)

Flow with a window:

1. Probe slice (exists) → sample (1, corrected(1)). If corrected(1) > window → error
   `"не укладывается даже 1 объект: <h> ч > окно <H> ч"` (message in English like other
   errors: `even one object exceeds the time window: ...`).
2. Normal packing and final slice of the max plate incl. the tower repack logic (exists) →
   sample (Nmax, corrected(Nmax)). If it fits, done (window does not bind).
3. N = floor of the affine solve through the two samples, clamped to 1..Nmax.
4. Keep the first N placements (packing order = prefix order, same rule as the scheduler's
   `subset_3mf`), rebuild the project, slice `final_window_<N>`, check the tower against the
   reserved keepout (`evaluate_final_tower`), compute corrected(N). A tower that grew past the
   reserved keepout (RepackRequired) is an error only if the grown keepout reaches a kept
   placement — that would void the layout's collision checks; growth that hits nothing is
   accepted, the same rule as on the max plate (no repack on a window plate).
5. Over the window → N -= max(1, ceil(excess / slope)), repeat step 4; at most 4 window slices,
   then error. `slope` is the secant through the window plate just measured and the max plate,
   (corrected(N) − corrected(Nmax)) / (N − Nmax): both were sliced from the assembled output with
   identical settings (relocated tower, nozzle-clean wipe), while the probe is sliced from the
   source with the tower at the source position. The probe line is only the fallback when no
   such pair exists. With `--nozzle-clean` the probe slices a copy of the source with the same
   injection, so step 1 compares like with like. Under the window is accepted (no upward search: the affine fit already aimed at
   the boundary; prefer a guaranteed fit).

Report (`*.verification.json`) gains `time`: `{ window_hours, margin_pct, real_change_s,
slicer_change_s, prep_s, samples: [{count, estimate_s, changes, corrected_s}], chosen_count,
corrected_s }`, also written without a window (single sample of the final plate) so the
frontend can always show the corrected time.

## Scheduler

- Per-printer key `change-s=<seconds>` in `--printer` and global `--ams-correction` (uses the
  profile's `filament_change_seconds` unless `change-s` overrides).
- `BenchmarkRecord` gains `filament_changes` and `change_correction_seconds`
  (`#[serde(default)]`, so `--reuse-benchmarks` of old reports still loads); `job_seconds`
  includes the correction; interpolated records interpolate the corrected value as before.

## Webapp

- Pack form: "Уложиться в окно" (checkbox + hours, default 12), "Коррекция смены прутка (AMS)"
  (checkbox, default value shown from the profile, editable seconds), margin % (default 10).
  Fields: `timeWindowHours`, `amsCorrection`, `changeSeconds`, `timeMarginPct`; validated in
  `JobForms.kt` (window 0.5–240 h, change 0–600 s, margin 0–50 %); passed to the CLI only when
  set. Same fields in `server/Queue.kt`.
- Schedule form: the same AMS checkbox + seconds.
- `ProfileService` exposes `filamentChangeSeconds` (null when absent → checkbox disabled with a
  hint "для этого принтера коррекция не нужна").
- Result view shows the corrected time from the report.

## Profiles

- `bambu_a2l.json`: `"filament_change_seconds": 95`; `p1s.json`: `91`; U1: absent (tool
  changer). Comment the source of the values in the spec, not JSON.

## Decisions

- Two-sample affine fit, max plate as the second sample: it is already sliced today, so the
  window costs one to a few extra slices, never one per object.
- Prefix of the existing layout instead of a re-pack with `max_objects`: a prefix of a valid
  layout is valid, keeps placement order meaningful for sub-project 4 (centre-out order makes
  prefixes compact).
- Margin default 10 %: the calibration test is a lower bound and the per-change cost varies
  ±15 % with interval length.

## Testing

Self-contained unit tests (no slicer): settings parsing for all AMS-type branches, change count
parsing from fixture snippets, corrected time arithmetic, affine N solve (incl. clamp and
"even one does not fit"), the shrink step, serde default for old benchmark records, CLI arg
parsing. Real-slicer check by hand (A2L multi-colour source, window 12 h).

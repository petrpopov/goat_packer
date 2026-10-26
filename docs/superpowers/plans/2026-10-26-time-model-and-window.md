# Time Model and Packer Time Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Corrected print-time model (AMS change correction, margin, prep) shared by packer and scheduler, and an opt-in packer mode that keeps only as many objects as fit a time window.

**Architecture:** New library module `src/print_time.rs` (pure functions, unit-tested); `SliceEstimate` learns the change count; `colored_3mf_packer` samples (1, Nmax), solves N affinely, slices the N-prefix, shrinks if over; scheduler applies the same correction; webapp exposes the flags.

**Tech Stack:** Rust 2024 (serde_json), Kotlin/Spring backend, React frontend.

Spec: `docs/superpowers/specs/2026-10-26-time-model-and-window-design.md` (read it first; every formula and flag name is defined there).

Conventions: `cargo test --release --all` (release required); tests self-contained (no slicer, no network, fixtures under `tests/fixtures/` or inline strings); hand-rolled arg parsing in the `while let Some(option)` shape; comments explain why. Commit after each task with trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never commit `AGENTS.md`, `00000.log`, `result.json`.

---

### Task 1: `src/print_time.rs`

**Files:** Create `src/print_time.rs`, `tests/print_time.rs`; modify `src/lib.rs` (add `pub mod print_time;`).

Public API (exact names, used by later tasks):

```rust
pub struct TimeCorrection { pub real_change_s: Option<f64>, pub prep_s: f64, pub margin: f64 }
/// Seconds the project's slicer settings assume for one filament change (spec "Facts").
pub fn slicer_change_seconds(settings: &serde_json::Value) -> f64;
/// (estimate + changes * (real - slicer) + prep) * (1 + margin); real None => no change term.
pub fn corrected_seconds(estimate_s: f64, changes: u64, slicer_change_s: f64, correction: &TimeCorrection) -> f64;
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TimeSample { pub count: usize, pub estimate_s: f64, pub changes: u64, pub corrected_s: f64 }
pub enum WindowFit { All, Count(usize), NotEvenOne }
/// Affine line through two samples (a.count < b.count); largest N in 1..=b.count with
/// line(N) <= window_s. b fits => All; line(1) > window => NotEvenOne.
pub fn count_for_window(a: TimeSample, b: TimeSample, window_s: f64) -> WindowFit;
/// Next smaller N after a measured overshoot: n - max(1, ceil(excess / slope)), min 1.
pub fn shrink_count(n: usize, corrected_s: f64, window_s: f64, slope_s_per_object: f64) -> usize;
```

Settings values are JSON strings (`"23"`) or arrays of strings (take element 0); parse leniently.

- [ ] Write tests first: `slicer_change_seconds` for ams_type 1/2/3 (A2L-like settings: 44, 23, 37), P1S-like (no `default_ams_type`, `ams_filament_load_time_ams` "0", machine 29/28 → 57), type present but pair 0 → machine fallback, nothing → 0, array-valued keys; `corrected_seconds` with/without real, negative difference, margin; `count_for_window` All / Count / NotEvenOne / clamp; `shrink_count` slope 0 guard and min 1.
- [ ] Run `cargo test --release --test print_time` → fails (module missing).
- [ ] Implement; run → pass. Commit "Add print_time: corrected time model and window solve".

### Task 2: change count in `SliceEstimate`

**Files:** `src/scheduler.rs` (`SliceEstimate`, `parse_bambu_result`, `parse_orca_gcode`), `tests/scheduler_parse.rs`.

- [ ] Add `pub filament_changes: u64` to `SliceEstimate`. Bambu: `filament_change_times` (absent → 0). Orca: number of lines containing `; toolchange #`.
- [ ] Tests: Bambu JSON with `"filament_change_times": 227` → 227; without the key → 0; Orca snippet with 3 `; toolchange #` lines → 3. Fix every construction site of `SliceEstimate` (compiler lists them).
- [ ] `cargo test --release --all` → pass. Commit "SliceEstimate carries the filament change count".

### Task 3: profile field

**Files:** `src/colored_packer.rs` (`PrinterProfile`), `profiles/bambu_a2l.json` (95), `profiles/p1s.json` (91), `tests/colored_profile.rs`.

- [ ] `#[serde(default)] pub filament_change_seconds: Option<f64>` with a doc comment: real full duration of one change with the AMS actually attached (gen-1 AMS measured: P1S 57+34, A2L 23+72); absent = tool changer / no correction. Fix struct literals (e.g. `colored_packer.rs` test helper, `colored_3mf_packer.rs` tests).
- [ ] Tests: A2L 95, P1S 91, U1 None.
- [ ] Commit "Profiles carry the real filament change duration".

### Task 4: packer time window

**Files:** `src/bin/colored_3mf_packer.rs`.

- [ ] Args + parsing: `--time-window-hours`, `--ams-correction`, `--change-seconds` (implies correction), `--time-margin-pct` (default 10), `--prep-min` (default 0). Validate > 0 / ranges like the spec; unit-test the parsing if the file already has an arg-parsing test module (it has `#[cfg(test)]` tests near line 919 — follow them).
- [ ] `slice_project` currently returns the G-code only; make the slice estimate available too (Bambu: parse `result.json` in the output dir with `goat_packer::scheduler::parse_bambu_result`; Orca: `parse_orca_gcode(gcode, count)`), choosing by `profile.slicer_kind`. Keep the existing return for the tower logic (e.g. return a struct `{ gcode, estimate }`).
- [ ] Read `slicer_change_seconds` from the source `Metadata/project_settings.config` once.
- [ ] Window flow exactly as spec "Packer time window" steps 1–5, after the existing tower/repack block. Prefix: `packed.placements.truncate(n)` on a clone, then `build_colored_project` + `slice_project(.., "final_window_<n>")`; `evaluate_final_tower(probe_tower, tower, reserved)` where RepackRequired → error. Print progress lines like the existing ones (`time window 12.0 h: max plate 41 objects = 31.2 h corrected; trying 15`).
- [ ] Report: add `time` (spec) to `ColoredPackReport`; also filled without a window (single sample of the final plate; `chosen_count` = object count).
- [ ] `cargo build --release` warning-free, `cargo test --release --all` pass. Commit "Packer time window: keep the prefix that fits N hours".

### Task 5: scheduler correction

**Files:** `src/bin/daily_scheduler.rs`, `tests/` (existing scheduler tests as pattern).

- [ ] `--ams-correction` global flag; `change-s=` key in `--printer` spec. Effective `real_change_s` = key or profile value when the flag is on.
- [ ] `BenchmarkRecord`: `#[serde(default)] filament_changes: u64`, `#[serde(default)] change_correction_seconds: u64`; `record_from_estimate` computes the correction with `print_time::corrected_seconds` (margin 0, prep handled as today) and adds it to `job_seconds`; `to_timing` must use the corrected seconds so optimisation sees them (pass `slicer_total_seconds + change_correction_seconds` as the slicer part). Read `slicer_change_seconds` from the printer source once.
- [ ] Test: an old report JSON without the new fields deserialises (serde default).
- [ ] Commit "Scheduler applies the AMS change correction".

### Task 6: webapp

**Files:** `webapp/backend/.../dto/JobForms.kt`, `service/PackService.kt`, `job/JobRunner.kt` (PackParams + CLI build; schedule CLI build), `server/Queue.kt`, `service/ProfileService.kt`, `webapp/frontend/src/pages/PackPage.tsx`, `SchedulePage.tsx`, result view showing the report; corresponding backend tests (follow existing ones, e.g. JobRunner / form validation tests).

- [ ] Backend: fields `timeWindowHours` (0.5–240), `amsCorrection` (bool), `changeSeconds` (0–600), `timeMarginPct` (0–50, default 10); map to CLI flags only when set. Schedule: `amsCorrection`, `changeSeconds` → `--ams-correction` and `change-s=` in the printer spec. Profile DTO `filamentChangeSeconds`.
- [ ] Frontend: controls as in spec "Webapp", Russian labels; AMS checkbox disabled with hint when the profile has no value; show corrected time from the report on the result.
- [ ] Run the backend tests and frontend build/tests (see `webapp/README.md` for commands). Commit "Webapp: time window and AMS correction controls".

### Task 7: real-slicer check (manual)

- [ ] A2L multi-colour source (`../../other/koza-mini/archive/a2l_old/koza_a2l_020_bambu_x25.3mf` won't work — needs a single painted object; use `../../other/koza-mini/print/a2l/koza_a2l_bambu_04_024.3mf` only if single-object, else any single-object multi-colour A2L/P1S file found with `object_count_3mf == 1` and `filament_change_times > 0`), `--time-window-hours 12 --ams-correction`: expect a plate whose report `corrected_s <= 12 h` and `chosen_count` < max.

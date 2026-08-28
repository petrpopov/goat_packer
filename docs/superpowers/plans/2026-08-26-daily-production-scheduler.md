# Daily Goat Production Scheduler Implementation Plan

> **For agentic workers:** Use subagent-driven-development or executing-plans only when delegation or a separate execution session adds value. For small work, implement directly. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Benchmark every feasible batch size from the two final faster 3MF projects, optimize a manually serviced 10:00-to-10:00 production cycle, and deliver a Rust planner plus a visually clear Excel schedule.

**Architecture:** A new Rust CLI creates temporary nested 3MF subsets by retaining the first N build objects and matching model-settings records, slices P1S subsets with BambuStudio and U1 subsets with OrcaSlicer, and parses each slicer's native time estimate. A second dynamic-programming stage chooses an ordered sequence of batch sizes whose starts occur between 10:00 and 23:00 and whose final completion is no later than 10:00 next day, maximizing total objects with configurable turnaround and U1 preparation time. The planner emits auditable JSON/CSV; an artifact-tool workbook builder turns those results into input, benchmark, schedule, sensitivity, and dashboard sheets with formulas and charts.

**Tech Stack:** Rust 1.97 (`zip`, `xmltree`, `serde_json`), BambuStudio 02.08 CLI, OrcaSlicer 2.4 CLI, `@oai/artifact-tool` for `.xlsx`, official Snapmaker documentation for calibration assumptions.

---

## Chunk 1: Benchmark and scheduling engine

### Task 1: Add subset-3MF generation

**Files:**
- Modify: `goat_packer/Cargo.toml`
- Create: `goat_packer/src/scheduler.rs`
- Create: `goat_packer/tests/scheduler_subset.rs`

- [ ] Write failing tests that reduce a synthetic 3MF from three objects to two while keeping core resources/build items, model settings, plate instances, and assemble references consistent.
- [ ] Implement ZIP/XML subset rewriting without changing project settings.
- [ ] Verify generated subsets load through the relevant slicer CLI.

### Task 2: Benchmark all batch sizes

**Files:**
- Modify: `goat_packer/src/scheduler.rs`
- Create: `goat_packer/src/bin/daily_scheduler.rs`
- Create: `goat_packer/tests/scheduler_parse.rs`

- [ ] Write failing parsers for Bambu `result.json`/G-code time and Orca `estimated printing time (normal mode)` metadata.
- [ ] Add CLI orchestration for batch sizes 1..18 and 1..19.
- [ ] For U1 estimation only, blank the incompatible start G-code in the temporary benchmark archive because the slicer estimate excludes real printer preparation; preserve every other embedded setting.
- [ ] Save per-count time, filament, source file hash, and slicer version to JSON/CSV.

### Task 3: Optimize the daily sequence

**Files:**
- Modify: `goat_packer/src/scheduler.rs`
- Create: `goat_packer/tests/scheduler_optimize.rs`

- [ ] Write failing DP tests where multiple shorter batches beat repeated full plates.
- [ ] Implement configurable start 10:00, operator cutoff 23:00, horizon 10:00 next day, inter-job turnaround, and per-printer preparation time.
- [ ] Maximize units first; break ties by fewer plate changes, then earlier completion.
- [ ] Produce default and sensitivity scenarios for turnaround and U1 preparation assumptions.

## Chunk 2: Excel analysis and verification

### Task 4: Build the workbook

**Files:**
- Create: `goat_packer/build_schedule_workbook.mjs`
- Create: `outputs/goat4_daily_scheduler/goat4_daily_production_schedule.xlsx`

- [ ] Create `Inputs & Research`, `Batch Benchmarks`, `Optimal Schedule`, `Sensitivity`, and `Dashboard` sheets.
- [ ] Use formulas for derived durations, rates, cumulative units, idle time, and summary cards.
- [ ] Add charts comparing batch size vs time/throughput and the optimized 24-hour timeline.
- [ ] Cite the official Snapmaker FAQ and note that no official source gives a single complete per-print preparation duration; use 11 minutes as the editable default and show 6–15 minute sensitivity.

### Task 5: Validate and deliver

**Files:**
- Create: `goat_packer/schedule_output/benchmark.json`
- Create: `goat_packer/schedule_output/schedule.json`
- Create: `goat_packer/schedule_output/sensitivity.csv`

- [ ] Verify all Rust tests, fmt, and strict clippy.
- [ ] Reconcile full-plate benchmarks to approximately P1S 3:49 and U1 3:44 excluding real preparation.
- [ ] Inspect workbook key ranges and scan formula errors.
- [ ] Render every sheet and visually repair clipping, chart ranges, and schedule readability.
- [ ] Export exactly one final `.xlsx` and report the recommended daily schedules and sensitivity caveat.

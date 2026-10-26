# Slicer `--arrange 0` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every slicer CLI call passes `--arrange 0`, so neither Orca nor Bambu Studio re-arranges the packed plate while slicing.

**Architecture:** One library helper `goat_packer::slicer_cli::slice_command` builds the slicer `Command` (`--arrange 0 [extra] --slice 1 --outputdir <dir> <input>`); both call sites (`colored_3mf_packer` `slice_project`, `daily_scheduler` slice step) use it. Unit-tested via `Command::get_args()`; no slicer is run in tests.

**Tech Stack:** Rust 2024, `std::process::Command`.

Spec: `docs/superpowers/specs/2026-10-26-time-window-a2l-zones-ams-roadmap-design.md`, sub-project 1.

Background: without the flag Orca re-arranges objects on slice (probe moved the goat to bed centre; a dense U1 plate silently sliced 25 of 38). Both `BambuStudio --help` and `OrcaSlicer --help` list `--arrange option  Arrange options: 0-disable, 1-enable, others-auto`, so the flag is passed for both slicer kinds.

---

### Task 1: `slice_command` helper

**Files:**
- Create: `src/slicer_cli.rs`
- Modify: `src/lib.rs` (module list near line 14)
- Test: `tests/slicer_cli.rs`

- [ ] **Step 1: Write the failing test**

`tests/slicer_cli.rs`:

```rust
use std::{ffi::OsStr, path::Path};

use goat_packer::slicer_cli::slice_command;

fn args(command: &std::process::Command) -> Vec<&OsStr> {
    command.get_args().collect()
}

#[test]
fn slice_command_disables_arrange_before_slicing() {
    let command = slice_command(
        Path::new("/opt/slicer"),
        &[],
        Path::new("/work/out"),
        Path::new("/work/plate.3mf"),
    );
    assert_eq!(command.get_program(), "/opt/slicer");
    assert_eq!(
        args(&command),
        ["--arrange", "0", "--slice", "1", "--outputdir", "/work/out", "/work/plate.3mf"]
    );
}

#[test]
fn slice_command_keeps_extra_flags_before_slice() {
    let command = slice_command(
        Path::new("/opt/slicer"),
        &["--no-check"],
        Path::new("/work/out"),
        Path::new("/work/plate.3mf"),
    );
    assert_eq!(
        args(&command),
        ["--arrange", "0", "--no-check", "--slice", "1", "--outputdir", "/work/out", "/work/plate.3mf"]
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --release --test slicer_cli`
Expected: compile error `could not find slicer_cli in goat_packer`.

- [ ] **Step 3: Write minimal implementation**

`src/slicer_cli.rs`:

```rust
//! The one place that builds a slicer CLI invocation, so every slice gets the same guards.
use std::{path::Path, process::Command};

/// `--arrange 0` must never be dropped: without it OrcaSlicer re-arranges the plate while
/// slicing (the probe slice moved the goat to the bed centre, a dense U1 plate silently sliced
/// 25 of 38 objects, 2026-10-11 koza-mini). The packer's layout is then not what gets printed
/// and the final-slice check proves nothing. BambuStudio accepts the same flag
/// (`0-disable, 1-enable, others-auto`), so it is passed for both slicer kinds.
pub fn slice_command(slicer: &Path, extra: &[&str], output_dir: &Path, input: &Path) -> Command {
    let mut command = Command::new(slicer);
    command
        .args(["--arrange", "0"])
        .args(extra)
        .args(["--slice", "1", "--outputdir"])
        .arg(output_dir)
        .arg(input);
    command
}
```

`src/lib.rs`, after `pub mod process_output;`:

```rust
pub mod slicer_cli;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --release --test slicer_cli`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/slicer_cli.rs src/lib.rs tests/slicer_cli.rs
git commit -m "Add slice_command: every slicer call disables arrange"
```

### Task 2: Use the helper in `colored_3mf_packer`

**Files:**
- Modify: `src/bin/colored_3mf_packer.rs:587-596` (`slice_project`)

- [ ] **Step 1: Replace the hand-built command**

Current code:

```rust
    let mut command = Command::new(&profile.slicer_path);
    if profile.no_check_for_final_tower_probe && label.starts_with("final") {
        command.arg("--no-check");
    }
    let output = goat_packer::process_output::bounded_output(command
        .arg("--slice")
        .arg("1")
        .arg("--outputdir")
        .arg(&output_dir)
        .arg(&input))?;
```

Replace with:

```rust
    let extra: &[&str] = if profile.no_check_for_final_tower_probe && label.starts_with("final") {
        &["--no-check"]
    } else {
        &[]
    };
    let mut command = goat_packer::slicer_cli::slice_command(
        Path::new(&profile.slicer_path),
        extra,
        &output_dir,
        &input,
    );
    let output = goat_packer::process_output::bounded_output(&mut command)?;
```

If `Command` / `Path` imports become unused or missing, fix the `use` line at the top of the file (`cargo build` tells which).

- [ ] **Step 2: Build and run the full suite**

Run: `cargo build --release && cargo test --release --all`
Expected: build without warnings about unused imports; all tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/bin/colored_3mf_packer.rs
git commit -m "colored_3mf_packer slices through slice_command (--arrange 0)"
```

### Task 3: Use the helper in `daily_scheduler`

**Files:**
- Modify: `src/bin/daily_scheduler.rs:538-543`

- [ ] **Step 1: Replace the hand-built command**

Current code:

```rust
    let output = goat_packer::process_output::bounded_output(Command::new(slicer)
        .arg("--slice")
        .arg("1")
        .arg("--outputdir")
        .arg(&slice_dir)
        .arg(&subset))?;
```

Replace with:

```rust
    let output = goat_packer::process_output::bounded_output(
        &mut goat_packer::slicer_cli::slice_command(slicer, &[], &slice_dir, &subset),
    )?;
```

Remove `Command` from the `use std::process` import if it becomes unused.

- [ ] **Step 2: Build and run the full suite**

Run: `cargo build --release && cargo test --release --all`
Expected: no warnings, all tests pass.

- [ ] **Step 3: Confirm no hand-built slicer calls remain**

Run: `grep -rn '"--slice"' src`
Expected: only `src/slicer_cli.rs`.

- [ ] **Step 4: Commit**

```bash
git add src/bin/daily_scheduler.rs
git commit -m "daily_scheduler slices through slice_command (--arrange 0)"
```

### Task 4: Real-slicer verification and docs

Not automatable (tests never call a slicer). Run locally with the installed slicers.

**Files:**
- Modify: `CLAUDE.md` (section "Printer profiles / slicer quirks")

- [ ] **Step 1: Orca (U1) plate keeps its layout**

Pick a multi-object U1 source 3MF (e.g. from `../../other/koza-mini/print/`), point a copy of `profiles/snapmaker_u1.json` at the real `OrcaSlicer` binary (not the `orca_wrap.sh` wrapper), and run:

```bash
./target/release/colored_3mf_packer --source <u1.3mf> --profile <u1-local.json> \
  --output /tmp/u1-packed.3mf --work-dir /tmp/u1-work
```

Expected: success. In the final slice G-code (`/tmp/u1-work/final*_slice/*.gcode`) count the object definitions:

```bash
grep -c 'EXCLUDE_OBJECT_DEFINE' /tmp/u1-work/final*_slice/*.gcode
```

Expected: equal to the packed object count in the packer's report (before the fix: fewer, e.g. 25 of 38).

- [ ] **Step 2: Bambu (A2L or P1S) still slices**

Run the same command with an A2L or P1S source and its profile pointed at `BambuStudio`.
Expected: success, same object count as before the change.

- [ ] **Step 3: Document the quirk**

Add to `CLAUDE.md` under "Printer profiles / slicer quirks":

```markdown
- Every slicer call goes through `slicer_cli::slice_command`, which passes `--arrange 0`: without it Orca re-arranges the plate on slice (25 of 38 sliced on U1).
```

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md
git commit -m "Document --arrange 0 slicer invariant"
```

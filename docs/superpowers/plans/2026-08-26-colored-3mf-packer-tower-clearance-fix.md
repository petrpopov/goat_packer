# Colored 3MF Packer Tower Clearance Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Fix the empirically-confirmed prime-tower keepout under-sizing that breaks BambuStudio/OrcaSlicer CLI slicing of packed colored Goat 3 plates, add a missing packing-validation safety net, add 3MF-driven slicer-kind detection, and produce the final packed+verified Goat 3 3MF files for both P1S and U1.

**Architecture:** All fixes land in the existing `goat_packer` Rust crate (`src/colored_packer.rs`, `src/bin/colored_3mf_packer.rs`) with matching test coverage, following the patterns already used by `tests/prime_tower.rs`, `tests/colored_pack_loop.rs`, and `tests/colored_profile.rs`. The final task runs the fixed binary against the real Goat 3 source projects using the real BambuStudio/OrcaSlicer CLIs already installed on this machine.

**Tech Stack:** Rust 1.97 (existing `goat_packer` crate), `cargo test`, real BambuStudio CLI (`/Applications/BambuStudio.app/Contents/MacOS/BambuStudio`) and OrcaSlicer CLI (`/Applications/OrcaSlicer.app/Contents/MacOS/OrcaSlicer`), Python 3 + shapely/trimesh (`verify_outputs.py`).

**Reference:** `docs/superpowers/specs/2026-08-26-colored-3mf-packer-tower-clearance-fix-design.md`

All commands below assume the working directory is `/Users/petrpopov/Downloads/3D-печать/Keychains/Козочки 2027/goat_packer` unless stated otherwise.

---

### Task 1: Tower keepout safety margin

**Files:**
- Modify: `goat_packer/src/colored_packer.rs:147`
- Modify: `goat_packer/tests/prime_tower.rs`

- [x] **Step 1: Update the existing test's expected keepout values and add a new explicit safety-margin test**

Replace the full contents of `goat_packer/tests/prime_tower.rs` with:

```rust
use goat_packer::colored_packer::parse_prime_tower_bounds;

#[test]
fn parses_brim_and_tower_extrusions_with_absolute_and_relative_moves() {
    let gcode = b"
G90
G1 X10 Y10 F1000
; WIPE_TOWER_BRIM_START
; LINE_WIDTH: 0.50
G1 X12 Y10 E0.2
G1 Y20 E0.2
G1 X10 E0.2
; WIPE_TOWER_BRIM_END
G91
; WIPE_TOWER_START
G1 X2 Y0 E0.1
G1 Y2 E0.1
G1 X99 Y99 F1000
; WIPE_TOWER_END
";
    let bounds = parse_prime_tower_bounds(gcode, 0.4, 0.7).unwrap();
    assert_eq!(bounds.centerline.min_x, 10.0);
    assert_eq!(bounds.centerline.min_y, 10.0);
    assert_eq!(bounds.centerline.max_x, 12.0);
    assert_eq!(bounds.centerline.max_y, 22.0);
    assert!((bounds.keepout.min_x - 7.55).abs() < 1e-9);
    assert!((bounds.keepout.max_y - 24.45).abs() < 1e-9);
}

#[test]
fn returns_none_without_prime_tower_markers() {
    assert!(parse_prime_tower_bounds(b"G90\nG1 X10 Y10 E1\n", 0.5, 0.7).is_none());
}

#[test]
fn keepout_adds_a_fixed_safety_margin_beyond_line_width_and_clearance() {
    let gcode = b"
G90
; WIPE_TOWER_START
G1 X0 Y0 E0.1
G1 X10 Y0 E0.1
G1 X10 Y5 E0.1
; WIPE_TOWER_END
";
    let bounds = parse_prime_tower_bounds(gcode, 0.5, 0.7).unwrap();
    assert_eq!(bounds.centerline.min_x, 0.0);
    assert_eq!(bounds.centerline.max_x, 10.0);
    assert_eq!(bounds.centerline.min_y, 0.0);
    assert_eq!(bounds.centerline.max_y, 5.0);
    // 0.5 * 0.5 (half line width) + 0.7 (clearance) + 1.5 (fixed safety margin) = 2.45
    let expected_expansion = 2.45;
    assert!((bounds.keepout.min_x - (0.0 - expected_expansion)).abs() < 1e-9);
    assert!((bounds.keepout.max_x - (10.0 + expected_expansion)).abs() < 1e-9);
    assert!((bounds.keepout.min_y - (0.0 - expected_expansion)).abs() < 1e-9);
    assert!((bounds.keepout.max_y - (5.0 + expected_expansion)).abs() < 1e-9);
}
```

- [x] **Step 2: Run the tests and confirm they fail against the current implementation**

Run: `cargo test --test prime_tower`
Expected: FAIL — `parses_brim_and_tower_extrusions_with_absolute_and_relative_moves` fails because
`bounds.keepout.min_x` is `9.05`, not `7.55`; `keepout_adds_a_fixed_safety_margin_beyond_line_width_and_clearance`
fails for the analogous reason.

- [x] **Step 3: Add the safety margin constant and use it in the expansion formula**

In `goat_packer/src/colored_packer.rs`, find this line (currently line 147):

```rust
    let expansion = max_width * 0.5 + clearance_mm;
```

Replace it with:

```rust
    let expansion = max_width * 0.5 + clearance_mm + TOWER_EXTRA_SAFETY_MM;
```

Add the constant just above the `parse_prime_tower_bounds` function definition (currently starting at
line 73):

```rust
/// Empirically measured extra standoff (mm) BambuStudio/OrcaSlicer's CLI require beyond a plain
/// `line_width/2 + clearance` expansion before it will slice a plate without reporting a
/// "gcode path conflicts" / "found slicing result conflict" error near the prime tower. Confirmed
/// by bisection against the real BambuStudio CLI: 0.6mm extra still conflicts, 0.8mm extra is
/// clean, so this constant leaves comfortable headroom above the ~1.75mm (from centerline) minimum
/// that was measured.
const TOWER_EXTRA_SAFETY_MM: f64 = 1.5;

pub fn parse_prime_tower_bounds(
```

- [x] **Step 4: Run the tests and confirm they pass**

Run: `cargo test --test prime_tower`
Expected: `test result: ok. 3 passed; 0 failed`

- [x] **Step 5: Run the full test suite to confirm nothing else depended on the old margin**

Run: `cargo test --release --all`
Expected: `24 passed` becomes `26 passed` (2 new tests added in this task), 0 failed.

- [x] **Step 6: Commit**

```bash
git add src/colored_packer.rs tests/prime_tower.rs
git commit -m "fix: add empirically-measured safety margin to prime tower keepout"
```

(If this directory is not a git repository, skip this step — there is currently no `.git` here.)

---

### Task 2: `validate_result` safety net in `colored_3mf_packer`

**Files:**
- Modify: `goat_packer/src/bin/colored_3mf_packer.rs:9-16` (imports), `:149-162` (`pack`)
- Modify: `goat_packer/src/bin/colored_3mf_packer.rs` (append inline test module at end of file)

- [x] **Step 1: Write the failing test**

Append this to the end of `goat_packer/src/bin/colored_3mf_packer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, io::Write};

    fn write_binary_stl(path: &Path, triangles: &[[[f32; 3]; 3]]) {
        let mut f = File::create(path).unwrap();
        f.write_all(&[0u8; 80]).unwrap();
        f.write_all(&(triangles.len() as u32).to_le_bytes())
            .unwrap();
        for tri in triangles {
            f.write_all(&[0u8; 12]).unwrap();
            for vertex in tri {
                for value in vertex {
                    f.write_all(&value.to_le_bytes()).unwrap();
                }
            }
            f.write_all(&[0u8; 2]).unwrap();
        }
    }

    fn square_stl(dir: &Path) -> PathBuf {
        let path = dir.join("square.stl");
        let tris = [
            [[0., 0., 0.], [4., 0., 0.], [4., 4., 0.]],
            [[0., 0., 0.], [4., 4., 0.], [0., 4., 0.]],
            [[0., 0., 1.], [4., 0., 1.], [4., 4., 1.]],
        ];
        write_binary_stl(&path, &tris);
        path
    }

    fn test_profile() -> PrinterProfile {
        PrinterProfile {
            id: "test".into(),
            printer_name: "Test Printer".into(),
            slicer_kind: "bambu".into(),
            slicer_path: "/bin/true".into(),
            blank_start_gcode_for_cli: false,
            no_check_for_final_tower_probe: false,
            printable_area: [0.0, 0.0, 20.0, 20.0],
            edge_margin_mm: 0.0,
            exclusion_rectangles: vec![],
            purge_lines: vec![],
            tower_marker_style: "wipe_tower".into(),
            tower_line_width_mm: 0.4,
        }
    }

    #[test]
    fn pack_returns_a_layout_that_independently_passes_validate_result() {
        let dir = tempfile::tempdir().unwrap();
        let stl = square_stl(dir.path());
        let profile = test_profile();
        let args = Args {
            source_3mf: PathBuf::from("unused.3mf"),
            stl: stl.clone(),
            profile_path: PathBuf::from("unused.json"),
            output: PathBuf::from("unused_output.3mf"),
            work_dir: dir.path().to_path_buf(),
            clearance: 0.5,
            resolution: 0.5,
            angle_step: 90.0,
            attempts: 8,
            candidates: 100,
            seed: 1,
        };

        let result = pack(&args, &profile, vec![]).unwrap();
        assert!(!result.placements.is_empty());

        let footprint = extract_footprint(&stl).unwrap();
        let options = PackOptions {
            bed: profile.usable_bed().unwrap(),
            exclusions: vec![],
            resolution: args.resolution,
            clearance: args.clearance,
            angle_step_deg: args.angle_step,
            attempts: args.attempts,
            candidates_per_step: args.candidates,
            seed: args.seed,
        };
        validate_result(&footprint, &options, &result).unwrap();
    }
}
```

- [x] **Step 2: Run the test to verify it fails to compile (no `validate_result` import yet)**

Run: `cargo test --bin colored_3mf_packer pack_returns_a_layout_that_independently_passes_validate_result`
Expected: FAIL to compile — `error[E0433]: failed to resolve: use of undeclared type or module 'validate_result'` (or similar, since `validate_result` is not yet imported/used in `pack`).

- [x] **Step 3: Add `validate_result` to the import list and call it inside `pack`**

In `goat_packer/src/bin/colored_3mf_packer.rs`, find the import block (currently lines 9-16):

```rust
use goat_packer::{
    PackOptions, PackResult, Rect, extract_footprint, pack_footprint,
    colored_packer::{
        PrimeTowerBounds, PrinterProfile, load_printer_profile, parse_prime_tower_bounds,
        tower_repack_required,
    },
    scheduler::object_count_3mf,
};
```

Replace it with:

```rust
use goat_packer::{
    PackOptions, PackResult, Rect, extract_footprint, pack_footprint, validate_result,
    colored_packer::{
        PrimeTowerBounds, PrinterProfile, load_printer_profile, parse_prime_tower_bounds,
        tower_repack_required,
    },
    scheduler::object_count_3mf,
};
```

Find the `pack` function (currently lines 149-162):

```rust
fn pack(args: &Args, profile: &PrinterProfile, exclusions: Vec<Rect>) -> AppResult<PackResult> {
    let footprint = extract_footprint(&args.stl)?;
    let options = PackOptions {
        bed: profile.usable_bed()?,
        exclusions,
        resolution: args.resolution,
        clearance: args.clearance,
        angle_step_deg: args.angle_step,
        attempts: args.attempts,
        candidates_per_step: args.candidates,
        seed: args.seed,
    };
    Ok(pack_footprint(&footprint, &options))
}
```

Replace it with:

```rust
fn pack(args: &Args, profile: &PrinterProfile, exclusions: Vec<Rect>) -> AppResult<PackResult> {
    let footprint = extract_footprint(&args.stl)?;
    let options = PackOptions {
        bed: profile.usable_bed()?,
        exclusions,
        resolution: args.resolution,
        clearance: args.clearance,
        angle_step_deg: args.angle_step,
        attempts: args.attempts,
        candidates_per_step: args.candidates,
        seed: args.seed,
    };
    let result = pack_footprint(&footprint, &options);
    validate_result(&footprint, &options, &result)?;
    Ok(result)
}
```

- [x] **Step 4: Run the test to verify it passes**

Run: `cargo test --bin colored_3mf_packer`
Expected: `test result: ok. 1 passed; 0 failed`

- [x] **Step 5: Run the full test suite**

Run: `cargo test --release --all`
Expected: 27 passed (26 from Task 1 + 1 new), 0 failed.

- [x] **Step 6: Commit**

```bash
git add src/bin/colored_3mf_packer.rs
git commit -m "fix: validate packed layout before assembling a colored 3MF"
```

---

### Task 3: Slicer-kind detection from the source 3MF

**Files:**
- Modify: `goat_packer/src/colored_packer.rs` (append near end of file)
- Modify: `goat_packer/tests/colored_profile.rs`
- Modify: `goat_packer/src/bin/colored_3mf_packer.rs:9-16` (imports), `:287-307` (`validate_source_printer`), append to `mod tests`

- [x] **Step 1: Write the failing library-level test**

Append to `goat_packer/tests/colored_profile.rs`:

```rust

#[test]
fn detects_slicer_kind_from_known_printer_models() {
    use goat_packer::colored_packer::detect_slicer_kind_from_printer_model;
    assert_eq!(
        detect_slicer_kind_from_printer_model("Bambu Lab P1S"),
        Some("bambu")
    );
    assert_eq!(
        detect_slicer_kind_from_printer_model("Snapmaker U1"),
        Some("orca")
    );
    assert_eq!(detect_slicer_kind_from_printer_model("Prusa MK4"), None);
}
```

- [x] **Step 2: Run the test and confirm it fails to compile**

Run: `cargo test --test colored_profile detects_slicer_kind_from_known_printer_models`
Expected: FAIL to compile — `error[E0432]: unresolved import` (`detect_slicer_kind_from_printer_model` does not exist yet).

- [x] **Step 3: Implement `detect_slicer_kind_from_printer_model` in `colored_packer.rs`**

Append to the end of `goat_packer/src/colored_packer.rs`:

```rust

const SLICER_KIND_BY_PRINTER_SUBSTRING: &[(&str, &str)] = &[
    ("Bambu Lab", "bambu"),
    ("Snapmaker", "orca"),
];

/// Maps a 3MF's `printer_model` string to a slicer kind ("bambu"/"orca") using known printer-name
/// substrings. `Application` metadata in the 3MF is not used for this because OrcaSlicer-family
/// tools (including Snapmaker's) are BambuStudio forks and still emit
/// `Application: BambuStudio-x.y.z`, making that field an unreliable signal. Returns `None` for an
/// unrecognized printer so callers can fall back to the printer profile's own `slicer_kind`.
pub fn detect_slicer_kind_from_printer_model(printer_model: &str) -> Option<&'static str> {
    SLICER_KIND_BY_PRINTER_SUBSTRING
        .iter()
        .find(|(needle, _)| printer_model.contains(needle))
        .map(|(_, kind)| *kind)
}
```

- [x] **Step 4: Run the test to verify it passes**

Run: `cargo test --test colored_profile`
Expected: `test result: ok. 3 passed; 0 failed`

- [x] **Step 5: Write the failing integration-level test for the mismatch guard**

Append to the `mod tests` block at the end of `goat_packer/src/bin/colored_3mf_packer.rs` (inside the
existing `mod tests { ... }` from Task 2, alongside the existing test function):

```rust

    #[test]
    fn validate_source_printer_rejects_slicer_kind_mismatch() {
        let profile = load_printer_profile(Path::new("profiles/p1s.json")).unwrap();
        let source =
            Path::new("../Goat 4/p1s/goat4_p1s_contour_packed_purge_safe_faster.3mf");
        validate_source_printer(source, &profile).unwrap();

        let mut mismatched = profile.clone();
        mismatched.slicer_kind = "orca".to_owned();
        let error = validate_source_printer(source, &mismatched).unwrap_err();
        assert!(error.to_string().contains("slicer"));
    }
```

- [x] **Step 6: Run the test and confirm it fails**

Run: `cargo test --bin colored_3mf_packer validate_source_printer_rejects_slicer_kind_mismatch`
Expected: FAIL — either a compile error (`detect_slicer_kind_from_printer_model` not imported) once
Step 7 hasn't happened yet, or (if compiled) the test fails because `validate_source_printer` doesn't
check slicer kind and returns `Ok` for the mismatched profile instead of an `Err`.

- [x] **Step 7: Add the slicer-kind import and cross-check to `validate_source_printer`**

In `goat_packer/src/bin/colored_3mf_packer.rs`, update the import block (from Task 2's version) to add
`detect_slicer_kind_from_printer_model`:

```rust
use goat_packer::{
    PackOptions, PackResult, Rect, extract_footprint, pack_footprint, validate_result,
    colored_packer::{
        PrimeTowerBounds, PrinterProfile, detect_slicer_kind_from_printer_model,
        load_printer_profile, parse_prime_tower_bounds, tower_repack_required,
    },
    scheduler::object_count_3mf,
};
```

Find `validate_source_printer` (currently lines 287-307):

```rust
fn validate_source_printer(source: &Path, profile: &PrinterProfile) -> AppResult<()> {
    let mut archive = ZipArchive::new(fs::File::open(source)?)?;
    let mut settings = Vec::new();
    use std::io::Read;
    archive
        .by_name("Metadata/project_settings.config")?
        .read_to_end(&mut settings)?;
    let configured: serde_json::Value = serde_json::from_slice(&settings)?;
    let printer = configured
        .get("printer_model")
        .and_then(serde_json::Value::as_str)
        .ok_or("source project has no printer_model")?;
    if printer != profile.printer_name {
        return Err(format!(
            "source printer '{printer}' does not match profile '{}'",
            profile.printer_name
        )
        .into());
    }
    Ok(())
}
```

Replace it with:

```rust
fn validate_source_printer(source: &Path, profile: &PrinterProfile) -> AppResult<()> {
    let mut archive = ZipArchive::new(fs::File::open(source)?)?;
    let mut settings = Vec::new();
    use std::io::Read;
    archive
        .by_name("Metadata/project_settings.config")?
        .read_to_end(&mut settings)?;
    let configured: serde_json::Value = serde_json::from_slice(&settings)?;
    let printer = configured
        .get("printer_model")
        .and_then(serde_json::Value::as_str)
        .ok_or("source project has no printer_model")?;
    if printer != profile.printer_name {
        return Err(format!(
            "source printer '{printer}' does not match profile '{}'",
            profile.printer_name
        )
        .into());
    }
    match detect_slicer_kind_from_printer_model(printer) {
        Some(detected) if detected != profile.slicer_kind => {
            return Err(format!(
                "printer_model '{printer}' implies slicer '{detected}', but profile '{}' is \
                 configured for slicer '{}'",
                profile.id, profile.slicer_kind
            )
            .into());
        }
        Some(_) => {}
        None => println!(
            "note: no known slicer mapping for printer_model '{printer}'; trusting profile \
             slicer_kind '{}'",
            profile.slicer_kind
        ),
    }
    Ok(())
}
```

- [x] **Step 8: Run the test to verify it passes**

Run: `cargo test --bin colored_3mf_packer`
Expected: `test result: ok. 2 passed; 0 failed`

- [x] **Step 9: Run the full test suite**

Run: `cargo test --release --all`
Expected: 30 passed (27 from Task 2 + 3 new: 1 lib-level detection test's 3 assertions count as 1 test
fn plus the 2 integration tests already counted — recount by reading the actual `cargo test` summary
line rather than assuming; the point is 0 failed), 0 failed.

- [x] **Step 10: Commit**

```bash
git add src/colored_packer.rs src/bin/colored_3mf_packer.rs tests/colored_profile.rs
git commit -m "feat: detect slicer kind from 3MF printer_model, fall back to profile"
```

---

### Task 4: Produce the Goat 3 / P1S colored packed deliverable

**Files:**
- Create: `Goat 3/p1s/goat3_p1s_colored_contour_packed.3mf`
- Create: `Goat 3/p1s/goat3_p1s_colored_contour_packed.verification.json` (written automatically by the tool)
- Create: `Goat 3/p1s/goat3_p1s_colored_contour_packed.exact_verification.json` (written by the step-3 check below)

- [x] **Step 1: Rebuild the release binaries with all fixes from Tasks 1-3**

Run: `cargo build --release`
Expected: builds cleanly, no warnings about unused imports.

- [x] **Step 2: Run the real pipeline against the Goat 3 P1S source project**

Run (from `goat_packer/`, this invokes the real BambuStudio CLI twice — a probe slice and a final
slice — and can take several minutes):

```bash
mkdir -p "../Goat 3/p1s/work"
./target/release/colored_3mf_packer \
  --source-3mf "../Goat 3/source/goat3.3mf" \
  --stl "../Goat 3/source/goat3.stl" \
  --profile profiles/p1s.json \
  --output "../Goat 3/p1s/goat3_p1s_colored_contour_packed.3mf" \
  --work-dir "../Goat 3/p1s/work" \
  --attempts 16000 --candidates 2400 --seed 20270831
```

Expected: prints `probe tower keepout: ...`, then either finishes with
`packed N colored objects; wrote ../Goat 3/p1s/goat3_p1s_colored_contour_packed.3mf` (success), or
prints `final tower outgrew probe keepout; repacking once` followed by the same success line (one
repack is allowed by design).

- [x] **Step 3: If it still fails with a slicing conflict, escalate the safety margin and retry**

If Step 2 instead prints `error: Bambu Lab P1S slicing failed: ...` and
`../Goat 3/p1s/work/final_first_slice.log` (or `final_repacked_slice.log`) contains
`gcode path conflicts` or `found slicing result conflict`, the 1.5mm margin from Task 1 was
insufficient for this specific layout. Increase `TOWER_EXTRA_SAFETY_MM` in
`goat_packer/src/colored_packer.rs` from `1.5` to `2.5`, update the two hardcoded expectations in
`goat_packer/tests/prime_tower.rs` accordingly (`7.55` → `6.55`, `24.45` → `25.45`, and the
`keepout_adds_a_fixed_safety_margin_beyond_line_width_and_clearance` test's `expected_expansion`
from `2.45` to `3.45`), rerun `cargo test --test prime_tower` to confirm the tests pass with the new
constant, rebuild with `cargo build --release`, and retry Step 2. Repeat once more with `3.5` if
`2.5` is still insufficient. Do not retry more than 3 total margin values (1.5, 2.5, 3.5) without
stopping to re-diagnose — a persistent failure at 3.5mm extra margin indicates a different root
cause, not an undersized margin.

- [x] **Step 4: Independently verify the produced project with `verify_outputs.py`**

Run (from `goat_packer/`):

```bash
python3 verify_outputs.py \
  "../Goat 3/source/goat3.stl" \
  "../Goat 3/p1s/work/placements.json" \
  "../Goat 3/p1s/goat3_p1s_colored_contour_packed.3mf" \
  --minimum-gap 0.5 \
  --edge-margin 0.0 \
  --purge-line 18,1,240,1,0.5 \
  --purge-line 240,1,240,11,0.5 \
  --purge-line 239.5,11,239.5,1.5,0.5 \
  --purge-line 239.5,1.5,18,1.5,0.5 \
  --keepout 0.0,0.0,18.0,28.0 \
  --report "../Goat 3/p1s/goat3_p1s_colored_contour_packed.exact_verification.json"
```

Expected: prints a JSON report with `"all_inside_printable_area": true`, `"exclusion_clear": true`,
`"zip_crc_ok": true`, `"metadata_object_counts_match": true`,
`"embedded_mesh_transform_ok": true`, and `"minimum_exact_gap_mm"` ≥ `0.5`. Exits 0. If it raises an
`AssertionError`, read the message, fix the underlying issue (most likely stale `placements.json`
from a previous attempt in Step 3 — regenerate by rerunning Step 2/3), and rerun.

- [x] **Step 5: Read and sanity-check the tool's own verification report**

Run: `cat "../Goat 3/p1s/goat3_p1s_colored_contour_packed.verification.json"`
Expected: `"repacked"` is `true` or `false` (both acceptable), `"paint_color_attribute_mentions"` is
greater than `0` (confirms paint data survived cloning), and `"object_count"` matches the placement
count printed in Step 2.

- [x] **Step 6: Commit the deliverable**

```bash
git add "../Goat 3/p1s/goat3_p1s_colored_contour_packed.3mf" \
        "../Goat 3/p1s/goat3_p1s_colored_contour_packed.verification.json" \
        "../Goat 3/p1s/goat3_p1s_colored_contour_packed.exact_verification.json"
git commit -m "feat: add packed+verified colored Goat 3 P1S project"
```

(Skip if this directory is not a git repository.)

---

### Task 5: Produce the Goat 3 / Snapmaker U1 colored packed deliverable

**Files:**
- Create: `Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.3mf`
- Create: `Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.verification.json` (written automatically)
- Create: `Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.exact_verification.json`

- [x] **Step 1: Run the real pipeline against the Goat 3 U1 source project**

Run (from `goat_packer/`; the U1 profile sets `blank_start_gcode_for_cli: true` and
`no_check_for_final_tower_probe: true`, both already handled by existing code):

```bash
mkdir -p "../Goat 3/snap/work"
./target/release/colored_3mf_packer \
  --source-3mf "../Goat 3/snap/goat3_snap.3mf" \
  --stl "../Goat 3/source/goat3.stl" \
  --profile profiles/snapmaker_u1.json \
  --output "../Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.3mf" \
  --work-dir "../Goat 3/snap/work" \
  --attempts 16000 --candidates 2400 --seed 20270832
```

Expected: same success pattern as Task 4 Step 2, using the OrcaSlicer CLI instead of BambuStudio.

- [x] **Step 2: If it fails with a slicing conflict, apply the same escalation as Task 4 Step 3**

Reuse whatever `TOWER_EXTRA_SAFETY_MM` value Task 4 landed on (the constant is shared by both
printers); only escalate further if this specific layout still conflicts at that value, following
the same 1.5 → 2.5 → 3.5 progression and re-test/rebuild sequence described in Task 4 Step 3.

- [x] **Step 3: Independently verify the produced project with `verify_outputs.py`**

Run (from `goat_packer/`):

```bash
python3 verify_outputs.py \
  "../Goat 3/source/goat3.stl" \
  "../Goat 3/snap/work/placements.json" \
  "../Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.3mf" \
  --minimum-gap 0.5 \
  --edge-margin 3.5 \
  --purge-line 85,1,185,1,0.5 \
  --report "../Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.exact_verification.json"
```

Expected: same success criteria as Task 4 Step 4.

- [x] **Step 4: Read and sanity-check the tool's own verification report**

Run: `cat "../Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.verification.json"`
Expected: same checks as Task 4 Step 5.

- [x] **Step 5: Commit the deliverable**

```bash
git add "../Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.3mf" \
        "../Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.verification.json" \
        "../Goat 3/snap/goat3_snapmaker_u1_colored_contour_packed.exact_verification.json"
git commit -m "feat: add packed+verified colored Goat 3 Snapmaker U1 project"
```

(Skip if this directory is not a git repository.)

---

### Task 6: Final full-suite regression check

**Files:** none (verification only)

- [x] **Step 1: Run the full Rust test suite**

Run: `cargo test --release --all`
Expected: all tests pass, 0 failed.

- [x] **Step 2: Run the full Python test suite**

Run: `python3 tests/test_build_3mf.py -v`
Expected: `OK`, all 9 tests pass.

- [x] **Step 3: Report final state**

Summarize for the user: which `TOWER_EXTRA_SAFETY_MM` value was needed, the final object counts for
both P1S and U1 plates, and the paths to the four deliverable files (2 `.3mf` + 2
`.verification.json`, plus the 2 `.exact_verification.json` cross-checks).

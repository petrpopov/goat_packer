use std::{
    error::Error,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

use goat_packer::{
    Footprint, PackOptions, PackResult, Rect, extract_footprint, pack_footprint, validate_result,
    assemble::{AssembleOptions, assemble_project},
    colored_packer::{
        PrimeTowerBounds, PrinterProfile, detect_slicer_kind_from_printer_model,
        load_printer_profile, parse_prime_tower_bounds, tower_repack_required,
    },
    embedded_mesh::{embedded_mesh_triangles, write_binary_stl},
    scheduler::object_count_3mf,
};
use serde::Serialize;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Debug)]
struct Args {
    source_3mf: PathBuf,
    profile_path: PathBuf,
    output: PathBuf,
    work_dir: PathBuf,
    clearance: f64,
    resolution: f64,
    angle_step: f64,
    attempts: usize,
    candidates: usize,
    stall: usize,
    seed: u64,
}

#[derive(Debug, Serialize)]
struct ColoredPackReport {
    source_3mf: String,
    output_3mf: String,
    profile: String,
    object_count: usize,
    clearance_mm: f64,
    probe_tower: Option<PrimeTowerBounds>,
    final_tower: Option<PrimeTowerBounds>,
    repacked: bool,
    paint_color_attribute_mentions: usize,
}

/// What to do after comparing the probe-slice tower against the final-slice tower. A project with
/// no prime/wipe tower (e.g. a single-color plate) has `probe` and `final` both `None` throughout
/// and never needs a repack.
#[derive(Debug, PartialEq)]
enum TowerOutcome {
    Stable,
    RepackRequired,
}

/// Decides how to react to the final slice's tower given what the probe slice found. A `None`
/// probe means the source project has no prime/wipe tower (single material/color): packing never
/// reserved space for one, so a tower appearing in the final slice would silently invalidate every
/// collision check and must be rejected rather than repacked around.
fn evaluate_final_tower(
    probe_tower: Option<PrimeTowerBounds>,
    final_tower: Option<PrimeTowerBounds>,
    reserved_keepout: Option<Rect>,
) -> Result<TowerOutcome, String> {
    match (probe_tower, final_tower) {
        (None, None) => Ok(TowerOutcome::Stable),
        (None, Some(_)) => Err(
            "prime/wipe tower appeared in the final slice though the probe slice (1 object) had \
             none; packing did not reserve space for it"
                .to_owned(),
        ),
        (Some(_), None) => Err(
            "prime/wipe tower markers were not found in final slice, though the probe slice had \
             one"
                .to_owned(),
        ),
        (Some(_), Some(final_tower)) => {
            let reserved = reserved_keepout.ok_or("prime tower keepout was not reserved")?;
            if tower_repack_required(reserved, final_tower) {
                Ok(TowerOutcome::RepackRequired)
            } else {
                Ok(TowerOutcome::Stable)
            }
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let args = parse_args()?;
    let profile = load_printer_profile(&args.profile_path)?;
    validate_source_printer(&args.source_3mf, &profile)?;
    fs::create_dir_all(&args.work_dir)?;
    let source_count = object_count_3mf(&args.source_3mf)?;
    if source_count != 1 {
        return Err(format!(
            "source project must contain exactly one painted object; found {source_count}"
        )
        .into());
    }
    // Collide against the exact mesh the 3MF embeds, not a separately-provided STL: an
    // independently exported STL can drift from the embedded mesh (different export pass, tool
    // version, or manual edit) while keeping the same overall bounding box, silently invalidating
    // every collision check that assumes the two are identical. This reproduced as a real
    // near-zero-clearance touch between two placed Goat 3 objects on a real P1S plate: the
    // standalone STL's keyring-hole position did not match the embedded mesh's.
    let footprint_stl = args.work_dir.join("embedded_source.stl");
    derive_embedded_stl(&args.source_3mf, &footprint_stl)?;
    eprintln!("probing prime tower position (slicing 1 object)...");
    let probe_gcode = slice_project(&profile, &args.source_3mf, 1, &args.work_dir, "probe")?;
    let probe_tower =
        parse_prime_tower_bounds(&probe_gcode, profile.tower_line_width_mm, args.clearance);
    match probe_tower {
        Some(tower) => println!(
            "probe tower keepout: {:.2},{:.2} → {:.2},{:.2}",
            tower.keepout.min_x, tower.keepout.min_y, tower.keepout.max_x, tower.keepout.max_y
        ),
        None => println!(
            "no prime/wipe tower markers in source slice (single material/color); packing \
             without a tower keepout"
        ),
    }

    let mut keepouts = profile.fixed_keepouts()?;
    if let Some(tower) = probe_tower {
        keepouts.push(tower.keepout);
    }
    eprintln!("packing ({} attempts, seed {})...", args.attempts, args.seed);
    let mut packed = pack(&args, &profile, &footprint_stl, keepouts.clone())?;
    eprintln!("packed {} objects; assembling 3MF...", packed.placements.len());
    build_colored_project(&args, &profile, &footprint_stl, &packed, &keepouts)?;
    eprintln!("slicing final plate ({} objects)...", packed.placements.len());
    let mut final_gcode = slice_project(
        &profile,
        &args.output,
        packed.placements.len(),
        &args.work_dir,
        "final_first",
    )?;
    let mut final_tower =
        parse_prime_tower_bounds(&final_gcode, profile.tower_line_width_mm, args.clearance);
    let mut repacked = false;
    let reserved_tower_keepout = probe_tower.map(|tower| tower.keepout);
    if evaluate_final_tower(probe_tower, final_tower, reserved_tower_keepout)?
        == TowerOutcome::RepackRequired
    {
        println!("final tower outgrew probe keepout; repacking once");
        keepouts.pop();
        keepouts.push(final_tower.unwrap().keepout);
        eprintln!("re-packing ({} attempts, seed {})...", args.attempts, args.seed);
        packed = pack(&args, &profile, &footprint_stl, keepouts.clone())?;
        eprintln!("packed {} objects; assembling 3MF...", packed.placements.len());
        build_colored_project(&args, &profile, &footprint_stl, &packed, &keepouts)?;
        eprintln!("slicing repacked plate ({} objects)...", packed.placements.len());
        final_gcode = slice_project(
            &profile,
            &args.output,
            packed.placements.len(),
            &args.work_dir,
            "final_repacked",
        )?;
        final_tower =
            parse_prime_tower_bounds(&final_gcode, profile.tower_line_width_mm, args.clearance);
        if evaluate_final_tower(probe_tower, final_tower, keepouts.last().copied())?
            == TowerOutcome::RepackRequired
        {
            return Err("prime tower changed again after the permitted re-pack".into());
        }
        repacked = true;
    }

    let report = ColoredPackReport {
        source_3mf: args.source_3mf.display().to_string(),
        output_3mf: args.output.display().to_string(),
        profile: profile.id,
        object_count: packed.placements.len(),
        clearance_mm: args.clearance,
        probe_tower,
        final_tower,
        repacked,
        paint_color_attribute_mentions: paint_attribute_count(&args.source_3mf)?,
    };
    let report_path = args.output.with_extension("verification.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "packed {} colored objects; wrote {}\nreport: {}",
        report.object_count,
        args.output.display(),
        report_path.display()
    );
    Ok(())
}

fn derive_embedded_stl(source_3mf: &Path, output_stl: &Path) -> AppResult<()> {
    // The collision STL is the 3MF's own embedded mesh with the component's transform baked in;
    // see the call-site comment for why a separately exported STL cannot be trusted.
    let triangles = embedded_mesh_triangles(source_3mf).map_err(|error| {
        format!("failed to derive collision STL from embedded mesh: {error}")
    })?;
    // The Python exporter refused to write an STL with no triangles; keep that loud failure.
    if triangles.is_empty() {
        return Err(
            "failed to derive collision STL from embedded mesh: embedded mesh has no triangles"
                .into(),
        );
    }
    write_binary_stl(output_stl, &triangles).map_err(|error| {
        format!("failed to derive collision STL from embedded mesh: {error}")
    })?;
    Ok(())
}

fn pack(
    args: &Args,
    profile: &PrinterProfile,
    footprint_stl: &Path,
    exclusions: Vec<Rect>,
) -> AppResult<PackResult> {
    let footprint = extract_footprint(footprint_stl)?;
    let options = PackOptions {
        bed: profile.usable_bed()?,
        exclusions,
        resolution: args.resolution,
        clearance: args.clearance,
        angle_step_deg: args.angle_step,
        attempts: args.attempts,
        candidates_per_step: args.candidates,
        stall_attempts: args.stall,
        seed: args.seed,
    };
    let result = pack_footprint(&footprint, &options);
    validate_result(&footprint, &options, &result)?;
    let bounds = layout_world_bounds(&footprint, &result);
    eprintln!(
        "layout: {} objects within x [{:.1}, {:.1}], y [{:.1}, {:.1}] (bed {:.1}..{:.1} x \
         {:.1}..{:.1}); per-object positions: {}",
        result.placements.len(),
        bounds.min_x,
        bounds.max_x,
        bounds.min_y,
        bounds.max_y,
        options.bed.min_x,
        options.bed.max_x,
        options.bed.min_y,
        options.bed.max_y,
        args.work_dir.join("placements.json").display(),
    );
    Ok(result)
}

/// World-XY bounding box of the packed layout, from the exact footprint loops rotated per
/// placement. The packer's own bed-fit check works on rasterized masks; this recomputation is
/// for the operator-facing log line that says where the layout actually sits on the bed.
fn layout_world_bounds(footprint: &Footprint, result: &PackResult) -> Rect {
    let mut bounds = Rect::new(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for placement in &result.placements {
        let (sin, cos) = placement.angle_deg.to_radians().sin_cos();
        for point in footprint.loops.iter().flatten() {
            let x = placement.x_mm + point.x * cos - point.y * sin;
            let y = placement.y_mm + point.x * sin + point.y * cos;
            bounds.min_x = bounds.min_x.min(x);
            bounds.min_y = bounds.min_y.min(y);
            bounds.max_x = bounds.max_x.max(x);
            bounds.max_y = bounds.max_y.max(y);
        }
    }
    bounds
}

fn build_colored_project(
    args: &Args,
    profile: &PrinterProfile,
    footprint_stl: &Path,
    packed: &PackResult,
    keepouts: &[Rect],
) -> AppResult<()> {
    let placements = args.work_dir.join("placements.json");
    fs::write(&placements, serde_json::to_vec_pretty(packed)?)?;
    // Deliberately NOT reuse_source_object_instances: that mode (one shared <object>, N build
    // items) reliably reproduces BambuStudio's real CLI "gcode path conflicts found between
    // goat3.obj and goat3.obj" / "found slicing result conflict" error on a real ~20-object dense
    // plate, even though the exact same layout slices cleanly in the default mode used here (N
    // distinct <object> resources sharing one external mesh component, the same pattern already
    // proven for the uncolored Goat 4 packer). Paint attributes live in the untouched shared mesh
    // component either way, so this mode is equally paint-safe.
    let options = AssembleOptions {
        placements: packed.placements.clone(),
        stl_path: footprint_stl.to_path_buf(),
        edge_margin: profile.edge_margin_mm,
        purge_lines: profile.purge_lines.clone(),
        keepouts: keepouts.to_vec(),
        duplicate_mesh_per_object: false,
        reuse_source_object_instances: false,
        inline_colored_mesh_objects: false,
    };
    assemble_project(&args.source_3mf, &args.output, &options)
        .map_err(|error| format!("3MF assembly failed: {error}"))?;
    Ok(())
}

fn slice_project(
    profile: &PrinterProfile,
    project: &Path,
    _count: usize,
    work_dir: &Path,
    label: &str,
) -> AppResult<Vec<u8>> {
    let sanitized = work_dir.join(format!("{label}_cli.3mf"));
    let input = if prepare_cli_copy(profile, project, &sanitized)? {
        sanitized
    } else {
        project.to_owned()
    };
    let output_dir = work_dir.join(format!("{label}_slice"));
    if output_dir.exists() {
        fs::remove_dir_all(&output_dir)?;
    }
    fs::create_dir_all(&output_dir)?;
    let mut command = Command::new(&profile.slicer_path);
    if profile.no_check_for_final_tower_probe && label.starts_with("final") {
        command.arg("--no-check");
    }
    let output = command
        .arg("--slice")
        .arg("1")
        .arg("--outputdir")
        .arg(&output_dir)
        .arg(&input)
        .output()?;
    let log_path = work_dir.join(format!("{label}_slice.log"));
    fs::write(&log_path, [&output.stdout[..], &output.stderr[..]].concat())?;
    if !output.status.success() {
        // BambuStudio/Orca print their real diagnosis to stdout (and to result.json in the
        // output dir), not to stderr — reporting only stderr yielded an empty "slicing failed:"
        // message on real plates. Surface the most specific source first, plus enough context
        // to re-run the exact failing command by hand.
        let report_error = slicer_report_error(&output_dir)
            .unwrap_or_else(|| "<slicer wrote no result.json>".to_owned());
        return Err(format!(
            "{} slicing failed ({})\n\
             slicer report: {report_error}\n\
             slicer output (tail):\n{}\n\
             full log: {}\n\
             output dir: {}\n\
             rerun manually: {} --slice 1 --outputdir {} {}",
            profile.printer_name,
            output.status,
            output_tail(&output.stdout, &output.stderr, 15),
            log_path.display(),
            output_dir.display(),
            profile.slicer_path,
            output_dir.display(),
            input.display(),
        )
        .into());
    }
    let gcode = fs::read_dir(output_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "gcode"))
        .ok_or("slicer output did not contain G-code")?;
    Ok(fs::read(gcode)?)
}

/// Reads the slicer's own `result.json` from the output dir and extracts its `error_string` —
/// the single most useful line when a slice fails (e.g. "Some objects are located over the
/// boundary of the heated bed."). Returns `None` when the file or field is absent.
fn slicer_report_error(output_dir: &Path) -> Option<String> {
    let report = fs::read_dir(output_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.file_name().is_some_and(|name| name == "result.json"))?;
    let value: serde_json::Value = serde_json::from_slice(&fs::read(report).ok()?).ok()?;
    let error = value
        .get("error_string")
        .and_then(serde_json::Value::as_str)?
        .to_owned();
    (!error.is_empty()).then_some(error)
}

/// Last `max_lines` non-empty lines of the slicer's combined stdout+stderr, each indented, for
/// embedding in a failure message. Keeps `[error] ...` lines that BambuStudio logs mid-run.
fn output_tail(stdout: &[u8], stderr: &[u8], max_lines: usize) -> String {
    let combined = [stdout, stderr].concat();
    let text = String::from_utf8_lossy(&combined);
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..]
        .iter()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rewrites `project` into `output` with CLI-slicing workarounds applied, returning whether a
/// modified copy was actually written (callers should slice the original file directly when
/// nothing needed changing). Two independent fixes are applied to `Metadata/project_settings.config`:
/// blanking `machine_start_gcode` when the profile requires it, and resizing `flush_volumes_matrix`
/// to `filament_colour.len()^2` when they've drifted apart. The latter is a stale-data artifact seen
/// on "single_*" projects trimmed down from a multi-color template: `filament_colour` shrinks to the
/// filaments actually in use, but `flush_volumes_matrix` is left sized for the old (larger) count.
/// OrcaSlicer's CLI rejects that outright ("Flush volumes matrix do not match to the correct
/// size!") even on a project whose object never references a second filament, so the mismatched
/// values are never actually read during slicing — only the size needs to be correct.
fn prepare_cli_copy(profile: &PrinterProfile, project: &Path, output: &Path) -> AppResult<bool> {
    let mut source = ZipArchive::new(fs::File::open(project)?)?;
    let mut entries = Vec::with_capacity(source.len());
    let mut changed = false;
    for index in 0..source.len() {
        let mut entry = source.by_index(index)?;
        let name = entry.name().to_owned();
        let is_dir = entry.is_dir();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        if name == "Metadata/project_settings.config" {
            let mut settings: serde_json::Value = serde_json::from_slice(&bytes)?;
            if profile.blank_start_gcode_for_cli {
                settings["machine_start_gcode"] = serde_json::Value::String(String::new());
                changed = true;
            }
            if normalize_flush_volumes_matrix(&mut settings) {
                changed = true;
            }
            bytes = serde_json::to_vec(&settings)?;
        }
        entries.push((name, is_dir, bytes));
    }
    if !changed {
        return Ok(false);
    }
    let mut archive = ZipWriter::new(fs::File::create(output)?);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, is_dir, bytes) in entries {
        if is_dir {
            archive.add_directory(name, options)?;
        } else {
            archive.start_file(name, options)?;
            archive.write_all(&bytes)?;
        }
    }
    archive.finish()?;
    Ok(true)
}

/// Resizes `settings["flush_volumes_matrix"]` to `filament_colour.len()^2` entries when it doesn't
/// already match, reusing values from the overlapping top-left block of the old matrix (row-major,
/// square) and filling any newly-added cells with `"0"`. Returns `false` (no-op) when either field
/// is missing/not an array or the size already matches.
fn normalize_flush_volumes_matrix(settings: &mut serde_json::Value) -> bool {
    let Some(filament_count) = settings
        .get("filament_colour")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
    else {
        return false;
    };
    let Some(matrix) = settings
        .get("flush_volumes_matrix")
        .and_then(serde_json::Value::as_array)
        .cloned()
    else {
        return false;
    };
    let expected_len = filament_count * filament_count;
    if matrix.len() == expected_len {
        return false;
    }
    let old_side = (matrix.len() as f64).sqrt().round() as usize;
    let old_side = if old_side * old_side == matrix.len() { old_side } else { 0 };
    let zero = serde_json::Value::String("0".to_owned());
    let mut resized = Vec::with_capacity(expected_len);
    for row in 0..filament_count {
        for col in 0..filament_count {
            let value = if row < old_side && col < old_side {
                matrix[row * old_side + col].clone()
            } else {
                zero.clone()
            };
            resized.push(value);
        }
    }
    settings["flush_volumes_matrix"] = serde_json::Value::Array(resized);
    true
}

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

fn paint_attribute_count(source: &Path) -> AppResult<usize> {
    let mut archive = ZipArchive::new(fs::File::open(source)?)?;
    let mut root = Vec::new();
    use std::io::Read;
    archive.by_name("3D/3dmodel.model")?.read_to_end(&mut root)?;
    let text = String::from_utf8_lossy(&root);
    let marker = "p:path=\"/";
    let start = text.find(marker).ok_or("source core has no component path")? + marker.len();
    let end = text[start..]
        .find('"')
        .map(|offset| start + offset)
        .ok_or("component path is unterminated")?;
    let path = &text[start..end];
    let mut mesh = Vec::new();
    archive.by_name(path)?.read_to_end(&mut mesh)?;
    Ok(String::from_utf8_lossy(&mesh).matches("paint_color").count())
}

fn parse_args() -> AppResult<Args> {
    let mut args = std::env::args().skip(1);
    let mut source_3mf = None;
    let mut profile_path = None;
    let mut output = None;
    let mut work_dir = None;
    let mut clearance = 0.7;
    let mut resolution = 0.25;
    let mut angle_step = 2.0;
    let mut attempts = 16_000;
    let mut candidates = 2_400;
    let mut stall = None;
    let mut seed = 20_270_831;
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--source" => source_3mf = Some(PathBuf::from(value)),
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--work-dir" => work_dir = Some(PathBuf::from(value)),
            "--clearance" => clearance = value.parse()?,
            "--resolution" => resolution = value.parse()?,
            "--angle-step" => angle_step = value.parse()?,
            "--attempts" => attempts = value.parse()?,
            "--candidates" => candidates = value.parse()?,
            "--stall" => stall = Some(value.parse()?),
            "--seed" => seed = value.parse()?,
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    let output = output.ok_or("--output is required")?;
    let work_dir = work_dir.unwrap_or_else(|| output.with_extension("work"));
    let attempts = attempts.max(1);
    // Default: give the search a quarter of the restarts to keep improving after the last gain
    // before declaring it plateaued. 0 explicitly disables early stopping.
    let stall = stall.unwrap_or((attempts / 4).max(1));
    Ok(Args {
        source_3mf: source_3mf.ok_or("--source is required")?,
        profile_path: profile_path.ok_or("--profile is required")?,
        output,
        work_dir,
        clearance,
        resolution,
        angle_step,
        attempts,
        candidates,
        stall,
        seed,
    })
}

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
            profile_path: PathBuf::from("unused.json"),
            output: PathBuf::from("unused_output.3mf"),
            work_dir: dir.path().to_path_buf(),
            clearance: 0.5,
            resolution: 0.5,
            angle_step: 90.0,
            attempts: 8,
            candidates: 100,
            stall: 0,
            seed: 1,
        };

        let result = pack(&args, &profile, &stl, vec![]).unwrap();
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
            stall_attempts: args.stall,
            seed: args.seed,
        };
        validate_result(&footprint, &options, &result).unwrap();
    }

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

    fn sample_tower() -> PrimeTowerBounds {
        PrimeTowerBounds {
            centerline: Rect::new(1.0, 1.0, 5.0, 5.0),
            keepout: Rect::new(0.0, 0.0, 6.0, 6.0),
            extrusion_width_mm: 0.4,
        }
    }

    #[test]
    fn evaluate_final_tower_is_stable_when_no_project_has_a_tower() {
        let outcome = evaluate_final_tower(None, None, None).unwrap();
        assert_eq!(outcome, TowerOutcome::Stable);
    }

    #[test]
    fn evaluate_final_tower_rejects_a_tower_that_appears_only_in_the_final_slice() {
        let error = evaluate_final_tower(None, Some(sample_tower()), None).unwrap_err();
        assert!(error.contains("appeared in the final slice"));
    }

    #[test]
    fn evaluate_final_tower_rejects_a_tower_missing_from_the_final_slice() {
        let error =
            evaluate_final_tower(Some(sample_tower()), None, Some(sample_tower().keepout))
                .unwrap_err();
        assert!(error.contains("were not found in final slice"));
    }

    #[test]
    fn evaluate_final_tower_is_stable_when_final_tower_fits_the_reserved_keepout() {
        let reserved = sample_tower().keepout;
        let outcome = evaluate_final_tower(Some(sample_tower()), Some(sample_tower()), Some(reserved))
            .unwrap();
        assert_eq!(outcome, TowerOutcome::Stable);
    }

    #[test]
    fn evaluate_final_tower_requires_repack_when_final_tower_outgrows_reserved_keepout() {
        let reserved = sample_tower().keepout;
        let grown = PrimeTowerBounds {
            centerline: Rect::new(1.0, 1.0, 5.0, 5.0),
            keepout: Rect::new(-1.0, -1.0, 6.0, 6.0),
            extrusion_width_mm: 0.4,
        };
        let outcome =
            evaluate_final_tower(Some(sample_tower()), Some(grown), Some(reserved)).unwrap();
        assert_eq!(outcome, TowerOutcome::RepackRequired);
    }

    #[test]
    fn normalize_flush_volumes_matrix_shrinks_a_stale_larger_matrix() {
        let mut settings = serde_json::json!({
            "filament_colour": ["#C6001A", "#CBC6B8"],
            "flush_volumes_matrix": ["0", "214", "268", "381", "613", "0", "323", "558", "601",
                "222", "0", "442", "592", "280", "298", "0"],
        });
        assert!(normalize_flush_volumes_matrix(&mut settings));
        assert_eq!(
            settings["flush_volumes_matrix"],
            serde_json::json!(["0", "214", "613", "0"])
        );
    }

    #[test]
    fn normalize_flush_volumes_matrix_grows_a_stale_smaller_matrix_with_zeros() {
        let mut settings = serde_json::json!({
            "filament_colour": ["#C6001A", "#CBC6B8", "#FFFFFF"],
            "flush_volumes_matrix": ["0", "214", "613", "0"],
        });
        assert!(normalize_flush_volumes_matrix(&mut settings));
        assert_eq!(
            settings["flush_volumes_matrix"],
            serde_json::json!(["0", "214", "0", "613", "0", "0", "0", "0", "0"])
        );
    }

    #[test]
    fn normalize_flush_volumes_matrix_is_a_noop_when_already_consistent() {
        let mut settings = serde_json::json!({
            "filament_colour": ["#C6001A"],
            "flush_volumes_matrix": ["0"],
        });
        assert!(!normalize_flush_volumes_matrix(&mut settings));
    }

    #[test]
    fn output_tail_keeps_the_last_non_empty_lines_of_combined_slicer_output() {
        let stdout = b"trace line 1\ntrace line 2\n\n[error] plate 1: boom\n";
        let stderr = b"";
        let tail = output_tail(stdout, stderr, 2);
        assert_eq!(tail, "  trace line 2\n  [error] plate 1: boom");
    }

    #[test]
    fn slicer_report_error_reads_error_string_from_result_json() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(slicer_report_error(dir.path()), None);
        fs::write(
            dir.path().join("result.json"),
            r#"{"error_string":"Some objects are located over the boundary of the heated bed.","return_code":-52}"#,
        )
        .unwrap();
        assert_eq!(
            slicer_report_error(dir.path()).as_deref(),
            Some("Some objects are located over the boundary of the heated bed.")
        );
    }

    #[test]
    fn normalize_flush_volumes_matrix_is_a_noop_without_filament_colour() {
        let mut settings = serde_json::json!({
            "flush_volumes_matrix": ["0", "214", "268", "381", "613", "0", "323", "558", "601",
                "222", "0", "442", "592", "280", "298", "0"],
        });
        assert!(!normalize_flush_volumes_matrix(&mut settings));
    }
}

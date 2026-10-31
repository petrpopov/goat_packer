use std::{
    error::Error,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use goat_packer::{
    Footprint, PackOptions, PackResult, Point, Rect, extract_footprint, first_layer_footprint,
    grid_layout, loops_touch_rect, pack_footprint, patch_overshoot_mm, patch_within_silhouette,
    union_loops, validate_result,
    assemble::{AssembleOptions, assemble_project, source_model_center},
    colored_packer::{
        PrimeTowerBounds, PrinterProfile, apply_cli_settings_workarounds,
        detect_slicer_kind_from_printer_model, load_printer_profile, parse_prime_tower_bounds, tower_repack_required,
    },
    embedded_mesh::{embedded_mesh_triangles, stl_center_3d, write_binary_stl},
    gcode_collisions::{PlateAnalysis, SupportCollision, analyze_plate},
    nozzle_clean::{NozzleCleanSpec, inject_nozzle_clean},
    plate_centre::{
        RelocatedTower, TOWER_TARGET_TOLERANCE_MM, centre_out_order, ensure_relocated_tower_clear,
        loops_bounds, plate_centre,
        relocate_keepout, relocate_wipe_tower_in_3mf, smallest_containing_zone,
        tower_corner_error_mm, union_rect,
    },
    progress::{self, RepackReason, Stage, expected_reslice_seconds, expected_slice_seconds},
    print_time::{
        TimeCorrection, TimeSample, WindowFit, corrected_seconds, count_for_window,
        shrink_count, slicer_change_seconds,
    },
    scheduler::{SliceEstimate, object_count_3mf, parse_bambu_result, parse_orca_gcode},
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
    max_objects: usize,
    seed: u64,
    /// Overrides the profile's edge margin (Snapmaker Orca's spiral lifting warns unless the
    /// gap is strictly above 3.5 mm, which a 3.5 margin hits exactly).
    edge_margin: Option<f64>,
    /// `search` runs the full deterministic packing; `grid` places the deterministic grid only
    /// and skips the search — faster, perfectly uniform, a bit sparser.
    layout: LayoutMode,
    nozzle_clean: Option<NozzleCleanSpec>,
    time: TimeOptions,
    /// `--support-overlap-mm`: how far the supports of different objects may overlap
    /// (`PackOptions::support_overlap_mm`); `None` = unlimited, the plain two-zone rule.
    support_overlap_mm: Option<f64>,
}

/// Time-model options (spec 2026-10-26-time-model-and-window-design). Without
/// `--time-window-hours` they only shape the corrected time written to the report.
#[derive(Clone, Debug, PartialEq)]
struct TimeOptions {
    window_hours: Option<f64>,
    ams_correction: bool,
    /// Overrides the profile's `filament_change_seconds`; setting it implies `ams_correction`.
    change_seconds: Option<f64>,
    margin_pct: f64,
    prep_min: f64,
    /// `--no-centre`: keep the corner sweep and the source tower position on a time-window
    /// plate (spec 2026-10-26-centre-out-zones-tower-design).
    no_centre: bool,
}

impl Default for TimeOptions {
    fn default() -> Self {
        // 10 % margin: the AMS calibration is a lower bound and the per-change cost varies
        // about ±15 % with the interval between changes.
        Self {
            window_hours: None,
            ams_correction: false,
            change_seconds: None,
            margin_pct: 10.0,
            prep_min: 0.0,
            no_centre: false,
        }
    }
}

impl TimeOptions {
    /// Centre-out ordering, heat zones and the tower relocation apply only to time-window
    /// plates: a max-count plate keeps today's corner sweep, since a centre tower fragments the
    /// free space and could cost objects.
    fn centre_out(&self) -> bool {
        self.window_hours.is_some() && !self.no_centre
    }

    /// The effective correction. A profile without `filament_change_seconds` (tool changer)
    /// has nothing to correct, so the change term stays off even when asked for.
    fn correction(&self, profile: &PrinterProfile) -> TimeCorrection {
        let real_change_s = if self.ams_correction {
            self.change_seconds.or(profile.filament_change_seconds)
        } else {
            None
        };
        TimeCorrection {
            real_change_s,
            prep_s: self.prep_min * 60.0,
            margin: self.margin_pct / 100.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LayoutMode {
    Search,
    Grid,
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
    /// Corrected print time of the written plate; `None` only when no window was requested
    /// and the slicer estimate could not be read (the plate itself is still valid).
    time: Option<TimeReport>,
    /// Smallest profile heat zone holding the kept objects plus the tower keepout; `None`
    /// without a time window, without profile zones, or when no zone contains the plate.
    heat_zone: Option<String>,
    /// Centre of the relocated tower keepout (centre-out window plates with a tower).
    tower_relocated_to: Option<[f64; 2]>,
    /// Measured distance of the first final slice's tower keepout min corner from the relocated
    /// keepout's min corner (the corner `wipe_tower_x/y` anchors; growth is one-sided).
    tower_relocation_error_mm: Option<f64>,
    /// The probe's first-layer patch reaches past the mesh silhouette (supports/brim), so the
    /// orientation is locked to the sliced one.
    supports_detected: bool,
    /// Echo of `--support-overlap-mm`; `null` = unlimited.
    support_overlap_mm: Option<f64>,
    /// Which pair rule packed the written plate: `landing` (foreign supports may reach under
    /// a silhouette, not onto its first-layer body), `silhouette` (the strict two-zone rule:
    /// no supports, or the fallback after the landing rule failed the G-code check).
    support_rule: SupportRule,
    /// The reserved support patch was widened to what the final slice really printed.
    support_patch_learned: bool,
    /// Supports touching a foreign body in the written plate's G-code (normally empty).
    support_collisions: Vec<SupportCollision>,
}

/// `time` block of the verification report. `samples` lists every plate whose time was
/// measured: without a window just the final plate; with one the probe (1 object), the max
/// plate and each window slice in order. `chosen_count`/`corrected_s` describe the written plate.
#[derive(Debug, Serialize)]
struct TimeReport {
    window_hours: Option<f64>,
    margin_pct: f64,
    real_change_s: Option<f64>,
    slicer_change_s: f64,
    prep_s: f64,
    samples: Vec<TimeSample>,
    chosen_count: usize,
    corrected_s: f64,
}

/// One slicer run: the G-code (tower and first-layer parsing) plus the slicer's own time
/// estimate. The estimate is kept as a Result so a plate whose summary cannot be parsed still
/// packs when no time window depends on it.
struct SliceOutput {
    gcode: Vec<u8>,
    estimate: Result<SliceEstimate, String>,
}

/// Window slices allowed after the max plate before giving up; each costs a full slice, and
/// the affine aim plus the shrink step normally land within one or two.
const MAX_WINDOW_SLICES: usize = 4;

/// Outcome of the window search when the window binds: the chosen prefix length and the
/// window slices measured on the way (the last one is the chosen plate).
#[derive(Debug, PartialEq)]
struct WindowChoice {
    count: usize,
    samples: Vec<TimeSample>,
}

fn hours(seconds: f64) -> f64 {
    seconds / 3600.0
}

fn even_one_exceeds(corrected_s: f64, window_s: f64) -> String {
    format!(
        "even one object exceeds the time window: {:.1} h > window {:.1} h",
        hours(corrected_s),
        hours(window_s)
    )
}

/// Spec steps 3-5: aim at the window with the affine line through the probe (1 object) and the
/// max plate, then measure that prefix and shrink while it is over. Under the window is
/// accepted without searching upward: the line already aimed at the boundary, and a
/// guaranteed fit beats one more object. `None` = the max plate fits (window does not bind).
/// `measure(n)` builds and slices the n-prefix and returns its corrected sample.
fn fit_window(
    probe: TimeSample,
    max: TimeSample,
    window_s: f64,
    mut measure: impl FnMut(usize) -> AppResult<TimeSample>,
) -> AppResult<Option<WindowChoice>> {
    if probe.corrected_s > window_s {
        return Err(even_one_exceeds(probe.corrected_s, window_s).into());
    }
    let mut count = match count_for_window(probe, max, window_s) {
        WindowFit::All => return Ok(None),
        // The probe fits (checked above), so the line through it cannot be over the window at
        // one object; NotEvenOne here means the max plate itself holds a single object and is
        // over. That measured plate is the evidence, not the probe.
        WindowFit::NotEvenOne => return Err(even_one_exceeds(max.corrected_s, window_s).into()),
        WindowFit::Count(count) => count,
    };
    let mut samples = Vec::new();
    loop {
        println!(
            "time window {:.1} h: max plate {} objects = {:.1} h corrected; trying {count}",
            hours(window_s),
            max.count,
            hours(max.corrected_s)
        );
        let sample = measure(count)?;
        samples.push(sample);
        if sample.corrected_s <= window_s {
            println!(
                "time window {:.1} h: {count} objects = {:.1} h corrected; fits",
                hours(window_s),
                hours(sample.corrected_s)
            );
            return Ok(Some(WindowChoice { count, samples }));
        }
        if count == 1 {
            return Err(even_one_exceeds(sample.corrected_s, window_s).into());
        }
        if samples.len() >= MAX_WINDOW_SLICES {
            return Err(format!(
                "time window {:.1} h not met after {MAX_WINDOW_SLICES} window slices; last: \
                 {count} objects = {:.1} h corrected",
                hours(window_s),
                hours(sample.corrected_s)
            )
            .into());
        }
        count = shrink_count(count, sample.corrected_s, window_s, shrink_slope(probe, max, sample));
    }
}

/// Seconds per object for the shrink step after a window slice overshot. The secant through
/// that window plate and the max plate, because those two were sliced with identical settings
/// (both from the assembled output: relocated tower, nozzle-clean wipe), while the probe is
/// sliced from the source (tower at the source corner, so every colour change travels
/// differently). Any per-plate offset the probe misses cancels in the secant. The probe line is
/// only the fallback when no such pair exists (the sample has the max plate's count).
fn shrink_slope(probe: TimeSample, max: TimeSample, sample: TimeSample) -> f64 {
    let secant = |a: TimeSample, b: TimeSample| {
        (a.corrected_s - b.corrected_s) / (a.count as f64 - b.count as f64)
    };
    if sample.count != max.count {
        secant(sample, max)
    } else if sample.count != probe.count {
        secant(sample, probe)
    } else {
        // No slope information at all; `shrink_count` steps by one on a non-positive slope.
        0.0
    }
}

fn time_sample(
    count: usize,
    estimate: &SliceEstimate,
    slicer_change_s: f64,
    correction: &TimeCorrection,
) -> TimeSample {
    let estimate_s = estimate.slicer_total_seconds as f64;
    TimeSample {
        count,
        estimate_s,
        changes: estimate.filament_changes,
        corrected_s: corrected_seconds(
            estimate_s,
            estimate.filament_changes,
            slicer_change_s,
            correction,
        ),
    }
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
    let mut profile = load_printer_profile(&args.profile_path)?;
    if let Some(edge_margin) = args.edge_margin {
        profile.edge_margin_mm = edge_margin;
    }
    validate_source_printer(&args.source_3mf, &profile)?;
    // Read once: the assembled output copies project_settings.config byte-for-byte, so the
    // source's per-change assumption holds for every plate sliced below.
    let slicer_change_s = slicer_change_seconds(&source_project_settings(&args.source_3mf)?);
    let correction = args.time.correction(&profile);
    if args.time.ams_correction && correction.real_change_s.is_none() {
        eprintln!(
            "note: profile '{}' has no filament_change_seconds (tool changer); AMS change \
             correction is off",
            profile.id
        );
    }
    let window_s = args.time.window_hours.map(|window_hours| window_hours * 3600.0);
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
    // With --nozzle-clean the probe slices a copy carrying the same wipe injection as every
    // final plate, so its time (and tower) is comparable to theirs; the wipe block is gated on
    // `layer_num > 0`, so the first-layer patch parsed below is unaffected. The probe still
    // prints the tower at the source position (the relocation is decided from this very
    // slice); the window's shrink step uses the window-vs-max secant, which cancels that.
    let probe_project = match &args.nozzle_clean {
        Some(spec) => {
            let copy = args.work_dir.join("probe_source.3mf");
            inject_nozzle_clean(&profile, &args.source_3mf, &copy, spec)
                .map_err(|error| format!("nozzle-clean injection failed: {error}"))?;
            copy
        }
        None => args.source_3mf.clone(),
    };
    progress::stage(
        Stage::ProbeSlice,
        "probing prime tower position (slicing 1 object)...",
        None,
        Some(1),
    );
    let probe_started = std::time::Instant::now();
    let probe = slice_project(&profile, &probe_project, 1, &args.work_dir, "probe")?;
    // Scales the expected duration of every later slice for the progress bar.
    let probe_seconds = probe_started.elapsed().as_secs_f64();
    // Spec step 1: with a window, a single object that already overruns it fails here, before
    // the expensive packing.
    let probe_sample = match window_s {
        Some(window_s) => {
            let estimate = probe
                .estimate
                .as_ref()
                .map_err(|error| format!("probe slice time estimate unavailable: {error}"))?;
            let sample = time_sample(1, estimate, slicer_change_s, &correction);
            if sample.corrected_s > window_s {
                return Err(even_one_exceeds(sample.corrected_s, window_s).into());
            }
            println!(
                "time window {:.1} h: 1 object = {:.1} h corrected ({} filament changes)",
                hours(window_s),
                hours(sample.corrected_s),
                sample.changes
            );
            Some(sample)
        }
        None => None,
    };
    let probe_gcode = probe.gcode;
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

    // The probe slice already shows the object exactly as it prints — body, tree supports,
    // brim. Packing against that first-layer patch stops supports from touching neighbouring
    // bodies, which the pure mesh projection could not promise (tree supports wander several
    // mm past the silhouette on a real lying-animal plate).
    let silhouette = extract_footprint(&footprint_stl)?;
    // Where the model's bbox center sits in the probe slice: the embedded mesh's center put
    // through the source build item's transform. The layer-1 patch must be centered exactly
    // there to stay in the assembler's placement frame.
    let model_world_center = model_center_on_bed(&args.source_3mf, &footprint_stl);
    let (mut footprint, lock_orientation, purge_tip_overshoot) =
        match first_layer_footprint(&probe_gcode, model_world_center) {
        Some(patch) => {
            eprintln!(
                "probe slice's first layer: patch {:.1}x{:.1} mm, mesh silhouette {:.1}x{:.1} mm",
                patch.width, patch.height, silhouette.width, silhouette.height
            );
            // Arc-fitted probes parse differently from straight-line ones (the patch is
            // rebuilt from flattened G2/G3 sweeps); the count tells the operator which kind
            // of gcode the patch below came from when a layout surprises them.
            let arc_moves = probe_gcode
                .split(|&byte| byte == b'\n')
                .filter(|line| line.starts_with(b"G2 ") || line.starts_with(b"G3 "))
                .count();
            if arc_moves > 0 {
                eprintln!(
                    "probe gcode carries {arc_moves} G2/G3 arc moves; the patch flattens them"
                );
            }
            // Supports and brim legitimately add a few mm, but a patch far larger than the
            // silhouette means foreign extrusions (prime tower, purge line) leaked into the
            // parsed first layer — every clone would reserve them and the layout would come
            // out sparse with no other error. This caught the Bambu dialect bug where the
            // tower inflated a 54 mm goat to a 97.5 mm patch.
            const FOREIGN_EXTRUSION_EXCESS_MM: f64 = 15.0;
            if patch.width > silhouette.width + FOREIGN_EXTRUSION_EXCESS_MM
                || patch.height > silhouette.height + FOREIGN_EXTRUSION_EXCESS_MM
            {
                eprintln!(
                    "WARNING: first-layer patch {:.1}x{:.1} mm exceeds the mesh silhouette \
                     {:.1}x{:.1} mm by more than {FOREIGN_EXTRUSION_EXCESS_MM} mm — foreign \
                     extrusions (prime tower / purge line) probably leaked into the patch; \
                     expecting a sparse layout",
                    patch.width, patch.height, silhouette.width, silhouette.height
                );
            }
            // A patch that stays inside the mesh outline carries no supports or brim beyond
            // the model, so the silhouette is the honest footprint — and it unlocks
            // rotations, which tile much denser than one locked orientation (a flat goat
            // plate: 31 with rotations, 28 locked). Supports past the outline keep the patch
            // and the lock: the slicer rebuilds them in table coordinates. The 2.0 mm
            // tolerance absorbs wall-bead artifacts (miter on sharp tips ~1 mm; arachne
            // slivers along a curved belly up to ~1.6 mm, 2.45 mm² total on the P1S goat);
            // real supports wander 3-5 mm past the outline with hundreds of mm².
            if patch_within_silhouette(&patch, &silhouette, args.resolution, 2.0) {
                eprintln!(
                    "patch fits inside the mesh silhouette (+2.0 mm) — no supports or brim \
                     reach past the model; packing the silhouette with rotations allowed"
                );
                // Within tolerance is not within: Arachne thin-tip lines print past the mesh
                // outline (measured up to 1.66 mm on a P1S goat's legs), and the prime line
                // is machine start gcode — nothing routes it around objects, so a goat whose
                // silhouette cleared the purge-line keepout printed its leg tips straight
                // through the line. The measured overshoot widens the PURGE-LINE keepout
                // only: the tower and exclusion corner keep their own validated safeties,
                // and object-vs-object clearance is untouched.
                let overshoot = patch_overshoot_mm(&patch, &silhouette, args.resolution);
                if overshoot > 0.0 {
                    eprintln!(
                        "first layer overshoots the mesh silhouette by up to {:.1} mm at thin \
                         tips; the purge-line keepout keeps that standoff",
                        overshoot
                    );
                }
                // Bed bounds still measure the printed patch: the overshoot at thin tips is
                // real ink past the silhouette, and a silhouette that clears the bed edge can
                // print outside it (see the bed_loops note after the match).
                let mut footprint = silhouette.clone();
                footprint.bed_loops = Some(union_loops(&patch, &silhouette));
                (footprint, false, overshoot)
            } else {
                // How far the patch actually sticks out separates the two realities this
                // branch exists for: real supports/brim wander whole millimetres, while a
                // fraction of a millimetre is a parse artifact (a stray chord, a mitered
                // tip) worth a second look before accepting the locked-orientation layout.
                let overshoot = patch_overshoot_mm(&patch, &silhouette, args.resolution);
                eprintln!(
                    "patch reaches past the mesh silhouette by up to {overshoot:.1} mm \
                     (supports/brim) — packing the patch ∪ silhouette, orientation locked to \
                     the sliced one"
                );
                // Collisions must respect BOTH truths: the printed patch (supports/brim
                // beyond the outline) AND the mesh body where layer 1 prints nothing (a
                // lying animal's raised horns; on P1S the patch is inset ~0.4 mm per side,
                // so a bare-patch layout glued 24 goat pairs by up to 1.7 mm²). The body
                // zone is the mesh outline — walls and walls' strips are all inset
                // representations of the same body — and silhouettes keep clearance from
                // each other. A neighbour's supports only must not land on what this object
                // prints on the bed: its first-layer body (`support_landing_loops`; see the
                // field for the 40-goat plate that proved the silhouette rule too strict).
                let mut combined = patch.clone();
                combined.loops = union_loops(&patch, &silhouette);
                combined.body_loops = silhouette.loops.clone();
                combined.support_landing_loops = patch.body_loops.clone();
                // The patch is the measured print (tips included), so purge checks run
                // against its real extent already — no extra overshoot.
                (combined, true, 0.0)
            }
        }
        None => {
            eprintln!(
                "probe slice's first layer has no object extrusions; packing against the mesh \
                 silhouette ({:.1}x{:.1} mm)",
                silhouette.width, silhouette.height
            );
            // No probe data to measure the tip overshoot from; the base 0.5 mm purge safety
            // applies, as it always did in this degraded path.
            (silhouette.clone(), false, 0.0)
        }
    };
    // Centre-out time-window plate (spec 2026-10-26-centre-out-zones-tower-design): the kept
    // objects gather around the smallest heat zone's centre, and the tower moves there too so
    // every colour change travels a short way. The tower's size does not depend on its
    // position, so the probe keepout shifted by the same vector is the reservation — no
    // re-probe; the first final slice verifies the slicer really moved it.
    let centre = if args.time.centre_out() {
        Some(plate_centre(&profile.heat_zones, profile.usable_bed()?))
    } else {
        None
    };
    let fixed_keepouts = profile.fixed_keepouts(purge_tip_overshoot)?;
    let usable_bed = profile.usable_bed()?;
    let centre_relocation: Option<RelocatedTower> = match (centre, probe_tower) {
        (Some(centre), Some(tower)) => {
            let moved = relocate_keepout(tower.keepout, centre, usable_bed);
            // The clamp only knows the bed; a centre near an exclusion corner or purge line
            // would put the printed tower on it.
            ensure_relocated_tower_clear(moved.keepout, &fixed_keepouts)?;
            println!(
                "centre-out plate around ({:.1}, {:.1}): prime tower moves by ({:+.1}, {:+.1}) mm; \
                 keepout {:.2},{:.2} → {:.2},{:.2}",
                centre.0,
                centre.1,
                moved.shift.0,
                moved.shift.1,
                moved.keepout.min_x,
                moved.keepout.min_y,
                moved.keepout.max_x,
                moved.keepout.max_y
            );
            Some(moved)
        }
        (Some(centre), None) => {
            println!(
                "centre-out plate around ({:.1}, {:.1}); no tower to relocate",
                centre.0, centre.1
            );
            None
        }
        (None, _) => None,
    };
    // Bed bounds and the prime-tower keepout follow the patch UNION the whole-model
    // silhouette: the slicer's bed-collision warning measures the full model extent, which
    // hangs below the printed patch on a lying animal (its tail), silently eating the edge
    // margin — the manual 3.5/4.0 mm gap complaint. Every branch above has already set
    // bed_loops to that union: with rotations allowed the patch carries the measured tip
    // overshoot (~2 mm of Arachne miter past the silhouette at thin tips), and a layout
    // whose silhouette clears the bed edge can still print outside it — BambuStudio then
    // refuses the whole final slice with "too close to exclusion area" / "no object fully
    // inside it" (a real 17-object P1S goat plate died at mesh x=0.06 and x=255.7 on a
    // 256 mm bed this way).
    if footprint.bed_loops.is_none() {
        footprint.bed_loops = Some(footprint.loops.clone());
    }
    let mut plan = PackPlan {
        footprint,
        lock_orientation,
        fixed_keepouts,
        tower_keepout: centre_relocation
            .map(|relocated| relocated.keepout)
            .or(probe_tower.map(|tower| tower.keepout)),
        relocation: centre_relocation,
        rule: if lock_orientation {
            SupportRule::Landing
        } else {
            SupportRule::Silhouette
        },
        patch_learned: false,
    };
    progress::stage(
        Stage::Packing,
        &format!("packing ({} attempts, seed {})...", args.attempts, args.seed),
        None,
        None,
    );
    // Without centre-out the tower may move: the source position competes with the bed
    // corners and the densest plate wins (spec 2026-10-31 rule 3).
    let mut tower_from_contest = false;
    let mut packed = match (centre, probe_tower) {
        (None, Some(tower)) => {
            let (relocation, packed) = choose_tower_position(&args, &profile, &plan, tower.keepout)?;
            if let Some(relocated) = relocation {
                plan.tower_keepout = Some(relocated.keepout);
                plan.relocation = Some(relocated);
                tower_from_contest = true;
            }
            packed
        }
        _ => plan.pack(&args, &profile)?,
    };
    let mut tower_regrown = false;
    let mut repacked = false;
    let mut final_slice;
    let mut final_tower;
    let mut tower_relocation_error_mm;
    let mut support_collisions;
    // (seconds, objects) of the last full-plate slice: later slices are expected to scale it.
    let mut measured_slice: Option<(f64, usize)> = None;
    let expected_plate_seconds = |measured: Option<(f64, usize)>, count: usize| match measured {
        Some((seconds, objects)) => expected_reslice_seconds(seconds, objects, count),
        None => expected_slice_seconds(probe_seconds, count),
    };
    loop {
        if let Some(centre) = centre {
            order_centre_out(&plan.footprint, &mut packed, centre);
        }
        let count = packed.placements.len();
        progress::stage(
            Stage::Assembling,
            &format!("packed {count} objects; assembling 3MF..."),
            None,
            Some(count),
        );
        plan.build(&args, &profile, &footprint_stl, &packed)?;
        progress::stage(
            Stage::FinalSlice,
            &format!("slicing final plate ({count} objects)..."),
            Some(expected_plate_seconds(measured_slice, count)),
            Some(count),
        );
        let label = if repacked { "final_repacked" } else { "final_first" };
        let slice_started = std::time::Instant::now();
        final_slice = match slice_project(&profile, &args.output, count, &args.work_dir, label) {
            Ok(slice) => {
                measured_slice = Some((slice_started.elapsed().as_secs_f64(), count));
                slice
            }
            // A tower the contest moved to a bed corner is the one untested input here: when
            // the slicer rejects that plate, the source position (which the probe sliced
            // fine) gets one repack before the error is final.
            Err(error) if tower_from_contest => {
                println!(
                    "final slice failed with the tower moved to a corner; repacking with the \
                     tower at its source position\n  {}",
                    error.to_string().lines().take(2).collect::<Vec<_>>().join("\n  ")
                );
                tower_from_contest = false;
                plan.relocation = None;
                plan.tower_keepout = probe_tower.map(|tower| tower.keepout);
                repacked = true;
                progress::repack(
                    RepackReason::TowerCornerFailed,
                    &format!("re-packing ({} attempts, seed {})...", args.attempts, args.seed),
                );
                packed = plan.pack(&args, &profile)?;
                continue;
            }
            Err(error) => return Err(error),
        };
        final_tower =
            parse_prime_tower_bounds(&final_slice.gcode, profile.tower_line_width_mm, args.clearance);
        // Checked before the tower evaluation: a slicer that silently dropped the relocation
        // (e.g. a setting reset to the system value because it is not listed in
        // `different_settings_to_system`) prints the tower at the source corner, which would
        // read as a "grown" tower and trigger a pointless repack instead of naming the cause.
        tower_relocation_error_mm = None;
        if let (Some(relocated), Some(tower)) = (plan.relocation, final_tower) {
            let error = tower_corner_error_mm(tower.keepout, relocated.keepout);
            let (mx, my) = (tower.keepout.min_x, tower.keepout.min_y);
            if error > TOWER_TARGET_TOLERANCE_MM {
                return Err(format!(
                    "slicer ignored the relocated wipe tower (wipe_tower_x/y): final tower corner \
                     ({mx:.1}, {my:.1}) is {error:.1} mm from the target ({:.1}, {:.1})",
                    relocated.keepout.min_x, relocated.keepout.min_y,
                )
                .into());
            }
            println!(
                "relocated prime tower confirmed in the final slice: corner ({mx:.1}, {my:.1}), \
                 {error:.1} mm from the target"
            );
            tower_relocation_error_mm = Some(error);
        }
        if evaluate_final_tower(probe_tower, final_tower, plan.tower_keepout)?
            == TowerOutcome::RepackRequired
        {
            let grown_keepout = final_tower.unwrap().keepout;
            // The probe tower is sized by ONE object's colour changes; a full plate's tower
            // grows. But only growth that physically reaches a placement's body invalidates
            // the layout (supports may enter the tower area, see `body_exclusions`):
            // repacking on bare keepout arithmetic cost a real object (29 → 28) on plates
            // where the grown strip hit nothing at all. The final slice already ran clean on
            // this exact file, so an untouched layout is already validated.
            let collision = packed.placements.iter().position(|placement| {
                loops_touch_rect(&placement_body_loops(&plan.footprint, placement), grown_keepout)
            });
            if let Some(index) = collision {
                if tower_regrown {
                    return Err("prime tower changed again after the permitted re-pack".into());
                }
                tower_regrown = true;
                repacked = true;
                println!(
                    "final tower outgrew the probe keepout and reaches placement {index}; \
                     repacking once"
                );
                plan.tower_keepout = Some(grown_keepout);
                progress::repack(
                    RepackReason::TowerGrown,
                    &format!("re-packing ({} attempts, seed {})...", args.attempts, args.seed),
                );
                packed = plan.pack(&args, &profile)?;
                continue;
            }
            println!(
                "final tower outgrew the probe keepout (y up to {:.1} mm) but reaches no \
                 placement; keeping the layout",
                grown_keepout.max_y
            );
        }
        progress::stage(
            Stage::GcodeCheck,
            &format!("checking supports layer by layer ({count} objects)..."),
            Some(5.0),
            Some(count),
        );
        let analysis = analyze_plate(&final_slice.gcode);
        support_collisions = analysis.collisions.clone();
        if analysis.labelled_lines == 0 {
            // Without object labels (slicer's label_objects off) lines cannot be attributed;
            // say so instead of reporting a clean plate.
            println!("support check skipped: the G-code carries no object labels");
            break;
        }
        if support_collisions.is_empty() {
            println!(
                "support check: {} layers, no support touches a foreign body",
                analysis.layers
            );
            break;
        }
        describe_collisions(&support_collisions);
        if !plan.lock_orientation {
            // No support zone was reserved at all (support-free probe): nothing to learn.
            break;
        }
        // The slicer re-generated bigger supports than the probe showed (the realization
        // drifts between slices). First learn what it really prints and repack; if that is
        // not enough either, fall back to the strict rule (supports kept off the whole
        // foreign silhouette) — what the packer always did before the landing rule.
        let reason = if !plan.patch_learned {
            RepackReason::SupportContact
        } else {
            RepackReason::SupportStrict
        };
        if !plan.patch_learned {
            let learned = learn_support_patch(&plan.footprint, &packed, &analysis)?;
            println!(
                "support check: learned the final slice's support patch ({:.1}x{:.1} mm, \
                 probe {:.1}x{:.1} mm); repacking",
                learned.width, learned.height, plan.footprint.width, plan.footprint.height
            );
            plan.footprint = learned;
            plan.patch_learned = true;
        } else if plan.rule == SupportRule::Landing {
            println!(
                "support check: still touching after learning the patch; repacking with the \
                 strict rule (supports kept off foreign silhouettes)"
            );
            plan.rule = SupportRule::Silhouette;
        } else {
            eprintln!(
                "WARNING: supports still touch foreign bodies under the strict rule; keeping \
                 the layout (listed in the report as support_collisions)"
            );
            break;
        }
        repacked = true;
        progress::repack(
            reason,
            &format!("re-packing ({} attempts, seed {})...", args.attempts, args.seed),
        );
        packed = plan.pack(&args, &profile)?;
    }

    let validated_tower_keepout = plan.tower_keepout;
    let time = match (window_s, probe_sample) {
        (Some(window_s), Some(probe_sample)) => {
            let estimate = final_slice
                .estimate
                .as_ref()
                .map_err(|error| format!("final slice time estimate unavailable: {error}"))?;
            let max_sample =
                time_sample(packed.placements.len(), estimate, slicer_change_s, &correction);
            let full = packed.clone();
            let mut window_tower = None;
            let mut window_collisions = None;
            let choice = fit_window(probe_sample, max_sample, window_s, |count| {
                // A prefix of a valid layout is valid (same rule as the scheduler's
                // subset_3mf), and packing order puts the kept objects together.
                let mut prefix = full.clone();
                prefix.placements.truncate(count);
                plan.build(&args, &profile, &footprint_stl, &prefix)?;
                progress::stage(
                    Stage::WindowSlice,
                    &format!("slicing time-window plate ({count} objects)..."),
                    Some(expected_plate_seconds(measured_slice, count)),
                    Some(count),
                );
                let slice = slice_project(
                    &profile,
                    &args.output,
                    count,
                    &args.work_dir,
                    &format!("final_window_{count}"),
                )?;
                let tower = parse_prime_tower_bounds(
                    &slice.gcode,
                    profile.tower_line_width_mm,
                    args.clearance,
                );
                if evaluate_final_tower(probe_tower, tower, validated_tower_keepout)?
                    == TowerOutcome::RepackRequired
                {
                    // Fewer objects must not grow the tower; if it did and reaches a kept
                    // placement, the layout's collision checks are void. A growth that hits
                    // nothing is accepted for the same reason as on the max plate.
                    let grown = tower.unwrap().keepout;
                    if prefix.placements.iter().any(|placement| {
                        loops_touch_rect(&placement_body_loops(&plan.footprint, placement), grown)
                    }) {
                        return Err(format!(
                            "prime tower of the {count}-object time-window plate outgrew the \
                             reserved keepout and reaches a placement"
                        )
                        .into());
                    }
                }
                let estimate = slice.estimate.map_err(|error| {
                    format!("time-window slice ({count} objects) estimate unavailable: {error}")
                })?;
                window_tower = Some(tower);
                // A prefix drops objects, and the slicer re-generates every support: check the
                // plate actually written, report (not repack) what still touches.
                let collisions = analyze_plate(&slice.gcode).collisions;
                describe_collisions(&collisions);
                window_collisions = Some(collisions);
                Ok(time_sample(count, &estimate, slicer_change_s, &correction))
            })?;
            let mut samples = vec![probe_sample, max_sample];
            let chosen = match choice {
                Some(choice) => {
                    packed.placements.truncate(choice.count);
                    final_tower = window_tower.expect("a chosen window plate was sliced");
                    support_collisions =
                        window_collisions.expect("a chosen window plate was checked");
                    samples.extend(choice.samples);
                    *samples.last().expect("window samples are non-empty")
                }
                None => {
                    println!(
                        "time window {:.1} h: max plate {} objects = {:.1} h corrected; fits",
                        hours(window_s),
                        max_sample.count,
                        hours(max_sample.corrected_s)
                    );
                    max_sample
                }
            };
            Some(TimeReport {
                window_hours: args.time.window_hours,
                margin_pct: args.time.margin_pct,
                real_change_s: correction.real_change_s,
                slicer_change_s,
                prep_s: correction.prep_s,
                samples,
                chosen_count: chosen.count,
                corrected_s: chosen.corrected_s,
            })
        }
        _ => match &final_slice.estimate {
            Ok(estimate) => {
                let sample =
                    time_sample(packed.placements.len(), estimate, slicer_change_s, &correction);
                Some(TimeReport {
                    window_hours: None,
                    margin_pct: args.time.margin_pct,
                    real_change_s: correction.real_change_s,
                    slicer_change_s,
                    prep_s: correction.prep_s,
                    samples: vec![sample],
                    chosen_count: sample.count,
                    corrected_s: sample.corrected_s,
                })
            }
            Err(error) => {
                eprintln!(
                    "note: final slice time estimate unavailable ({error}); report has no time"
                );
                None
            }
        },
    };

    // Zone report for window plates only: a max-count plate fills the bed by construction.
    let heat_zone = if window_s.is_some() {
        let (bbox, zone) =
            heat_zone_of(&profile, &plan.footprint, &packed, validated_tower_keepout);
        if let (false, Some(bbox)) = (profile.heat_zones.is_empty(), bbox) {
            println!(
                "heat zone: {} (objects + tower x [{:.1}, {:.1}], y [{:.1}, {:.1}])",
                zone.as_deref().unwrap_or("none"),
                bbox.min_x,
                bbox.max_x,
                bbox.min_y,
                bbox.max_y
            );
        }
        zone
    } else {
        None
    };
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
        time,
        heat_zone,
        tower_relocated_to: plan.relocation.map(|relocated| {
            [
                (relocated.keepout.min_x + relocated.keepout.max_x) * 0.5,
                (relocated.keepout.min_y + relocated.keepout.max_y) * 0.5,
            ]
        }),
        tower_relocation_error_mm,
        // lock_orientation is set exactly in the "patch past silhouette (supports/brim)"
        // branch: the patch carries a support zone beyond the body.
        supports_detected: lock_orientation,
        support_overlap_mm: args.support_overlap_mm,
        support_rule: plan.rule,
        support_patch_learned: plan.patch_learned,
        support_collisions,
    };
    progress::stage(Stage::Writing, "writing the verification report...", None, None);
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

/// The footprint loops of one placement in bed coordinates — rotated by the placement's
/// angle, then translated to its position. For checking a placed object against a
/// bed-space rect such as a grown tower keepout.
fn placement_world_loops(
    footprint: &Footprint,
    placement: &goat_packer::PackedPlacement,
) -> Vec<Vec<Point>> {
    let radians = placement.angle_deg.to_radians();
    let (sin, cos) = radians.sin_cos();
    footprint
        .loops
        .iter()
        .map(|loop_| {
            loop_
                .iter()
                .map(|p| Point {
                    x: p.x * cos - p.y * sin + placement.x_mm,
                    y: p.x * sin + p.y * cos + placement.y_mm,
                })
                .collect()
        })
        .collect()
}

/// World-XY bbox of one placement's footprint loops (`None` only for an empty footprint).
fn placement_world_bounds(
    footprint: &Footprint,
    placement: &goat_packer::PackedPlacement,
) -> Option<Rect> {
    loops_bounds(&placement_world_loops(footprint, placement))
}

/// Reorders the layout centre-out: by the distance from `centre` to each placement's world
/// footprint bbox centre, ties by packing order. Any subset of a valid layout is valid, so the
/// time window's prefix then always drops the farthest object first.
fn order_centre_out(footprint: &Footprint, packed: &mut PackResult, centre: (f64, f64)) {
    let centres: Vec<(f64, f64)> = packed
        .placements
        .iter()
        .map(|placement| {
            placement_world_bounds(footprint, placement).map_or(
                (placement.x_mm, placement.y_mm),
                |bounds| {
                    ((bounds.min_x + bounds.max_x) * 0.5, (bounds.min_y + bounds.max_y) * 0.5)
                },
            )
        })
        .collect();
    let order = centre_out_order(&centres, centre);
    packed.placements = order.into_iter().map(|index| packed.placements[index].clone()).collect();
}

/// Spec "zone report": bbox of the kept placements ∪ the tower keepout (the first layer the
/// A2L start G-code heat-soaks) and the smallest heat zone containing it.
fn heat_zone_of(
    profile: &PrinterProfile,
    footprint: &Footprint,
    packed: &PackResult,
    tower_keepout: Option<Rect>,
) -> (Option<Rect>, Option<String>) {
    let bbox = packed
        .placements
        .iter()
        .filter_map(|placement| placement_world_bounds(footprint, placement))
        .chain(tower_keepout)
        .reduce(union_rect);
    let zone = bbox
        .and_then(|bbox| smallest_containing_zone(&profile.heat_zones, bbox))
        .map(|zone| zone.name.clone());
    (bbox, zone)
}

/// The source model's bbox center in bed coordinates: the baked STL's center transformed by
/// the source 3MF's build item. Falls back to None (patch centers itself) when the archive
/// cannot be read — the probe slice still packs, just with the strip-center caveat.
fn model_center_on_bed(source_3mf: &Path, baked_stl: &Path) -> Option<(f64, f64)> {
    let (cx, cy, cz) = stl_center_3d(baked_stl).ok()?;
    let core = read_core_entry(source_3mf).ok()?;
    source_model_center(&core, (cx, cy, cz)).ok()
}

fn read_core_entry(source_3mf: &Path) -> AppResult<Vec<u8>> {
    let file = fs::File::open(source_3mf)?;
    let mut archive = ZipArchive::new(file)?;
    let mut entry = archive.by_name("3D/3dmodel.model")?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes)?;
    Ok(bytes)
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

/// Which pair rule the packer applies between two objects' supports and bodies.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SupportRule {
    /// A foreign patch must not touch the first-layer body (`support_landing_loops`);
    /// silhouettes keep clearance from each other.
    Landing,
    /// The plain two-zone rule: a foreign patch must not touch the whole silhouette. Used
    /// for support-free footprints and as the fallback when the landing rule fails the
    /// final G-code check twice.
    Silhouette,
}

/// Everything one packing pass depends on; the verify loop in `run` mutates it between passes
/// (grown tower, learned support patch, strict rule).
struct PackPlan {
    footprint: Footprint,
    lock_orientation: bool,
    /// Bed exclusion corners and purge-line strips: the full-extent bbox must clear them.
    fixed_keepouts: Vec<Rect>,
    /// The reserved prime-tower area: only the model body must clear it (supports purge with
    /// the tower area, see `PackOptions::body_exclusions`).
    tower_keepout: Option<Rect>,
    /// Set when the tower moves (centre-out plate or a better bed corner): the keepout the
    /// final slice is checked against and the `wipe_tower_x/y` shift.
    relocation: Option<RelocatedTower>,
    rule: SupportRule,
    patch_learned: bool,
}

impl PackPlan {
    /// The footprint the current rule packs: under the strict rule the landing zone is the
    /// whole silhouette again.
    fn rule_footprint(&self) -> Footprint {
        let mut footprint = self.footprint.clone();
        if self.rule == SupportRule::Silhouette {
            footprint.support_landing_loops.clear();
        }
        footprint
    }

    fn keepouts(&self) -> Vec<Rect> {
        self.fixed_keepouts.iter().copied().chain(self.tower_keepout).collect()
    }

    fn pack(&self, args: &Args, profile: &PrinterProfile) -> AppResult<PackResult> {
        pack(args, profile, &self.rule_footprint(), self.lock_orientation, &self.fixed_keepouts, self.tower_keepout, true)
    }

    fn build(
        &self,
        args: &Args,
        profile: &PrinterProfile,
        footprint_stl: &Path,
        packed: &PackResult,
    ) -> AppResult<()> {
        build_colored_project(
            args,
            profile,
            footprint_stl,
            &self.footprint,
            packed,
            &self.keepouts(),
            self.relocation.map(|relocated| relocated.shift),
        )
    }
}

/// Fraction of the probe tower's depth reserved above a corner-placed tower for its growth
/// on the full plate: a P1S goat tower grew 18.5 → 23.2 mm (×1.25) from 1 to 40 objects, and
/// the tower grows one-sided from `wipe_tower_y`. A top corner without this room would push
/// the grown tower off the bed.
const CORNER_TOWER_GROWTH: f64 = 0.3;

/// Gap between a corner-placed tower keepout and the usable bed edge. A keepout flush with
/// the bed edge failed BambuStudio's final slice ("Found G-code outside of the printable
/// area ... try moving the wipe tower further inside": it checks the tower with conservative
/// wipe/brim margins), while the user's hand-placed P1S corner tower printed with its keepout
/// 5.2 mm from the right edge and 8 mm from the top. A slice that still fails with a moved
/// tower falls back to the source position (see `run`).
const CORNER_TOWER_INSET_MM: f64 = 6.0;

/// Spec 2026-10-31 rule 3: the source tower position competes with the four usable-bed
/// corners; the densest plate wins, ties keep the source position (then the corner order).
/// A corner keepout reserves the growth room above the tower (`CORNER_TOWER_GROWTH`), and a
/// corner on a fixed keepout (P1S exclusion corner, purge line) is skipped; corners keep
/// `CORNER_TOWER_INSET_MM` from the bed edge. Returns the chosen
/// relocation (`None` = source position) with its packed layout.
fn choose_tower_position(
    args: &Args,
    profile: &PrinterProfile,
    plan: &PackPlan,
    probe_keepout: Rect,
) -> AppResult<(Option<RelocatedTower>, PackResult)> {
    let bed = profile.usable_bed()?;
    let footprint = plan.rule_footprint();
    let width = probe_keepout.max_x - probe_keepout.min_x;
    let depth = (probe_keepout.max_y - probe_keepout.min_y) * (1.0 + CORNER_TOWER_GROWTH);
    let corner = |name: &'static str, min_x: f64, min_y: f64| {
        let keepout = Rect::new(min_x, min_y, min_x + width, min_y + depth);
        let shift = (min_x - probe_keepout.min_x, min_y - probe_keepout.min_y);
        (name, Some(RelocatedTower { keepout, shift }))
    };
    let inset = CORNER_TOWER_INSET_MM;
    let (left, right) = (bed.min_x + inset, bed.max_x - inset - width);
    let (bottom, top) = (bed.min_y + inset, bed.max_y - inset - depth);
    let candidates = [
        ("source position", None),
        corner("bottom-left corner", left, bottom),
        corner("bottom-right corner", right, bottom),
        corner("top-left corner", left, top),
        corner("top-right corner", right, top),
    ];
    let mut best: Option<(&str, Option<RelocatedTower>, PackResult)> = None;
    for (name, relocation) in candidates {
        let keepout = relocation.map_or(probe_keepout, |relocated| relocated.keepout);
        if relocation.is_some()
            && (keepout.min_x < bed.min_x - 1e-9
                || keepout.min_y < bed.min_y - 1e-9
                || keepout.max_x > bed.max_x + 1e-9
                || keepout.max_y > bed.max_y + 1e-9
                || ensure_relocated_tower_clear(keepout, &plan.fixed_keepouts).is_err())
        {
            continue;
        }
        let packed = pack(
            args,
            profile,
            &footprint,
            plan.lock_orientation,
            &plan.fixed_keepouts,
            Some(keepout),
            false,
        )?;
        println!("tower at {name}: {} objects", packed.placements.len());
        if best
            .as_ref()
            .is_none_or(|(_, _, current)| packed.placements.len() > current.placements.len())
        {
            best = Some((name, relocation, packed));
        }
    }
    let (name, relocation, packed) = best.expect("the source position is always a candidate");
    if let Some(relocated) = relocation {
        println!(
            "prime tower moves to the {name} by ({:+.1}, {:+.1}) mm: keepout {:.2},{:.2} → \
             {:.2},{:.2}",
            relocated.shift.0,
            relocated.shift.1,
            relocated.keepout.min_x,
            relocated.keepout.min_y,
            relocated.keepout.max_x,
            relocated.keepout.max_y
        );
    }
    Ok((relocation, packed))
}

/// The body loops of one placement in bed coordinates (the whole footprint when it has no
/// separate body): what a grown tower must not reach.
fn placement_body_loops(
    footprint: &Footprint,
    placement: &goat_packer::PackedPlacement,
) -> Vec<Vec<Point>> {
    let mut body = footprint.clone();
    if !body.body_loops.is_empty() {
        body.loops = body.body_loops.clone();
    }
    placement_world_loops(&body, placement)
}

fn describe_collisions(collisions: &[SupportCollision]) {
    for collision in collisions {
        println!(
            "support check: support of object {} touches object {} on layers {}..{} ({} \
             layers, gap {:.2} mm)",
            collision.support_object,
            collision.body_object,
            collision.first_layer,
            collision.last_layer,
            collision.layers,
            collision.min_gap_mm
        );
    }
}

/// `learn_support_patch` against the current layout, with the context an operator needs.
fn learn_support_patch(
    footprint: &Footprint,
    packed: &PackResult,
    analysis: &PlateAnalysis,
) -> AppResult<Footprint> {
    goat_packer::gcode_collisions::learn_support_patch(
        footprint,
        &packed.placements,
        &analysis.first_layer,
    )
    .map_err(|error| format!("support check: {error}").into())
}

/// One packing pass: the deterministic grid as a floor, then (`--layout search`) the search.
/// `verbose` = log the grid baseline and layout bounds (off for the tower-position contest,
/// whose candidates print one line each).
fn pack(
    args: &Args,
    profile: &PrinterProfile,
    footprint: &Footprint,
    lock_orientation: bool,
    fixed_keepouts: &[Rect],
    tower_keepout: Option<Rect>,
    verbose: bool,
) -> AppResult<PackResult> {
    let options = PackOptions {
        body_exclusions: tower_keepout.into_iter().collect(),
        bed: profile.usable_bed()?,
        exclusions: fixed_keepouts.to_vec(),
        resolution: args.resolution,
        clearance: args.clearance,
        angle_step_deg: args.angle_step,
        attempts: args.attempts,
        candidates_per_step: args.candidates,
        max_objects: args.max_objects,
        // The probe-derived patch is only valid at the orientation it was sliced at; rotating
        // it would let the slicer's rebuilt tree supports wander outside the reserved area.
        lock_orientation,
        stall_attempts: args.stall,
        seed: args.seed,
        // Only bites when the footprint has a support zone (the locked patch ∪ silhouette
        // branch); a silhouette footprint has body == full and the option is a no-op.
        support_overlap_mm: args.support_overlap_mm,
    };
    // The deterministic grid runs first as a floor score: small models tile as densely as
    // their bounding boxes allow, and the deterministic search only wins when it beats that.
    // Taking the grid on a tie keeps the layout reproducible. `--layout grid` stops right
    // here — no deterministic search at all.
    let grid = grid_layout(&footprint, &options);
    let result = match args.layout {
        LayoutMode::Grid => {
            if verbose {
                eprintln!(
                    "grid layout: {} objects (deterministic search skipped)",
                    grid.placements.len()
                );
            }
            grid
        }
        LayoutMode::Search => {
            if verbose {
                eprintln!(
                    "grid baseline: {} objects; deterministic search needs more to win",
                    grid.placements.len()
                );
            }
            let searched = pack_footprint(&footprint, &options);
            if searched.placements.len() > grid.placements.len() {
                searched
            } else {
                grid
            }
        }
    };
    validate_result(&footprint, &options, &result)?;
    if !verbose {
        return Ok(result);
    }
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
    footprint: &Footprint,
    packed: &PackResult,
    keepouts: &[Rect],
    tower_shift: Option<(f64, f64)>,
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
        footprint_override: Some(footprint.clone()),
    };
    assemble_project(&args.source_3mf, &args.output, &options)
        .map_err(|error| format!("3MF assembly failed: {error}"))?;
    // The one documented exception to "project_settings.config is copied byte-for-byte": a
    // centre-out window plate moves the tower next to its objects, and the tower position IS a
    // project setting (`wipe_tower_x/y[0]`). Applied to the output, so the CLI copies sliced
    // below and the written project carry the same tower the keepout reserved. Each build
    // re-assembles from the source, so the shift never accumulates.
    if let Some(shift) = tower_shift {
        let (x, y) = relocate_wipe_tower_in_3mf(&args.output, shift)
            .map_err(|error| format!("wipe tower relocation failed: {error}"))?;
        eprintln!("wipe tower relocated: wipe_tower_x/y[0] = {x:.3}, {y:.3}");
    }
    // Applied after assembly (both here and on the single repack path): the CLI copies sliced
    // for final/window checks derive from this same file, so their timings and tower state
    // reflect the real, wipe-carrying G-code (the probe gets its own injected copy in `run`).
    if let Some(spec) = &args.nozzle_clean {
        inject_nozzle_clean(profile, &args.output, &args.output, spec)
            .map_err(|error| format!("nozzle-clean injection failed: {error}"))?;
        // Loud on purpose: an invisible injection once read as "the checkbox does nothing" —
        // the job log must state plainly that the wipe macro reached the output file.
        eprintln!(
            "nozzle clean injected into layer_change_gcode ({}) of {}",
            spec.describe(),
            args.output.display()
        );
    }
    Ok(())
}

fn slice_project(
    profile: &PrinterProfile,
    project: &Path,
    count: usize,
    work_dir: &Path,
    label: &str,
) -> AppResult<SliceOutput> {
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
    let log_path = work_dir.join(format!("{label}_slice.log"));
    fs::write(&log_path, [&output.stdout[..], &output.stderr[..]].concat())?;
    if !output.status.success() {
        // BambuStudio/Orca print their real diagnosis to stdout (and to result.json in the
        // output dir), not to stderr — reporting only stderr yielded an empty "slicing failed:"
        // message on real plates. Surface the most specific source first, plus enough context
        // to re-run the exact failing command by hand.
        let report_error = slicer_report_error(&output_dir)
            .unwrap_or_else(|| "<slicer wrote no result.json>".to_owned());
        // Rendered from the Command itself: a hand-written copy dropped `--arrange 0`, and a
        // rerun without it lets Orca re-arrange the plate, i.e. reproduces a different slice.
        let rerun = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        return Err(format!(
            "{} slicing failed ({})\n\
             slicer report: {report_error}\n\
             slicer output (tail):\n{}\n\
             full log: {}\n\
             output dir: {}\n\
             rerun manually: {rerun}",
            profile.printer_name,
            output.status,
            output_tail(&output.stdout, &output.stderr, 15),
            log_path.display(),
            output_dir.display(),
        )
        .into());
    }
    let gcode_path = fs::read_dir(&output_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "gcode"))
        .ok_or("slicer output did not contain G-code")?;
    let gcode = fs::read(gcode_path)?;
    // Same sources as the scheduler's benchmark: BambuStudio writes its summary (with the
    // change count) to result.json, Orca only leaves the estimate in the G-code.
    let estimate = if profile.slicer_kind == "bambu" {
        fs::read(output_dir.join("result.json"))
            .map_err(|error| format!("cannot read result.json: {error}"))
            .and_then(|bytes| parse_bambu_result(&bytes).map_err(|error| error.to_string()))
    } else {
        parse_orca_gcode(&gcode, count).map_err(|error| error.to_string())
    };
    Ok(SliceOutput { gcode, estimate })
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
            changed = apply_cli_settings_workarounds(profile, &mut settings);
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

fn source_project_settings(source: &Path) -> AppResult<serde_json::Value> {
    let mut archive = ZipArchive::new(fs::File::open(source)?)?;
    let mut settings = Vec::new();
    archive
        .by_name("Metadata/project_settings.config")?
        .read_to_end(&mut settings)?;
    Ok(serde_json::from_slice(&settings)?)
}

fn validate_source_printer(source: &Path, profile: &PrinterProfile) -> AppResult<()> {
    let configured = source_project_settings(source)?;
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
    parse_args_from(std::env::args().skip(1))
}

/// Parses a time option and rejects non-finite or out-of-range values up front: a NaN window
/// would compare false everywhere and silently disable the fit.
fn parse_time_value(
    option: &str,
    value: &str,
    min: f64,
    max: f64,
    min_exclusive: bool,
) -> AppResult<f64> {
    let parsed: f64 = value
        .parse()
        .map_err(|error| format!("invalid value for {option}: {value} ({error})"))?;
    let above_min = if min_exclusive { parsed > min } else { parsed >= min };
    if !parsed.is_finite() || !above_min || parsed > max {
        let low = if min_exclusive { "(" } else { "[" };
        return Err(format!("{option} must be in {low}{min}, {max}]; got {value}").into());
    }
    Ok(parsed)
}

fn parse_args_from(mut args: impl Iterator<Item = String>) -> AppResult<Args> {
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
    let mut max_objects = 0usize;
    let mut edge_margin: Option<f64> = None;
    let mut layout = LayoutMode::Search;
    let mut seed = 20_270_831;
    let mut nozzle_clean = None;
    let mut time = TimeOptions::default();
    let mut support_overlap_mm = None;
    while let Some(option) = args.next() {
        // The valueless switches; every other option consumes the next argument.
        if option == "--ams-correction" {
            time.ams_correction = true;
            continue;
        }
        if option == "--no-centre" {
            time.no_centre = true;
            continue;
        }
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
            "--max-objects" => max_objects = value.parse()?,
            "--edge-margin" => edge_margin = Some(value.parse()?),
            "--layout" => {
                layout = match value.as_str() {
                    "search" => LayoutMode::Search,
                    "grid" => LayoutMode::Grid,
                    other => return Err(format!("unknown layout mode: {other}").into()),
                };
            }
            "--seed" => seed = value.parse()?,
            "--nozzle-clean" => {
                nozzle_clean = Some(NozzleCleanSpec::parse(&value)?);
            }
            "--time-window-hours" => {
                time.window_hours =
                    Some(parse_time_value(&option, &value, 0.0, f64::INFINITY, true)?);
            }
            "--change-seconds" => {
                time.change_seconds = Some(parse_time_value(&option, &value, 0.0, 600.0, false)?);
                time.ams_correction = true;
            }
            "--time-margin-pct" => {
                time.margin_pct = parse_time_value(&option, &value, 0.0, 50.0, false)?;
            }
            "--prep-min" => {
                time.prep_min = parse_time_value(&option, &value, 0.0, f64::INFINITY, false)?;
            }
            "--support-overlap-mm" => {
                support_overlap_mm =
                    Some(parse_time_value(&option, &value, 0.0, f64::INFINITY, false)?);
            }
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    let output = output.ok_or("--output is required")?;
    let work_dir = work_dir.unwrap_or_else(|| output.with_extension("work"));
    let attempts = attempts.max(1);
    // Compatibility option: the deterministic finite portfolio does not stop on random stalls.
    let stall = stall.unwrap_or(0);
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
        max_objects,
        seed,
        edge_margin,
        layout,
        nozzle_clean,
        time,
        support_overlap_mm,
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
            nozzle_clean_gcode: None,
            filament_change_seconds: None,
            heat_zones: vec![],
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
            max_objects: 0,
            seed: 1,
            edge_margin: None,
            layout: LayoutMode::Search,
            nozzle_clean: None,
            time: TimeOptions::default(),
            support_overlap_mm: None,
        };

        let footprint = extract_footprint(&stl).unwrap();
        let result = pack(&args, &profile, &footprint, false, &[], None, true).unwrap();
        assert!(!result.placements.is_empty());

        let footprint = extract_footprint(&stl).unwrap();
        let options = PackOptions {
            body_exclusions: Vec::new(),
            bed: profile.usable_bed().unwrap(),
            exclusions: vec![],
            resolution: args.resolution,
            clearance: args.clearance,
            angle_step_deg: args.angle_step,
            attempts: args.attempts,
            candidates_per_step: args.candidates,
            max_objects: 0,
        lock_orientation: false,
        stall_attempts: args.stall,
            seed: args.seed,
            support_overlap_mm: None,
        };
        validate_result(&footprint, &options, &result).unwrap();
    }

    #[test]
    fn validate_source_printer_rejects_slicer_kind_mismatch() {
        let profile = load_printer_profile(Path::new("profiles/p1s.json")).unwrap();
        let source = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/bambu.3mf"));
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

    fn parse(list: &[&str]) -> AppResult<Args> {
        parse_args_from(list.iter().map(|item| (*item).to_owned()))
    }

    const REQUIRED: [&str; 6] = ["--source", "s.3mf", "--profile", "p.json", "--output", "o.3mf"];

    fn parse_with(extra: &[&str]) -> AppResult<Args> {
        let mut list = REQUIRED.to_vec();
        list.extend_from_slice(extra);
        parse(&list)
    }

    #[test]
    fn time_options_default_to_no_window_and_no_correction() {
        let args = parse(&REQUIRED).unwrap();
        assert_eq!(args.time.window_hours, None);
        assert!(!args.time.ams_correction);
        assert_eq!(args.time.change_seconds, None);
        assert_eq!(args.time.margin_pct, 10.0);
        assert_eq!(args.time.prep_min, 0.0);
    }

    #[test]
    fn time_options_parse_including_valueless_ams_flag() {
        let args = parse_with(&[
            "--time-window-hours", "12", "--ams-correction", "--time-margin-pct", "15",
            "--prep-min", "5", "--clearance", "0.9",
        ])
        .unwrap();
        assert_eq!(args.time.window_hours, Some(12.0));
        assert!(args.time.ams_correction);
        assert_eq!(args.time.margin_pct, 15.0);
        assert_eq!(args.time.prep_min, 5.0);
        assert_eq!(args.clearance, 0.9);
    }

    #[test]
    fn support_overlap_defaults_to_unlimited_and_rejects_negatives() {
        assert_eq!(parse(&REQUIRED).unwrap().support_overlap_mm, None);
        assert_eq!(
            parse_with(&["--support-overlap-mm", "0"]).unwrap().support_overlap_mm,
            Some(0.0)
        );
        assert_eq!(
            parse_with(&["--support-overlap-mm", "1.5"]).unwrap().support_overlap_mm,
            Some(1.5)
        );
        for bad in ["-0.5", "NaN", "inf", "x"] {
            assert!(parse_with(&["--support-overlap-mm", bad]).is_err(), "{bad}");
        }
    }

    #[test]
    fn centre_out_only_with_a_window_and_without_no_centre() {
        assert!(!parse(&REQUIRED).unwrap().time.centre_out());
        assert!(!parse_with(&["--no-centre"]).unwrap().time.centre_out());
        assert!(parse_with(&["--time-window-hours", "12"]).unwrap().time.centre_out());
        let opted_out = parse_with(&["--no-centre", "--time-window-hours", "12"]).unwrap();
        assert!(opted_out.time.no_centre);
        assert!(!opted_out.time.centre_out());
    }

    #[test]
    fn centre_out_orders_placements_by_world_bbox_centre() {
        let square = vec![
            Point { x: -1.0, y: -1.0 },
            Point { x: 1.0, y: -1.0 },
            Point { x: 1.0, y: 1.0 },
            Point { x: -1.0, y: 1.0 },
        ];
        // Footprint off its own origin: the bbox centre, not the placement point, is ranked.
        let shifted: Vec<Point> = square.iter().map(|p| Point { x: p.x + 10.0, y: p.y }).collect();
        let footprint = Footprint {
            support_landing_loops: Vec::new(),
            body_loops: vec![shifted.clone()],
            bed_loops: None,
            loops: vec![shifted],
            width: 2.0,
            height: 2.0,
            min_z: 0.0,
            max_z: 0.0,
            triangle_count: 0,
        };
        let at = |x_mm: f64, y_mm: f64, angle_deg: f64| goat_packer::PackedPlacement {
            x_mm,
            y_mm,
            angle_deg,
        };
        let mut packed = PackResult {
            placements: vec![at(0.0, 0.0, 0.0), at(90.0, 100.0, 0.0), at(100.0, 90.0, 180.0)],
            resolution_mm: 0.25,
            requested_clearance_mm: 0.7,
            angle_step_deg: 2.0,
            attempts: 1,
            attempts_run: 1,
            seed: 1,
        };
        // Centres: (10,0), (100,100), (90,90) -> nearest to (90,90) first.
        order_centre_out(&footprint, &mut packed, (90.0, 90.0));
        let xs: Vec<f64> = packed.placements.iter().map(|p| p.x_mm).collect();
        assert_eq!(xs, vec![100.0, 90.0, 0.0]);
    }

    #[test]
    fn change_seconds_implies_ams_correction() {
        let args = parse_with(&["--change-seconds", "80"]).unwrap();
        assert!(args.time.ams_correction);
        assert_eq!(args.time.change_seconds, Some(80.0));
    }

    #[test]
    fn time_options_reject_out_of_range_values() {
        for bad in [
            &["--time-window-hours", "0"][..],
            &["--time-window-hours", "-1"],
            &["--time-window-hours", "NaN"],
            &["--change-seconds", "-1"],
            &["--change-seconds", "601"],
            &["--time-margin-pct", "-1"],
            &["--time-margin-pct", "51"],
            &["--prep-min", "-1"],
        ] {
            assert!(parse_with(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn time_correction_uses_profile_or_override_only_when_enabled() {
        let mut profile = test_profile();
        profile.filament_change_seconds = Some(95.0);
        let off = parse(&REQUIRED).unwrap().time;
        assert_eq!(off.correction(&profile).real_change_s, None);
        let on = parse_with(&["--ams-correction", "--prep-min", "2"]).unwrap().time;
        let correction = on.correction(&profile);
        assert_eq!(correction.real_change_s, Some(95.0));
        assert_eq!(correction.prep_s, 120.0);
        assert!((correction.margin - 0.1).abs() < 1e-12);
        let overridden = parse_with(&["--change-seconds", "70"]).unwrap().time;
        assert_eq!(overridden.correction(&profile).real_change_s, Some(70.0));
        // A tool changer has no profile value: correction silently stays off.
        assert_eq!(on.correction(&test_profile()).real_change_s, None);
    }

    fn sample(count: usize, corrected_s: f64) -> TimeSample {
        TimeSample { count, estimate_s: corrected_s, changes: 0, corrected_s }
    }

    #[test]
    fn fit_window_keeps_everything_when_the_max_plate_fits() {
        let fit = fit_window(sample(1, 1_000.0), sample(30, 30_000.0), 40_000.0, |_| {
            panic!("no window slice expected")
        })
        .unwrap();
        assert_eq!(fit, None);
    }

    #[test]
    fn fit_window_rejects_a_single_object_over_the_window() {
        let error = fit_window(sample(1, 50_000.0), sample(30, 90_000.0), 43_200.0, |_| {
            panic!("no window slice expected")
        })
        .unwrap_err();
        assert!(error.to_string().contains("even one object exceeds the time window"));
    }

    #[test]
    fn fit_window_reports_the_max_plate_time_when_a_single_object_max_plate_is_over() {
        // The probe fits, but the bed holds only one object and that real plate (relocated
        // tower, wipe) is over the window: the error must name the measured 13.9 h, not the
        // probe's 0.3 h.
        let error = fit_window(sample(1, 1_000.0), sample(1, 50_000.0), 43_200.0, |_| {
            panic!("no window slice expected")
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("even one object exceeds the time window"), "{error}");
        assert!(error.contains("13.9 h > window 12.0 h"), "{error}");
    }

    #[test]
    fn fit_window_accepts_the_affine_count_when_it_fits() {
        // line(N) = 1000 * N; window 15500 -> 15.
        let mut sliced = Vec::new();
        let fit = fit_window(sample(1, 1_000.0), sample(41, 41_000.0), 15_500.0, |n| {
            sliced.push(n);
            Ok(sample(n, 1_000.0 * n as f64))
        })
        .unwrap()
        .unwrap();
        assert_eq!(sliced, vec![15]);
        assert_eq!(fit.count, 15);
        assert_eq!(fit.samples.len(), 1);
    }

    #[test]
    fn fit_window_shrinks_along_the_secant_through_the_window_plate_and_the_max_plate() {
        // The probe is not comparable to window plates (no relocated tower, possibly no
        // nozzle-clean wipe): real plates lie on the line through (15, 17800) and (41, 41000),
        // i.e. 892.3 s per object, well off the probe line.
        let real = |n: usize| 41_000.0 - (41_000.0 - 17_800.0) / 26.0 * (41 - n) as f64;
        let mut sliced = Vec::new();
        let fit = fit_window(sample(1, 1_000.0), sample(41, 41_000.0), 15_500.0, |n| {
            sliced.push(n);
            Ok(sample(n, real(n)))
        })
        .unwrap()
        .unwrap();
        // 15 -> 17800 s, excess 2300 over the secant slope 892.3 -> step 3 -> 12 (15123 s)
        // fits. The probe slope (17800 - 1000) / 14 = 1200 would step 2 to 13 (16015 s) and
        // overshoot again.
        assert_eq!(sliced, vec![15, 12]);
        assert_eq!(fit.count, 12);
        assert_eq!(fit.samples.last().unwrap().count, 12);
    }

    #[test]
    fn shrink_slope_prefers_the_secant_and_falls_back_to_the_probe_line() {
        let probe = sample(1, 1_000.0);
        let max = sample(41, 41_000.0);
        assert!((shrink_slope(probe, max, sample(15, 17_800.0)) - 23_200.0 / 26.0).abs() < 1e-9);
        // No comparable pair (the sample is the max plate itself): the probe line.
        assert!((shrink_slope(probe, max, sample(41, 41_000.0)) - 1_000.0).abs() < 1e-9);
    }

    #[test]
    fn fit_window_gives_up_after_four_window_slices() {
        let mut sliced = 0;
        let error = fit_window(sample(1, 1_000.0), sample(41, 41_000.0), 15_500.0, |n| {
            sliced += 1;
            // Every measured prefix lands just over the window.
            Ok(sample(n, 15_501.0))
        })
        .unwrap_err();
        assert_eq!(sliced, MAX_WINDOW_SLICES);
        assert!(error.to_string().contains("time window"));
    }
}

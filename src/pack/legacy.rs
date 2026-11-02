//! A legacy `Footprint` as a run of the portfolio: what `pack_footprint` and `grid_layout`
//! do now. The footprint is put in its canonical frame (`frame`) and turned rigidly, so its
//! classes are angles: the search sweeps every angle family (an angle and its half-turn),
//! then a mixed palette of the best families; the brick rows run at 0° and 90°. Every
//! candidate goes through the same selection as `pack`: raster check of the shortlist,
//! vector check of the winner (the legacy validators, in the footprint's own frames).

use super::context::Ctx;
use super::print_placements;
use super::rows::brick_rows;
use super::select::{Candidate, Member, choose, failure_summary};
use super::sweep::{
    MAX_SWEEP_VARIANTS, MIXED_OFFSETS_DEG, MIXED_VARIANTS_PER_ANGLE, family_order, mixed_palette,
    sweeps,
};
use crate::footprint::Footprint;
use crate::frame::Frame;
use crate::options::{PackOptions, PackResult, PackedPlacement};
use crate::raster_mask::LegacyPrint;
use crate::validation::RasterValidator;
use crate::validation_geometry::validate_geometry;
use rayon::prelude::*;

/// Which members a legacy run uses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Portfolio {
    /// `pack_footprint`: angle families, the mixed palette and the brick rows.
    Search,
    /// `grid_layout`: the brick rows alone.
    Rows,
}

/// Every angle a legacy run may place at, numbered: the family angles and their quarter
/// turns, computed with the same expressions the members use, so ids are exact lookups.
struct AngleTable {
    angles: Vec<f64>,
}

impl AngleTable {
    fn new(options: &PackOptions) -> AngleTable {
        let mut angles: Vec<f64> = Vec::new();
        for offset in MIXED_OFFSETS_DEG {
            for family in 0..family_count(options) {
                let angle = (family_angle(family, options) + offset).rem_euclid(360.0);
                if !angles.iter().any(|a| a.to_bits() == angle.to_bits()) {
                    angles.push(angle);
                }
            }
        }
        AngleTable { angles }
    }

    fn id(&self, angle: f64) -> usize {
        self.angles
            .iter()
            .position(|a| a.to_bits() == angle.to_bits())
            .expect("every angle a member uses is in the table")
    }
}

/// Angle families: one per `angle_step_deg` over a half turn (each family also holds the
/// half-turn), or the single 0° family of a locked footprint.
fn family_count(options: &PackOptions) -> usize {
    if options.lock_orientation {
        1
    } else {
        (180.0 / options.angle_step_deg).ceil().max(1.0) as usize
    }
}

fn family_angle(family: usize, options: &PackOptions) -> f64 {
    family as f64 * options.angle_step_deg
}

/// The legacy run of `footprint` (in its input frame), as a `PackResult` in that frame.
pub(crate) fn pack_legacy(
    footprint: &Footprint,
    options: &PackOptions,
    portfolio: Portfolio,
) -> PackResult {
    let frame = Frame::new(footprint, options.lock_orientation);
    let print = LegacyPrint::new(&frame.footprint, options).expect("invalid packing input");
    let table = AngleTable::new(options);
    let (mut candidates, attempts_run) = match portfolio {
        Portfolio::Search => search_candidates(&print, &table, options),
        Portfolio::Rows => (Vec::new(), 1),
    };
    candidates.extend(rows_candidates(&frame.footprint, &print, &table, options));
    let layout = select_layout(footprint, &frame, &print, &table, options, candidates);
    let attempts = match portfolio {
        Portfolio::Search => options.attempts,
        Portfolio::Rows => 1,
    };
    PackResult {
        placements: layout.iter().map(|p| frame.to_input(p)).collect(),
        resolution_mm: options.resolution,
        requested_clearance_mm: options.clearance,
        angle_step_deg: options.angle_step_deg,
        attempts,
        attempts_run,
        seed: options.seed,
    }
}

/// A run over the classes at `angles`, numbered by `table`.
fn legacy_ctx<'a>(print: &'a LegacyPrint, table: &AngleTable, angles: &[f64]) -> Ctx<'a> {
    let masks = angles.par_iter().map(|&a| print.masks_at(a)).collect();
    let ids = angles.iter().map(|&a| table.id(a)).collect();
    Ctx::new(masks, ids, &print.zones, print.grid)
}

/// Families (in parallel, collected in angle order) and, with effort left, the mixed palette
/// of the best families. Returns the candidates and the number of sweeps run.
fn search_candidates(
    print: &LegacyPrint,
    table: &AngleTable,
    options: &PackOptions,
) -> (Vec<Candidate>, usize) {
    let count = family_count(options);
    let variants = options
        .attempts
        .max(1)
        .div_ceil(count)
        .clamp(1, MAX_SWEEP_VARIANTS);
    let families: Vec<(Candidate, (usize, u64))> = (0..count)
        .into_par_iter()
        .map(|family| {
            let angle = family_angle(family, options);
            let mut angles = vec![angle];
            if !options.lock_orientation {
                angles.push((angle + 180.0).rem_euclid(360.0));
            }
            let ctx = legacy_ctx(print, table, &angles);
            let run = sweeps(&ctx, None, variants, options.max_objects);
            let detail = vec![family as i64, run.variant as i64];
            let candidate = ctx.candidate(Member::Sweep, None, detail, &run.layout);
            (candidate, run.baseline)
        })
        .collect();
    let (mut candidates, baselines): (Vec<Candidate>, Vec<(usize, u64)>) =
        families.into_iter().unzip();
    let mut attempts_run = count * variants;
    if !options.lock_orientation && options.attempts > count {
        let palette = mixed_palette(&family_order(&baselines), options.angle_step_deg);
        let ctx = legacy_ctx(print, table, &palette);
        let mixed_variants =
            (options.attempts - count).min(MIXED_VARIANTS_PER_ANGLE * palette.len());
        let run = sweeps(&ctx, None, mixed_variants, options.max_objects);
        attempts_run += mixed_variants;
        let detail = vec![run.variant as i64];
        candidates.push(ctx.candidate(Member::Mixed, None, detail, &run.layout));
    }
    (candidates, attempts_run)
}

/// Brick rows at 0° and, for an oblong footprint that may turn, at 90°.
fn rows_candidates(
    canonical: &Footprint,
    print: &LegacyPrint,
    table: &AngleTable,
    options: &PackOptions,
) -> Vec<Candidate> {
    let square = (canonical.width - canonical.height).abs() < 1e-9;
    let angles: &[f64] = if options.lock_orientation || square {
        &[0.0]
    } else {
        &[0.0, 90.0]
    };
    let ctx = legacy_ctx(print, table, angles);
    (0..ctx.classes())
        .into_par_iter()
        .map(|class| {
            let (layout, shift) = brick_rows(&ctx, class, None, options.max_objects);
            let detail = vec![class as i64, i64::from(shift)];
            ctx.candidate(Member::Rows, None, detail, &layout)
        })
        .collect()
}

/// The selected layout in the canonical frame. When no shortlisted layout passes both
/// checks, the best one is returned as is and the caller's `validate_result` rejects it, as
/// before validation moved into the search.
fn select_layout(
    footprint: &Footprint,
    frame: &Frame,
    print: &LegacyPrint,
    table: &AngleTable,
    options: &PackOptions,
    candidates: Vec<Candidate>,
) -> Vec<PackedPlacement> {
    let canonical = |c: &Candidate| -> Vec<PackedPlacement> {
        let placed = c.placements.iter().map(|p| {
            let (x_mm, y_mm) = p.mm(&print.grid);
            PackedPlacement {
                x_mm,
                y_mm,
                angle_deg: table.angles[p.class],
            }
        });
        placed.collect()
    };
    let mut raster =
        RasterValidator::legacy(print, table.angles.clone(), options.support_overlap_mm);
    // The validator's classes are the table's angles, so a candidate's class ids carry over.
    let raster_check = |shortlist: &[Candidate]| {
        let mut check = |c: &Candidate| raster.check(&print_placements(c, &print.grid));
        shortlist.iter().map(&mut check).collect()
    };
    let vector_check = |c: &Candidate| {
        let result = PackResult {
            placements: canonical(c).iter().map(|p| frame.to_input(p)).collect(),
            resolution_mm: options.resolution,
            requested_clearance_mm: options.clearance,
            angle_step_deg: options.angle_step_deg,
            attempts: options.attempts,
            attempts_run: 0,
            seed: options.seed,
        };
        validate_geometry(footprint, options, &result)
    };
    match choose(candidates, raster_check, vector_check) {
        Ok((winner, _)) => canonical(&winner),
        Err((shortlist, report)) => {
            eprintln!("pack_footprint: {}", failure_summary(&report));
            shortlist.first().map(canonical).unwrap_or_default()
        }
    }
}

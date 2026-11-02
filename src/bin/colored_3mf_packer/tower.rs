//! The prime (wipe) tower: where the probe saw it, where it may move, and whether the final
//! slice kept to what was reserved for it.

use std::path::Path;

use goat_packer::{
    Footprint, PackResult, Point, Rect, extract_footprint, loops_touch_rect,
    colored_packer::{
        PrimeTowerBounds, PrinterProfile, parse_prime_tower_bounds, tower_repack_required,
    },
    plate_centre::{
        RelocatedTower, ensure_relocated_tower_clear, free_tower_spot, gui_tower_anchor_range,
        union_rect, wipe_tower_anchor,
    },
};

use crate::AppResult;
use crate::args::Args;
use crate::placement::placement_world_loops;
use crate::job::Job;
use crate::plan::PackPlan;
use crate::source::source_project_settings;

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
pub const CORNER_TOWER_INSET_MM: f64 = 6.0;

/// Slack when testing a corner keepout against the usable bed, mm.
const BED_FIT_SLACK_MM: f64 = 1e-9;

/// What to do after comparing the probe-slice tower against the final-slice tower. A project with
/// no prime/wipe tower (e.g. a single-color plate) has `probe` and `final` both `None` throughout
/// and never needs a repack.
#[derive(Debug, PartialEq)]
pub enum TowerOutcome {
    Stable,
    RepackRequired,
}

/// Decides how to react to the final slice's tower given what the probe slice found. A `None`
/// probe means the source project has no prime/wipe tower (single material/color): packing never
/// reserved space for one, so a tower appearing in the final slice would silently invalidate every
/// collision check and must be rejected rather than repacked around.
pub fn evaluate_final_tower(
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

/// The tower of several probe slices. It depends on the supports' flush volume too, so the
/// largest run is reserved: the union of every run's tower.
pub fn probe_tower_of(
    gcodes: &[&[u8]],
    profile: &PrinterProfile,
    clearance: f64,
) -> Option<PrimeTowerBounds> {
    gcodes
        .iter()
        .map(|gcode| parse_prime_tower_bounds(gcode, profile.tower_line_width_mm, clearance))
        .reduce(|a, b| match (a, b) {
            (Some(a), Some(b)) => Some(PrimeTowerBounds {
                centerline: union_rect(a.centerline, b.centerline),
                keepout: union_rect(a.keepout, b.keepout),
                ..a
            }),
            (a, b) => a.or(b),
        })
        .flatten()
}

/// Where the slicer GUI keeps the stored tower position on open (`gui_tower_anchor_range`),
/// plus the source's stored position to translate it into keepout coordinates.
#[derive(Clone, Copy, Debug)]
pub struct GuiTowerRange {
    anchor_range: Rect,
    source_anchor: (f64, f64),
}

impl GuiTowerRange {
    /// The same range for the min corner of a tower keepout measured with the source position:
    /// the stored position and the keepout's min corner move together (`RelocatedTower::shift`).
    pub fn keepout_min_range(self, probe_keepout: Rect) -> Rect {
        let offset_x = self.source_anchor.0 - probe_keepout.min_x;
        let offset_y = self.source_anchor.1 - probe_keepout.min_y;
        Rect::new(
            self.anchor_range.min_x - offset_x,
            self.anchor_range.min_y - offset_y,
            self.anchor_range.max_x - offset_x,
            self.anchor_range.max_y - offset_y,
        )
    }
}

/// `None` when the project shows no tower or the GUI range is empty.
pub fn gui_tower_range(
    source: &Path,
    profile: &PrinterProfile,
    footprint_stl: &Path,
) -> AppResult<Option<GuiTowerRange>> {
    let settings = source_project_settings(source)?;
    let mesh = extract_footprint(footprint_stl)?;
    let Some(anchor_range) = gui_tower_anchor_range(
        &settings,
        &profile.slicer_kind,
        profile.printable_bed(),
        mesh.max_z - mesh.min_z,
    ) else {
        return Ok(None);
    };
    let Ok(source_anchor) = wipe_tower_anchor(&settings) else {
        return Ok(None);
    };
    Ok(Some(GuiTowerRange { anchor_range, source_anchor }))
}

/// Moves a relocated tower (keepout and setting shift together) so its keepout's min corner
/// lies in `range`, where the slicer GUI leaves it on open.
pub fn keep_in_gui_range(relocated: RelocatedTower, range: Option<Rect>) -> RelocatedTower {
    let Some(range) = range else {
        return relocated;
    };
    let keepout = relocated.keepout;
    let dx = keepout.min_x.clamp(range.min_x, range.max_x) - keepout.min_x;
    let dy = keepout.min_y.clamp(range.min_y, range.max_y) - keepout.min_y;
    RelocatedTower {
        keepout: keepout.translated(dx, dy),
        shift: (relocated.shift.0 + dx, relocated.shift.1 + dy),
    }
}

/// The source position and the four usable-bed corners, in the order of preference on a tie.
/// A corner keepout reserves the growth room above the tower (`CORNER_TOWER_GROWTH`) and keeps
/// `CORNER_TOWER_INSET_MM` from the bed edge; `None` = the source position unchanged.
fn tower_candidates(
    probe_keepout: Rect,
    bed: Rect,
    gui_range: Option<Rect>,
) -> [(&'static str, Option<RelocatedTower>); 5] {
    let width = probe_keepout.max_x - probe_keepout.min_x;
    let probe_depth = probe_keepout.max_y - probe_keepout.min_y;
    let depth = probe_depth * (1.0 + CORNER_TOWER_GROWTH);
    let corner = |name: &'static str, min_x: f64, min_y: f64| {
        let keepout = Rect::new(min_x, min_y, min_x + width, min_y + depth);
        let shift = (min_x - probe_keepout.min_x, min_y - probe_keepout.min_y);
        (name, Some(RelocatedTower { keepout, shift }))
    };
    let inset = CORNER_TOWER_INSET_MM;
    // The slicer GUI would pull a corner tower back into its range on open (see
    // `gui_tower_anchor_range`), so the corners are where it keeps them.
    let clamp_x = |x: f64| gui_range.map_or(x, |range| x.clamp(range.min_x, range.max_x));
    let clamp_y = |y: f64| gui_range.map_or(y, |range| y.clamp(range.min_y, range.max_y));
    let (left, right) = (clamp_x(bed.min_x + inset), clamp_x(bed.max_x - inset - width));
    let (bottom, top) = (clamp_y(bed.min_y + inset), clamp_y(bed.max_y - inset - depth));
    // A source tower outside the range moves on open as well; it competes where it will print.
    let source = keep_in_gui_range(
        RelocatedTower { keepout: probe_keepout, shift: (0.0, 0.0) },
        gui_range,
    );
    let source = (source.shift != (0.0, 0.0)).then(|| {
        let keepout = Rect::new(
            source.keepout.min_x,
            source.keepout.min_y,
            source.keepout.min_x + width,
            source.keepout.min_y + probe_depth,
        );
        RelocatedTower { keepout, shift: source.shift }
    });
    [
        ("source position", source),
        corner("bottom-left corner", left, bottom),
        corner("bottom-right corner", right, bottom),
        corner("top-left corner", left, top),
        corner("top-right corner", right, top),
    ]
}

/// Spec 2026-10-31 rule 3: the source tower position competes with the four usable-bed
/// corners; the densest plate wins, ties keep the source position (then the corner order).
/// A corner on a fixed keepout (P1S exclusion corner, purge line) or off the bed is skipped.
/// Returns the chosen relocation (`None` = source position) with its packed layout.
pub fn choose_tower_position(
    job: &Job,
    plan: &PackPlan,
    probe_keepout: Rect,
    gui_range: Option<Rect>,
) -> AppResult<(Option<RelocatedTower>, PackResult)> {
    let bed = job.profile.usable_bed()?;
    let mut best: Option<(&str, Option<RelocatedTower>, PackResult)> = None;
    for (name, relocation) in tower_candidates(probe_keepout, bed, gui_range) {
        let keepout = relocation.map_or(probe_keepout, |relocated| relocated.keepout);
        if relocation.is_some()
            && (keepout.sticks_out_of(bed, BED_FIT_SLACK_MM)
                || ensure_relocated_tower_clear(keepout, &plan.fixed_keepouts).is_err())
        {
            continue;
        }
        let packed = plan.pack_at(job, Some(keepout), false)?;
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

/// When a placement's full patch (supports included) touches the reserved tower keepout,
/// the nearest spot of the usable bed where a tower of the same width, with the corner
/// contest's growth room above it, keeps the clearance from every patch and fixed keepout
/// (`None`: nothing touches, or no such spot). Corner contest rules apply: the spot keeps
/// `CORNER_TOWER_INSET_MM` from the bed edge, and the shift is relative to the probe keepout,
/// whose corner the source `wipe_tower_x/y` anchors.
pub fn clear_tower_of_patches(
    args: &Args,
    profile: &PrinterProfile,
    plan: &PackPlan,
    packed: &PackResult,
    probe_keepout: Rect,
    gui_range: Option<Rect>,
) -> AppResult<Option<RelocatedTower>> {
    let Some(reserved) = plan.tower_keepout else {
        return Ok(None);
    };
    // The patch and its bed variant (patch ∪ overhanging silhouette) stay separate objects:
    // both cover the same area, and one even-odd loop set would read the overlap as a hole.
    let mut overhang = plan.footprint.clone();
    overhang.loops = plan.footprint.bed_loops.clone().unwrap_or_default();
    let touching = placements_touching(
        [&plan.footprint, &overhang],
        packed,
        reserved.grown(args.clearance),
    );
    if touching == 0 {
        return Ok(None);
    }
    let width = reserved.max_x - reserved.min_x;
    let depth = (reserved.max_y - reserved.min_y)
        .max((probe_keepout.max_y - probe_keepout.min_y) * (1.0 + CORNER_TOWER_GROWTH));
    let mut area = profile.usable_bed()?.inset(CORNER_TOWER_INSET_MM)?;
    // Only spots the slicer GUI keeps on open (see `gui_tower_anchor_range`).
    if let Some(range) = gui_range {
        area = Rect::new(
            area.min_x.max(range.min_x),
            area.min_y.max(range.min_y),
            area.max_x.min(range.max_x + width),
            area.max_y.min(range.max_y + depth),
        );
        if area.min_x >= area.max_x || area.min_y >= area.max_y {
            println!("prime tower touches the supports of {touching} placement(s); no free spot the slicer GUI keeps");
            return Ok(None);
        }
    }
    let objects: Vec<Vec<Vec<Point>>> = packed
        .placements
        .iter()
        .flat_map(|placement| {
            [
                placement_world_loops(&plan.footprint, placement),
                placement_world_loops(&overhang, placement),
            ]
        })
        .filter(|loops| !loops.is_empty())
        .collect();
    let Some(spot) = free_tower_spot(
        reserved,
        (width, depth),
        area,
        &objects,
        &plan.fixed_keepouts,
        args.clearance,
    ) else {
        println!(
            "prime tower touches the supports of {touching} placement(s); no free spot to move it"
        );
        return Ok(None);
    };
    let shift = (spot.min_x - probe_keepout.min_x, spot.min_y - probe_keepout.min_y);
    println!(
        "prime tower touches the supports of {touching} placement(s); moving it to a free spot \
         by ({:+.1}, {:+.1}) mm from the source: keepout {:.2},{:.2} → {:.2},{:.2}",
        shift.0, shift.1, spot.min_x, spot.min_y, spot.max_x, spot.max_y
    );
    Ok(Some(RelocatedTower { keepout: spot, shift }))
}

/// How many placements have any of `zones` touching `area`.
fn placements_touching(zones: [&Footprint; 2], packed: &PackResult, area: Rect) -> usize {
    packed
        .placements
        .iter()
        .filter(|placement| {
            zones
                .iter()
                .any(|zone| loops_touch_rect(&placement_world_loops(zone, placement), area))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

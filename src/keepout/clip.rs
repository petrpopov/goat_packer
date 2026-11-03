//! Supports across the bed edge (`PlateRules::support_clip_mm`): the one condition that makes
//! it safe.
//!
//! The slicer builds ONE support per rotation class and prints it under every copy, but it
//! clips that support by the bed in the frame of the FIRST copy of the class (BambuStudio
//! `TreeSupport.cpp`: `m_machine_border` is translated by `instances().front().shift`, then
//! roof, roof-first-layer and base areas are intersected with it). So the support a copy
//! prints is its envelope cut at the line where the first copy meets each edge. A copy whose
//! envelope crosses an edge by `d` mm is on the plate after the cut only when the first copy
//! crosses the same edge by at least `d`; otherwise it keeps support material the cut did not
//! remove, hanging past the edge. [`dominating_copy`] finds the first copy that satisfies
//! this for every edge at once.
//!
//! Clipping only removes material, so the foreign-gap checks made with the unclipped envelope
//! stay conservative for the clipped support (an envelope that is a superset of what prints).
//! What the clip does NOT cover: BambuStudio also treats everything outside the bed as an
//! obstacle for the branches of the first copy (`support_model::clearance::bed`), which bends
//! trees inward rather than just cutting them, so a first copy standing at the edge prints a
//! realization that is not a subset of the interior one. The caps in
//! `GuiFlavour::support_clip_cap_mm` (`colored_packer::bed_rules`) come from a plate that printed clean with such a first
//! copy; a verification slice remains the only proof.

use crate::geometry::Rect;
use crate::object_print::ObjectPrint;
use crate::plate_centre::loops_bounds;

/// Numerical slack when comparing overhangs, mm.
const SAME_MM: f64 = 1e-6;

/// How far the support of `print` standing at `at` reaches past each edge of `printable`:
/// `[min_x, min_y, max_x, max_y]` sides, 0 when it stays inside. Zero for a print without
/// supports.
pub fn support_overhang(print: &ObjectPrint, at: (f64, f64), printable: Rect) -> [f64; 4] {
    let loops: Vec<_> = std::iter::once(&print.support_base)
        .chain(&print.support)
        .chain(&print.interface)
        .flatten()
        .cloned()
        .collect();
    let Some(bounds) = loops_bounds(&loops) else {
        return [0.0; 4];
    };
    let placed = bounds.translated(at.0, at.1);
    [
        printable.min_x - placed.min_x,
        printable.min_y - placed.min_y,
        placed.max_x - printable.max_x,
        placed.max_y - printable.max_y,
    ]
    .map(|reach| reach.max(0.0))
}

/// The copy whose overhang is at least every other copy's on all four sides, the earliest of
/// equals; `None` when no copy is (two copies cross different edges). With no overhang at all
/// every copy qualifies and the first is returned.
pub fn dominating_copy(overhangs: &[[f64; 4]]) -> Option<usize> {
    (0..overhangs.len()).find(|&candidate| {
        overhangs.iter().all(|other| {
            (0..4).all(|side| overhangs[candidate][side] >= other[side] - SAME_MM)
        })
    })
}

/// The condition on a finished layout in 3MF order: when supports may cross the edge
/// (`clip_mm > 0`) and any copy's does, the first copy of every class must cross each edge at
/// least as far as every other copy of the class. `placements` are `(class, x, y)` in 3MF order.
pub fn check_first_copies(
    prints: &[ObjectPrint],
    placements: &[(usize, f64, f64)],
    printable: Rect,
    clip_mm: f64,
) -> Result<(), String> {
    if clip_mm <= 0.0 {
        return Ok(());
    }
    for (class, print) in prints.iter().enumerate() {
        let overhangs: Vec<[f64; 4]> = placements
            .iter()
            .filter(|(of, ..)| *of == class)
            .map(|&(_, x, y)| support_overhang(print, (x, y), printable))
            .collect();
        if !crosses_any(&overhangs) {
            continue;
        }
        if let Some(over) = overhangs.iter().flatten().find(|&&reach| reach > clip_mm + SAME_MM) {
            return Err(format!("class {class}: support crosses the bed edge by {over:.3} mm, more than {clip_mm} mm"));
        }
        if dominating_copy(&overhangs) != Some(0) {
            return Err(format!(
                "class {class}: the first copy must cross every bed edge as far as any other copy, \
                 because the slicer clips the shared support at the first copy ({overhangs:?})"
            ));
        }
    }
    Ok(())
}

/// Whether some order of `placements` satisfies [`check_first_copies`]: every class either
/// keeps its supports inside or has a dominating copy.
pub fn first_copies_exist(
    prints: &[ObjectPrint],
    placements: &[(usize, f64, f64)],
    printable: Rect,
    clip_mm: f64,
) -> bool {
    if clip_mm <= 0.0 {
        return true;
    }
    prints.iter().enumerate().all(|(class, print)| {
        let overhangs: Vec<[f64; 4]> = placements
            .iter()
            .filter(|(of, ..)| *of == class)
            .map(|&(_, x, y)| support_overhang(print, (x, y), printable))
            .collect();
        !crosses_any(&overhangs) || dominating_copy(&overhangs).is_some()
    })
}

/// Whether any side of any copy crosses its edge.
pub fn crosses_any(overhangs: &[[f64; 4]]) -> bool {
    overhangs.iter().flatten().any(|&reach| reach > SAME_MM)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point;

    fn print_with_support(width: f64, height: f64) -> ObjectPrint {
        let p = |x: f64, y: f64| Point { x, y };
        ObjectPrint {
            support_base: vec![vec![
                p(-width, -height),
                p(width, -height),
                p(width, height),
                p(-width, height),
                p(-width, -height),
            ]],
            ..ObjectPrint::default()
        }
    }

    #[test]
    fn overhang_is_the_support_reach_past_each_edge() {
        let bed = Rect::new(0.0, 0.0, 100.0, 100.0);
        let print = print_with_support(10.0, 10.0);
        assert_eq!(support_overhang(&print, (50.0, 50.0), bed), [0.0; 4]);
        let reach = support_overhang(&print, (95.0, 4.0), bed);
        assert_eq!(reach, [0.0, 6.0, 5.0, 0.0]);
        assert_eq!(support_overhang(&ObjectPrint::default(), (95.0, 4.0), bed), [0.0; 4]);
    }

    #[test]
    fn a_layout_in_3mf_order_needs_its_first_copy_to_cross_the_most_and_within_the_cap() {
        let bed = Rect::new(0.0, 0.0, 100.0, 100.0);
        let prints = [print_with_support(10.0, 10.0)];
        let inner = (0, 50.0, 50.0);
        let top3 = (0, 50.0, 93.0);
        let top1 = (0, 30.0, 91.0);
        assert!(check_first_copies(&prints, &[top3, top1, inner], bed, 4.0).is_ok());
        let error = check_first_copies(&prints, &[inner, top3], bed, 4.0).unwrap_err();
        assert!(error.contains("first copy"), "{error}");
        let error = check_first_copies(&prints, &[top3], bed, 2.0).unwrap_err();
        assert!(error.contains("more than"), "{error}");
        // No clip: nothing is checked here, the zone rules keep supports inside.
        assert!(check_first_copies(&prints, &[inner, top3], bed, 0.0).is_ok());
        assert!(first_copies_exist(&prints, &[inner, top3], bed, 4.0));
        let right = (0, 93.0, 50.0);
        assert!(!first_copies_exist(&prints, &[top3, right], bed, 4.0));
    }

    #[test]
    fn the_first_copy_must_cross_every_edge_the_others_cross_by_as_much() {
        let top = [0.0, 0.0, 0.0, 3.0];
        let less_top = [0.0, 0.0, 0.0, 1.0];
        let right = [0.0, 0.0, 2.0, 0.0];
        let inside = [0.0; 4];
        assert_eq!(dominating_copy(&[inside, less_top, top]), Some(2));
        assert_eq!(dominating_copy(&[top, less_top, inside]), Some(0));
        assert_eq!(dominating_copy(&[top, right]), None);
        assert_eq!(dominating_copy(&[top, right, [0.0, 0.0, 2.0, 3.0]]), Some(2));
        assert_eq!(dominating_copy(&[inside, inside]), Some(0));
        assert!(crosses_any(&[inside, top]) && !crosses_any(&[inside, inside]));
    }
}

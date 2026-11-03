//! What a sliced plate teaches the plan: the supports every copy really printed, in its class's
//! frame, join the class's print (grown by the same margins as a probe), and a tower wider
//! than reserved widens the tower. The plate is then packed again, once.

use std::collections::HashMap;

use goat_packer::{
    PrintPlacement, SlicedObject,
    gcode_collisions::{ObjectFirstLayer, PlateAnalysis},
    object_print::{Loops, ObjectPrint},
    plate_centre::loops_bounds,
    realizations_from_gcode,
};

use crate::AppResult;
use crate::loops_ops::{grown, union};
use crate::margins::{DELTA_BAND_MM, DELTA_FIRST_LAYER_MM};
use crate::plan::PackPlan;

/// Farthest a sliced object's first-layer body centre may sit from where its placement puts
/// the print's landing and still be matched to it. Neighbouring goats are ~40 mm apart.
const MATCH_TOLERANCE_MM: f64 = 5.0;

/// Sliced object label → index of its placement, by the first-layer body centre. Objects
/// without a first-layer body or without a placement that close are left out.
pub fn match_placements(
    prints: &[ObjectPrint],
    placements: &[PrintPlacement],
    first_layer: &[ObjectFirstLayer],
) -> HashMap<String, usize> {
    let landing_centre = |print: &ObjectPrint| {
        let bounds = loops_bounds(&print.landing).or_else(|| loops_bounds(&print.silhouette))?;
        Some(((bounds.min_x + bounds.max_x) * 0.5, (bounds.min_y + bounds.max_y) * 0.5))
    };
    let expected: Vec<Option<(f64, f64)>> = placements
        .iter()
        .map(|placement| {
            let (cx, cy) = landing_centre(&prints[placement.print])?;
            Some((placement.x_mm + cx, placement.y_mm + cy))
        })
        .collect();
    let mut matched = HashMap::new();
    for object in first_layer {
        let Some(centre) = object.body_center() else {
            continue;
        };
        let nearest = expected
            .iter()
            .enumerate()
            .filter_map(|(index, at)| {
                let at = (*at)?;
                Some((index, (centre.0 - at.0).hypot(centre.1 - at.1)))
            })
            .filter(|(_, distance)| *distance <= MATCH_TOLERANCE_MM)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((index, _)) = nearest {
            matched.insert(object.label.clone(), index);
        }
    }
    matched
}

/// Unions what every matched object printed into its class's print.
pub fn learn_supports(
    plan: &mut PackPlan,
    gcode: &[u8],
    placements: &[PrintPlacement],
    analysis: &PlateAnalysis,
) -> AppResult<()> {
    let matched = match_placements(&plan.prints, placements, &analysis.first_layer);
    if matched.is_empty() {
        return Err("no sliced object matches a placement; cannot learn the realized supports".into());
    }
    let objects: Vec<(usize, SlicedObject)> = matched
        .iter()
        .map(|(label, &index)| {
            let placement = placements[index];
            let object = SlicedObject {
                label: Some(label.clone()),
                origin: (placement.x_mm, placement.y_mm),
            };
            (placement.print, object)
        })
        .collect();
    let sliced: Vec<SlicedObject> = objects.iter().map(|(_, object)| object.clone()).collect();
    let realizations = realizations_from_gcode(gcode, &sliced, 0)?;
    for ((class, _), realization) in objects.iter().zip(&realizations) {
        let print = &mut plan.prints[*class];
        let grown_base = grown(&realization.base, DELTA_FIRST_LAYER_MM);
        print.support_base = union(&[&print.support_base, &grown_base]);
        merge_bands(&mut print.support, &realization.above_base);
        merge_bands(&mut print.interface, &realization.interface);
    }
    Ok(())
}

/// Each band of `learned`, grown by δ_band, united into `bands`; missing bands are added.
fn merge_bands(bands: &mut Vec<Loops>, learned: &[Loops]) {
    if bands.len() < learned.len() {
        bands.resize(learned.len(), Loops::new());
    }
    for (band, learned) in bands.iter_mut().zip(learned) {
        if !learned.is_empty() {
            *band = union(&[band, &grown(learned, DELTA_BAND_MM)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{test_job, test_plan};
    use goat_packer::gcode_collisions::analyze_plate;

    /// Two labelled objects on layer 1, each a wall 10 mm long centred on its placement and a
    /// support line that reaches `reach` mm past the wall's end.
    fn plate(reach: [f64; 2]) -> String {
        let object = |label: u32, x: f64, reach: f64| {
            format!(
                "; start printing object, unique label id: {label}\n; FEATURE: Outer wall\n\
                 G1 X{} Y20\nG1 X{} Y20 E1\n; FEATURE: Support\nG1 X{} Y20\nG1 X{} Y20 E1\n\
                 ; stop printing object, unique label id: {label}\n",
                x - 5.0,
                x + 5.0,
                x + 5.5,
                x + 5.0 + reach
            )
        };
        format!(
            "; CHANGE_LAYER\n; Z_HEIGHT: 0.2\n; LINE_WIDTH: 0.4\n{}{}",
            object(1, 20.0, reach[0]),
            object(2, 50.0, reach[1])
        )
    }

    fn at(x_mm: f64) -> PrintPlacement {
        PrintPlacement { print: 0, x_mm, y_mm: 20.0 }
    }

    #[test]
    fn sliced_objects_are_matched_to_the_placement_nearest_their_first_layer_body() {
        let dir = tempfile::tempdir().unwrap();
        let plan = test_plan(&test_job(dir.path()), 1);
        let analysis = analyze_plate(plate([3.0, 3.0]).as_bytes());
        // The placements are listed in the other order: labels follow the geometry.
        let matched = match_placements(&plan.prints, &[at(50.0), at(20.0)], &analysis.first_layer);
        assert_eq!((matched["1"], matched["2"]), (1, 0));
        // An object farther than the tolerance from every placement is left out.
        let far = match_placements(&plan.prints, &[at(30.0)], &analysis.first_layer);
        assert!(far.is_empty());
    }

    #[test]
    fn what_every_copy_printed_joins_its_class_grown_by_the_margins() {
        let dir = tempfile::tempdir().unwrap();
        let mut plan = test_plan(&test_job(dir.path()), 1);
        let gcode = plate([3.0, 6.0]);
        let analysis = analyze_plate(gcode.as_bytes());
        learn_supports(&mut plan, gcode.as_bytes(), &[at(20.0), at(50.0)], &analysis).unwrap();
        // The second copy's support ends 6 mm past its wall end (frame x = 5 + 6 = 11).
        let reach = plan.prints[0].support_base.iter().flatten().map(|p| p.x).fold(f64::MIN, f64::max);
        assert!(reach >= 11.0 + DELTA_FIRST_LAYER_MM && reach < 11.0 + DELTA_FIRST_LAYER_MM + 0.5, "{reach}");
        assert!(learn_supports(&mut plan, gcode.as_bytes(), &[at(300.0)], &analysis).is_err());
    }
}

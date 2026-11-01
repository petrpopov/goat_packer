//! Learning from a sliced sample: height-band zones, the realized support patch, and which
//! sliced object is which placement.

use std::collections::{BTreeMap, HashMap};

use geo::Polygon;

use super::parser::{extents_of, parse};
use super::types::{ObjectFirstLayer, PlateAnalysis};
use crate::footprint::{Footprint, LayerZone};
use crate::geometry::{polygon_from_loop, strip_polygon, union_exterior_loops};
use crate::options::PackedPlacement;

/// A bed point in the footprint frame of `placement`: the inverse of placing the footprint.
fn to_footprint_frame(placement: &PackedPlacement, (x, y): (f64, f64)) -> (f64, f64) {
    let (sin, cos) = (-placement.angle_deg).to_radians().sin_cos();
    let (dx, dy) = (x - placement.x_mm, y - placement.y_mm);
    (dx * cos - dy * sin, dx * sin + dy * cos)
}

/// Height of one `Footprint::layer_zones` band. A band unions what its layers print, so a
/// support and a body in the same band but on different layers count as meeting: coarser
/// bands are safer and cost a little density; 2 mm keeps a 45 mm goat at ~23 bands.
pub const LAYER_BAND_MM: f64 = 2.0;

/// `Footprint::layer_zones` from a sliced sample of the plate (a few of its objects at their
/// placements): every matched object's lines of each `LAYER_BAND_MM` band, moved into the
/// footprint frame — supports from all of them (the sample objects print the same supports to
/// ~0.1 mm; the union absorbs the rest), the body from the first one (bodies are identical;
/// infill is left out, the walls around it bound the band's body ring). Also returns the
/// sample's analysis for learning the first-layer patch.
pub fn layer_zones_from_sample(
    gcodes: &[&[u8]],
    footprint: &Footprint,
    placements: &[PackedPlacement],
) -> Result<(Vec<LayerZone>, PlateAnalysis), String> {
    // Several slices of the same sample: tree supports differ between runs (see
    // `crate::first_layer_footprint_of`), so every run's supports join the bands and the
    // analysis carries every run's first layer for `learn_support_patch`.
    let mut runs = Vec::with_capacity(gcodes.len());
    for gcode in gcodes {
        let parser = parse(gcode, true);
        let matched = match_placements(footprint, placements, &parser.first_layer)?;
        if matched.is_empty() {
            return Err("no sample object matches a placement".into());
        }
        let index_of: HashMap<usize, usize> = parser
            .labels
            .iter()
            .enumerate()
            .filter_map(|(object, label)| Some((object, *matched.get(label)?)))
            .collect();
        runs.push((parser, index_of));
    }
    let analysis = PlateAnalysis {
        collisions: runs
            .iter()
            .flat_map(|(parser, _)| parser.collisions.values().cloned())
            .collect(),
        first_layer: runs
            .iter()
            .flat_map(|(parser, _)| parser.first_layer.iter().cloned())
            .collect(),
        layers: runs
            .iter()
            .map(|(parser, _)| parser.layer)
            .max()
            .unwrap_or(0),
        labelled_lines: runs.iter().map(|(parser, _)| parser.labelled_lines).sum(),
        extents: runs
            .iter()
            .flat_map(|(parser, _)| extents_of(parser))
            .collect(),
    };
    let mut bands: BTreeMap<usize, (Vec<Polygon>, Vec<Polygon>)> = BTreeMap::new();
    for (run, (parser, index_of)) in runs.iter().enumerate() {
        // The body comes from one object of the first run: bodies don't vary.
        let body_object = *index_of.keys().min().expect("matched is not empty");
        for line in &parser.kept {
            let Some(&index) = index_of.get(&line.object) else {
                continue;
            };
            if !line.strip.support && (run != 0 || line.object != body_object || line.infill) {
                continue;
            }
            let placement = &placements[index];
            let (from, to) = (
                to_footprint_frame(placement, line.strip.from),
                to_footprint_frame(placement, line.strip.to),
            );
            let polygon = strip_polygon(from.0, from.1, to.0, to.1, line.strip.width);
            let band = bands
                .entry((line.z.max(0.0) / LAYER_BAND_MM) as usize)
                .or_default();
            if line.strip.support {
                band.0.push(polygon);
            } else {
                band.1.push(polygon);
            }
        }
    }
    let zones = bands
        .into_values()
        .filter(|(support, body)| !support.is_empty() && !body.is_empty())
        .map(|(support, body)| LayerZone {
            support_loops: union_exterior_loops(&support),
            body_loops: union_exterior_loops(&body),
        })
        .collect();
    Ok((zones, analysis))
}

/// Farthest an object's first-layer body centre may sit from its placement's expected body
/// centre and still be matched to it. Neighbouring goats are ~40 mm apart.
const MATCH_TOLERANCE_MM: f64 = 5.0;

/// Widens `footprint`'s reserved patch to what a sliced plate really printed: every object's
/// first-layer lines, matched to its placement by the first-layer body centre and moved into
/// the footprint frame, are unioned into `loops` and `bed_loops`. Body, silhouette and landing
/// zones stay as they are — only the support reach grows. Errors when no object can be
/// matched, i.e. the G-code does not describe this layout.
pub fn learn_support_patch(
    footprint: &Footprint,
    placements: &[PackedPlacement],
    first_layer: &[ObjectFirstLayer],
) -> Result<Footprint, String> {
    let mut polygons: Vec<Polygon> = footprint
        .loops
        .iter()
        .chain(footprint.bed_loops.iter().flatten())
        .map(|loop_| polygon_from_loop(loop_))
        .collect();
    let matched = match_placements(footprint, placements, first_layer)?;
    if matched.is_empty() {
        return Err("no sliced object matches a placement; cannot learn the support patch".into());
    }
    for object in first_layer {
        let Some(&index) = matched.get(&object.label) else {
            continue;
        };
        let placement = &placements[index];
        for strip in object.strips.iter().filter(|strip| strip.support) {
            let from = to_footprint_frame(placement, strip.from);
            let to = to_footprint_frame(placement, strip.to);
            polygons.push(strip_polygon(from.0, from.1, to.0, to.1, strip.width));
        }
    }
    let loops = union_exterior_loops(&polygons);
    let bounds = crate::plate_centre::loops_bounds(&loops).ok_or("learned patch is empty")?;
    let mut learned = footprint.clone();
    learned.width = bounds.max_x - bounds.min_x;
    learned.height = bounds.max_y - bounds.min_y;
    learned.bed_loops = Some(loops.clone());
    learned.loops = loops;
    Ok(learned)
}

/// Sliced object label → index of its placement, matched by the first-layer body centre
/// (within `MATCH_TOLERANCE_MM` of where the placement puts the footprint's landing zone, or
/// its body/patch when it has none). Objects without a body on the first layer or without a
/// placement that close are left out.
pub fn match_placements(
    footprint: &Footprint,
    placements: &[PackedPlacement],
    first_layer: &[ObjectFirstLayer],
) -> Result<HashMap<String, usize>, String> {
    // Where the first-layer body centre sits in the footprint frame.
    let reference = [
        &footprint.support_landing_loops,
        &footprint.body_loops,
        &footprint.loops,
    ]
    .into_iter()
    .find_map(|loops| crate::plate_centre::loops_bounds(loops))
    .ok_or("footprint has no loops")?;
    let offset = (
        (reference.min_x + reference.max_x) * 0.5,
        (reference.min_y + reference.max_y) * 0.5,
    );
    let mut matched = HashMap::new();
    for object in first_layer {
        let Some(centre) = object.body_center() else {
            continue;
        };
        let nearest = placements
            .iter()
            .enumerate()
            .map(|(index, placement)| {
                let (sin, cos) = placement.angle_deg.to_radians().sin_cos();
                let expected = (
                    placement.x_mm + offset.0 * cos - offset.1 * sin,
                    placement.y_mm + offset.0 * sin + offset.1 * cos,
                );
                (index, (centre.0 - expected.0).hypot(centre.1 - expected.1))
            })
            .filter(|(_, distance)| *distance <= MATCH_TOLERANCE_MM)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((index, _)) = nearest {
            matched.insert(object.label.clone(), index);
        }
    }
    Ok(matched)
}

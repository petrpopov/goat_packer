//! What the slicer printed as supports for one object of a sliced plate, as a
//! `Realization`: the input verification and calibration compare with the support model.

use std::collections::BTreeMap;

use geo::Polygon;

use crate::gcode_collisions::{KeptLine, LAYER_BAND_MM, object_lines};
use crate::geometry::{strip_polygon, union_exterior_loops};
use crate::object_print::{Loops, MemberId, Realization};

/// One object to read a realization of: its label in the sliced plate (`None` = the plate's
/// only object) and where its placement put the object's frame origin on the plate.
#[derive(Clone, Debug, PartialEq)]
pub struct SlicedObject {
    pub label: Option<String>,
    pub origin: (f64, f64),
}

/// What the slicer printed as supports for one object of a sliced plate (verification and
/// calibration compare it with the support model): the first-layer support as `base`, the
/// supports above it per absolute `LAYER_BAND_MM` band, and the interface part of each band.
/// `label` picks the object (`None` = the plate's only object).
///
/// The loops are in the object's plate-aligned frame for its rotation class: the slicer printed
/// the mesh already rotated, and supports are not rotation-equivariant, so nothing is rotated
/// back. Placing an `ObjectPrint` is a pure translation by `origin`, and undoing it is the
/// whole inverse.
pub fn realization_from_gcode(
    gcode: &[u8],
    label: Option<&str>,
    origin: (f64, f64),
    member: MemberId,
) -> Result<Realization, String> {
    let object = SlicedObject {
        label: label.map(str::to_owned),
        origin,
    };
    let mut realizations = realizations_from_gcode(gcode, &[object], member)?;
    Ok(realizations.remove(0))
}

/// `realization_from_gcode` for several objects of one plate, reading the G-code once.
pub fn realizations_from_gcode(
    gcode: &[u8],
    objects: &[SlicedObject],
    member: MemberId,
) -> Result<Vec<Realization>, String> {
    let (labels, lines) = object_lines(gcode);
    objects
        .iter()
        .map(|object| {
            let index = object_index(&labels, object.label.as_deref())?;
            Ok(realization_of(&lines, index, object.origin, member))
        })
        .collect()
}

fn object_index(labels: &[String], label: Option<&str>) -> Result<usize, String> {
    match label {
        Some(label) => labels
            .iter()
            .position(|known| known == label)
            .ok_or_else(|| format!("the slice has no object labelled {label}")),
        None if labels.len() == 1 => Ok(0),
        None => Err(format!(
            "the slice holds {} labelled objects; name the one to read",
            labels.len()
        )),
    }
}

/// The support lines of object `index` moved into its frame and unioned per kind. Only
/// exterior rings are kept: the gaps between support lines are no room for a neighbour (as in
/// the learned patch and the sample's bands). Empty bands keep their index, and the support and
/// interface lists have the same length.
fn realization_of(
    lines: &[KeptLine],
    index: usize,
    origin: (f64, f64),
    member: MemberId,
) -> Realization {
    let mut base = Vec::new();
    let mut support: BTreeMap<usize, Vec<Polygon>> = BTreeMap::new();
    let mut interface: BTreeMap<usize, Vec<Polygon>> = BTreeMap::new();
    for line in lines
        .iter()
        .filter(|line| line.object == index && line.strip.support)
    {
        let (from, to) = (line.strip.from, line.strip.to);
        let polygon = strip_polygon(
            from.0 - origin.0,
            from.1 - origin.1,
            to.0 - origin.0,
            to.1 - origin.1,
            line.strip.width,
        );
        if line.layer <= 1 {
            base.push(polygon);
            continue;
        }
        let band = (line.z.max(0.0) / LAYER_BAND_MM) as usize;
        if line.interface {
            interface.entry(band).or_default().push(polygon.clone());
        }
        support.entry(band).or_default().push(polygon);
    }
    let bands = support.keys().next_back().map_or(0, |last| last + 1);
    Realization {
        member,
        base: union_exterior_loops(&base),
        support: band_loops(support, bands),
        interface: band_loops(interface, bands),
    }
}

fn band_loops(mut bands: BTreeMap<usize, Vec<Polygon>>, count: usize) -> Vec<Loops> {
    (0..count)
        .map(|band| union_exterior_loops(&bands.remove(&band).unwrap_or_default()))
        .collect()
}

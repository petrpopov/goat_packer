//! What one object prints, split into typed kinds (design 2026-11-02, §4): the body per height
//! band, the first-layer landing, the support base and the support bands. Every collision rule
//! is a statement about kinds (`keepout::matrix`), so a kind is the unit the packer masks,
//! dilates and compares.

use crate::geometry::{Point, Rect, rotate_loops};
use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;

/// Height of one absolute band, mm. A band unions what its layers print, so a support and a
/// body in the same band but on different layers count as meeting: coarser bands are safer and
/// cost a little density; 2 mm keeps a 45 mm goat at ~23 bands.
pub const LAYER_BAND_MM: f64 = 2.0;

/// Closed rings in the footprint frame, mm. Several rings follow the even-odd rule, so holes
/// are just more rings (`ObjectPrint::part_hulls` is the exception: one ring per part).
pub type Loops = Vec<Vec<Point>>;

/// Position of a realization in the ensemble schedule (`support_model::ensemble`).
pub type MemberId = u32;

/// The kinds of printed geometry the permission matrix talks about. Band kinds exist once per
/// absolute 2 mm band; the tower kinds belong to the plate, not to an object.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrintKind {
    /// Union of every body band: what neighbours' bodies keep clearance from by default.
    Silhouette,
    BodyBand,
    /// The first-layer body (layer 0 only, not band 0).
    Landing,
    Brim,
    SupportBase,
    SupportBand,
    InterfaceBand,
    TowerBody,
    TowerBrim,
}

impl PrintKind {
    pub const ALL: [PrintKind; 9] = [
        PrintKind::Silhouette,
        PrintKind::BodyBand,
        PrintKind::Landing,
        PrintKind::Brim,
        PrintKind::SupportBase,
        PrintKind::SupportBand,
        PrintKind::InterfaceBand,
        PrintKind::TowerBody,
        PrintKind::TowerBrim,
    ];

    /// Support material: the kinds the slicer clips at the bed and the bed rule may let
    /// cross the printable edge (`PlateRules::support_clip_mm`).
    pub const fn is_support(self) -> bool {
        matches!(
            self,
            PrintKind::SupportBase | PrintKind::SupportBand | PrintKind::InterfaceBand
        )
    }
}

/// Who produced a print and with which margins; kept so a state file says what its supports
/// were predicted with.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Provenance {
    /// `"legacy-footprint"` for a slicer-learned `Footprint`, otherwise the slicer flavour.
    pub flavour: String,
    /// Ensemble members (or sliced runs) whose union the supports are.
    pub members: usize,
    pub margins: Margins,
    /// Support-model cache key (SHA-256 hex); empty without a model.
    pub cache_key: String,
}

/// Growth added to the unioned realizations to cover what the ensemble missed (design §5.5).
/// It describes the slicer, not the pair gap between neighbours.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Margins {
    pub first_layer_mm: f64,
    pub band_mm: f64,
}

/// One object at one rotation class, in the footprint frame.
///
/// Frame convention: every loop is already in the plate-aligned frame of `rotation_deg`, the
/// mesh turned by that angle about the frame centre (the origin). Supports are not
/// rotation-equivariant (the slicer grows trees in table coordinates), so they are computed
/// in that frame, not turned with the model. Placing a print on the plate is therefore a
/// pure translation of its frame origin; another rotation class is another `ObjectPrint`.
/// Only a legacy `Footprint` (`from_legacy`, rigid with the model) may be turned.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ObjectPrint {
    /// Extra Z rotation about the frame centre that this print was built for. Supports are
    /// rebuilt per rotation, so a print is never rotated to another class.
    pub rotation_deg: f64,
    pub silhouette: Loops,
    /// Convex hull of every normal part: the slicer's exclusion rule tests these.
    pub part_hulls: Loops,
    /// XY bbox of every normal part: printable bounds and the restricted band test these.
    pub part_boxes: Vec<Rect>,
    pub landing: Loops,
    pub brim: Loops,
    /// Index = absolute band `k` covering z in [k * LAYER_BAND_MM, (k + 1) * LAYER_BAND_MM).
    /// Empty and one-sided bands keep their index, so all three band lists stay aligned.
    pub body: Vec<Loops>,
    pub support_base: Loops,
    pub support: Vec<Loops>,
    pub interface: Vec<Loops>,
    pub provenance: Provenance,
}

impl ObjectPrint {
    /// The print turned rigidly by `angle_deg` (counter-clockwise) about its frame origin,
    /// part boxes rebuilt from the turned hulls (or the turned box corners when there is no
    /// hull per box). Exact for a legacy footprint; for a modelled class only an estimate of
    /// the class at `rotation_deg + angle_deg`, because tree supports are not
    /// rotation-equivariant (roof line angle, bed-edge obstacle): the plate orientation search
    /// screens turns with it and models only the turns it keeps.
    pub fn turned(&self, angle_deg: f64) -> ObjectPrint {
        if angle_deg == 0.0 {
            return self.clone();
        }
        let turn = |loops: &Loops| rotate_loops(loops, angle_deg);
        let bands = |bands: &[Loops]| bands.iter().map(turn).collect();
        let part_hulls = turn(&self.part_hulls);
        let part_boxes = if part_hulls.len() == self.part_boxes.len() {
            part_hulls
                .iter()
                .map(|hull| Rect::of_points(hull.iter()))
                .collect()
        } else {
            self.part_boxes
                .iter()
                .map(|b| {
                    let corners = [
                        Point { x: b.min_x, y: b.min_y },
                        Point { x: b.max_x, y: b.min_y },
                        Point { x: b.max_x, y: b.max_y },
                        Point { x: b.min_x, y: b.max_y },
                    ];
                    let turned = rotate_loops(&[corners.to_vec()], angle_deg);
                    Rect::of_points(turned.iter().flatten())
                })
                .collect()
        };
        ObjectPrint {
            rotation_deg: self.rotation_deg + angle_deg,
            part_boxes,
            silhouette: turn(&self.silhouette),
            part_hulls,
            landing: turn(&self.landing),
            brim: turn(&self.brim),
            body: bands(&self.body),
            support_base: turn(&self.support_base),
            support: bands(&self.support),
            interface: bands(&self.interface),
            provenance: self.provenance.clone(),
        }
    }

    /// The loops of `kind`; `band` selects within the band kinds and is ignored otherwise.
    /// Empty for a band the print has no entry for, and for the plate-level tower kinds.
    pub fn loops_of(&self, kind: PrintKind, band: usize) -> &[Vec<Point>] {
        match kind {
            PrintKind::Silhouette => &self.silhouette,
            PrintKind::BodyBand => band_loops(&self.body, band),
            PrintKind::Landing => &self.landing,
            PrintKind::Brim => &self.brim,
            PrintKind::SupportBase => &self.support_base,
            PrintKind::SupportBand => band_loops(&self.support, band),
            PrintKind::InterfaceBand => band_loops(&self.interface, band),
            PrintKind::TowerBody | PrintKind::TowerBrim => &[],
        }
    }

    /// Number of bands any band kind has an entry for.
    pub fn band_count(&self) -> usize {
        self.body
            .len()
            .max(self.support.len())
            .max(self.interface.len())
    }
}

fn band_loops(bands: &[Loops], band: usize) -> &[Vec<Point>] {
    bands.get(band).map_or(&[], Vec::as_slice)
}

/// What one slicer run (or one ensemble member) printed for an object: the raw input of the
/// envelope, whether it came from the support model or from G-code.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Realization {
    pub member: MemberId,
    /// The first-layer body the slicer printed (walls and infill, no support): the measured
    /// landing, to cross-check `first_layer::landing`.
    pub landing: Loops,
    pub base: Loops,
    /// `support` without layer 0: what a neighbour's body keeps `SUPPORT_BODY_GAP_MM` from.
    /// The base has its own, looser rule against the landing, which band 0 would undo if it
    /// carried the base too (the pair table tests `support[k]` against `body[k]`).
    pub above_base: Vec<Loops>,
    pub support: Vec<Loops>,
    pub interface: Vec<Loops>,
}

/// The z range `[from, to)` in mm that absolute band `band` covers.
pub fn band_z_range(band: usize) -> (f64, f64) {
    (
        band as f64 * LAYER_BAND_MM,
        (band + 1) as f64 * LAYER_BAND_MM,
    )
}

/// Slack of the band edges, mm: a layer top printed as 2.0000001 does not reach the next band.
const BAND_EDGE_EPS_MM: f64 = 1e-6;

/// The absolute bands a layer occupying `(z_bottom, z_top]` belongs to (design §4.1, the band
/// rule of every kind): each band it overlaps with positive length. A layer topping exactly on
/// a band boundary belongs only to the lower band; one starting exactly on it, only to the upper
/// (the P1S layer on (1.88, 2.16] is in bands 0 and 1).
pub fn layer_bands(z_bottom: f64, z_top: f64) -> RangeInclusive<usize> {
    let first = libm::floor((z_bottom + BAND_EDGE_EPS_MM) / LAYER_BAND_MM).max(0.0) as usize;
    let last = (libm::ceil((z_top - BAND_EDGE_EPS_MM) / LAYER_BAND_MM).max(1.0) as usize) - 1;
    first..=last.max(first)
}

#[cfg(test)]
mod tests {
    use super::{ObjectPrint, layer_bands};
    use crate::geometry::{Point, Rect};

    #[test]
    fn a_quarter_turn_turns_every_kind_and_rebuilds_the_part_boxes() {
        let p = |x, y| Point { x, y };
        let strip = vec![vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 1.0), p(0.0, 1.0), p(0.0, 0.0)]];
        let print = ObjectPrint {
            rotation_deg: 180.0,
            silhouette: strip.clone(),
            part_hulls: strip.clone(),
            part_boxes: vec![Rect::new(0.0, 0.0, 4.0, 1.0)],
            support_base: strip.clone(),
            support: vec![strip.clone()],
            ..ObjectPrint::default()
        };
        let turned = print.turned(90.0);
        assert_eq!(turned.rotation_deg, 270.0);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let corner = turned.support[0][0][2];
        assert!(close(corner.x, -1.0) && close(corner.y, 4.0), "{corner:?}");
        let b = turned.part_boxes[0];
        assert!(close(b.min_x, -1.0) && close(b.max_x, 0.0) && close(b.min_y, 0.0) && close(b.max_y, 4.0));
        let back = turned.turned(-90.0);
        assert!(close(back.silhouette[0][1].x, 4.0) && close(back.silhouette[0][1].y, 0.0));
        assert_eq!(print.turned(0.0).support_base, print.support_base);
    }

    #[test]
    fn a_layer_belongs_to_every_band_it_overlaps_with_positive_length() {
        let bands = |bottom, top| layer_bands(bottom, top).collect::<Vec<_>>();
        assert_eq!(bands(0.0, 0.2), vec![0]);
        assert_eq!(bands(1.88, 2.16), vec![0, 1]);
        // Topping exactly on the boundary: lower band only; starting on it: upper only.
        assert_eq!(bands(1.72, 2.0), vec![0]);
        assert_eq!(bands(2.0, 2.28), vec![1]);
        assert_eq!(bands(3.8, 4.1), vec![1, 2]);
        // A thick layer spans three bands.
        assert_eq!(bands(1.9, 4.1), vec![0, 1, 2]);
    }
}

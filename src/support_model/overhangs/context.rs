//! Inputs and constants of overhang detection (spec 01 §3.0), shared by every step.

use crate::embedded_mesh::{ObjectMesh, PartKind};
use crate::paint::{PaintState, project_paint};
use crate::polygon_ops::{
    DEFAULT_MITER_LIMIT, FillRule, Grid, Join, Rings, offset, resolve, union_all,
};
use crate::support_model::SupportError;
use crate::support_model::flavour::Flavour;
use crate::support_model::layers::LayerGrid;
use crate::support_model::sections::{LayerSections, extrusion_width, volume_layers};
use crate::support_model::settings::SupportSettings;

/// Enforced overhangs are grown by this "tip diameter fudge", fixed even when Orca's tip
/// diameter differs (B `TreeSupport.cpp` `support_tree_tip_diameter = 0.8`).
pub(super) const ENFORCER_GROW_MM: f64 = 0.8;

/// Blockers are grown by the tree collision resolution, an f32 constant (spec 01 §3.0 e_K).
pub(super) const BLOCKER_GROW_MM: f32 = 0.2;

/// An island with a box larger than this in both axes is well supported (sharp tails).
pub(super) const WELL_SUPPORTED_MM: f64 = 6.0;

/// Cantilever: some vertex lies farther than this from the supported part.
pub(super) const CANTILEVER_MM: f64 = 3.0;

/// Sharp tails stop being tails above this accumulated height (f32 compare).
pub(super) const TAIL_MAX_HEIGHT_MM: f32 = 16.0;

/// A tail grows too fast once the new part's box outgrows the tails below by this much.
pub(super) const TAIL_GROWTH_MM: f64 = 5.0;

/// Tail supports are the tail minus the lower layer grown by `RESOLUTION`.
pub(super) const RESOLUTION_MM: f64 = 0.0125;

/// Bambu's big-overhang threshold (`thresh_big_overhang` = 10 mm; area thresholds are its
/// square).
pub(super) const BIG_OVERHANG_MM: f64 = 10.0;

/// Bambu's small-overhang length (2 mm) and radius factor (2.5 line widths).
pub(super) const SMALL_OVERHANG_MM: f64 = 2.0;
pub(super) const SMALL_OVERHANG_RADIUS_WIDTHS: f64 = 2.5;

/// Hand-off: sharp-tail polygons are grown by this before tip seeding (spec 01 §3.8).
pub(super) const TAIL_HANDOFF_GROW_MM: f64 = 0.2;

/// Vertical enforcer points become discs of this radius, drawn with a chord error of 25
/// grid units (upstream passes 25 where an error is expected; spec 01 §2.6 step 6).
pub(super) const VERTICAL_DISC_MM: f64 = 0.5;
pub(super) const VERTICAL_DISC_ERROR_UNITS: f64 = 25.0;

/// Painted and volume enforcers / blockers per layer (stage A3 output on the flavour grid).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaintInput {
    /// E_k: enforcer volume sections plus down-facing enforcer paint, one entry per layer.
    pub enforcers: Vec<Rings>,
    /// K_k: the same for blockers.
    pub blockers: Vec<Rings>,
    /// Centroids of vertical enforcer facets, object frame (mm).
    pub vertical_points: Vec<[f64; 3]>,
}

impl PaintInput {
    /// No enforcers or blockers on `layers` layers.
    pub fn none(layers: usize) -> PaintInput {
        PaintInput {
            enforcers: vec![Rings::new(); layers],
            blockers: vec![Rings::new(); layers],
            vertical_points: Vec::new(),
        }
    }

    /// E_k and K_k of a mesh in the slicer's object frame: support enforcer / blocker volumes
    /// sliced at the planes, then the projected paint of the normal parts (spec 01 §2.6).
    pub fn from_mesh(
        mesh: &ObjectMesh,
        grid: &LayerGrid,
        flavour: Flavour,
    ) -> Result<PaintInput, SupportError> {
        let units = flavour.grid();
        let planes = grid.slice_planes();
        let coding = flavour.paint_state_coding();
        let painted = |state: PaintState| {
            project_paint(mesh, state, &planes, coding)
                .map_err(|error| SupportError::Invalid(format!("support paint: {error}")))
        };
        let enforcer_paint = painted(PaintState::Enforcer)?;
        let blocker_paint = painted(PaintState::Blocker)?;
        if flavour.blockers_are_obstacles()
            && (mesh
                .parts
                .iter()
                .any(|part| part.kind == PartKind::SupportBlocker)
                || blocker_paint.layers.iter().any(|layer| !layer.is_empty()))
        {
            return Err(SupportError::unsupported(
                "support_blocker",
                "present",
                "Orca 2.5 makes blockers branch obstacles, only sketched in spec 02 §2.3",
            ));
        }
        let combine = |volumes: Vec<Rings>, paint: &[crate::object_print::Loops]| {
            volumes
                .iter()
                .zip(paint)
                .map(|(volume, painted)| {
                    let projected = units.rings(painted);
                    union_all([volume, &projected])
                })
                .collect::<Vec<Rings>>()
        };
        Ok(PaintInput {
            enforcers: combine(
                volume_layers(mesh, PartKind::SupportEnforcer, grid, units),
                &enforcer_paint.layers,
            ),
            blockers: combine(
                volume_layers(mesh, PartKind::SupportBlocker, grid, units),
                &blocker_paint.layers,
            ),
            vertical_points: enforcer_paint.vertical_points,
        })
    }
}

/// Everything the detection steps read.
pub(super) struct Context<'a> {
    pub flavour: Flavour,
    pub grid: Grid,
    pub auto: bool,
    pub critical_only: bool,
    pub remove_small: bool,
    /// `enforce_support_layers`: the bottom layers whose step is −0.15·w.
    pub enforce_layers: usize,
    /// X_k.
    pub extrudable: &'a [Rings],
    /// Layer heights as f32, as upstream divides them.
    pub heights: Vec<f32>,
    /// Slice planes as f32 (vertical enforcer lookup).
    pub planes: Vec<f32>,
    pub enforcers: &'a [Rings],
    pub blockers: &'a [Rings],
    /// B trims trailing empty layers off both lists; these are the trimmed lengths.
    pub enforcer_count: usize,
    pub blocker_count: usize,
    pub vertical_points: &'a [[f64; 3]],
    /// w in mm and in units (`extrusion_width_scaled`, a double).
    pub width_mm: f64,
    pub width: f64,
    /// θₑ in radians.
    pub threshold: f64,
}

impl<'a> Context<'a> {
    pub fn new(
        grid: &LayerGrid,
        sections: &'a LayerSections,
        paint: &'a PaintInput,
        settings: &SupportSettings,
        flavour: Flavour,
    ) -> Context<'a> {
        let units = flavour.grid();
        let count = sections.extrudable.len();
        let width_mm = extrusion_width(settings, flavour);
        let trimmed = |layers: &[Rings]| {
            layers
                .iter()
                .take(count)
                .rposition(|layer| !layer.is_empty())
                .map_or(0, |last| last + 1)
        };
        Context {
            flavour,
            grid: units,
            auto: settings.is_auto(),
            critical_only: settings.critical_regions_only,
            remove_small: settings.remove_small_overhang,
            enforce_layers: settings.enforce_support_layers.max(0) as usize,
            extrudable: &sections.extrudable,
            heights: grid
                .layers
                .iter()
                .take(count)
                .map(|layer| layer.height_f32())
                .collect(),
            planes: grid
                .layers
                .iter()
                .take(count)
                .map(|layer| layer.slice_z as f32)
                .collect(),
            enforcers: &paint.enforcers,
            blockers: &paint.blockers,
            enforcer_count: trimmed(&paint.enforcers),
            blocker_count: trimmed(&paint.blockers),
            vertical_points: &paint.vertical_points,
            width_mm,
            width: units.scaled(width_mm),
            threshold: threshold_radians(settings.threshold_angle),
        }
    }

    pub fn layers(&self) -> usize {
        self.extrudable.len()
    }

    pub fn units(&self, mm: f64) -> f64 {
        self.grid.scaled(mm)
    }

    /// d_k in mm: the allowed outward step from the layer below (spec 01 §3.0).
    pub fn step_mm(&self, layer: usize) -> f64 {
        if layer < self.enforce_layers {
            -0.15 * self.width_mm
        } else {
            f64::from(self.heights[layer - 1]) / libm::tan(self.threshold)
        }
    }

    pub fn enforcer(&self, layer: usize) -> &Rings {
        &self.enforcers[layer]
    }

    pub fn blocker(&self, layer: usize) -> &Rings {
        &self.blockers[layer]
    }

    /// K_k ⊕ e_K, or nothing without blockers on the layer.
    pub fn grown_blocker(&self, layer: usize) -> Rings {
        let blocker = resolve(self.blocker(layer), FillRule::NonZero);
        if blocker.is_empty() {
            return blocker;
        }
        offset(
            &blocker,
            self.units(f64::from(BLOCKER_GROW_MM)),
            Join::Miter(DEFAULT_MITER_LIMIT),
        )
    }

    /// `offset_ex(.., d)` with the slicers' default mitre join.
    pub fn grow(&self, rings: &[crate::polygon_ops::Ring], mm: f64) -> Rings {
        offset(rings, self.units(mm), Join::Miter(DEFAULT_MITER_LIMIT))
    }
}

/// θₑ = θ + 1° (30° for θ ≤ 0), at most 89° (spec 01 §3.0).
fn threshold_radians(angle: f64) -> f64 {
    let degrees = if angle > 1e-4 { angle + 1.0 } else { 30.0 };
    degrees.min(89.0) * std::f64::consts::PI / 180.0
}

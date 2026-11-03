//! GUI-equivalent acceptance checks of a sliced plate (design 2026-11-02 §10.2), written from
//! the slicers' behaviour (zones research §2-§3, §6), not from their code.
//!
//! The slicer CLIs accept plates their GUIs refuse: they let toolpaths reach 2 mm past the
//! printable area and never test exclusion areas. So a CLI "Success" proves little, and these
//! checks ask what the GUI would say about the same G-code (plate frame):
//! - **toolpath extent**: the centre lines of every extrusion except machine G-code (`Custom`),
//!   the tower included, must stay inside the printable bbox within `GUI_BED_EPSILON_MM`. The
//!   GUI collects each move's end point and its arc samples; this check takes the arc's exact
//!   extent, which is at most a sagitta stricter;
//! - **exclusions**: no extrusion segment (arcs cut into chords) may touch an exclusion polygon
//!   (BambuStudio tests every segment exactly; this check uses the exact polygons for every
//!   flavour);
//! - **conflicts**: the "G-code path conflicts" rule (`conflicts`) between objects, and between
//!   an object and the tower lines the target build compares (`GuiRules::tower_lines`). A
//!   support line over a compared tower brim loop is a conflict like any other: Snapmaker
//!   Orca 2.4.0 showed "Serious warning ... WipeTower <-> object" for it on the owner's U1
//!   plate while the CLI slice of the same plate passed;
//! - **part edge band**: Snapmaker Orca warns when a model part's bbox comes closer than
//!   `SNAPMAKER_SPIRAL_LIFT_BAND_MM` to the printable edge (`part_edge_hits`, from the layout;
//!   no G-code needed).
//!
//! The material checks of `analyze_plate` (object coverage, material past the plate including
//! the half width, supports touching foreign bodies) stay as they are; these come on top.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use super::conflicts::{ConflictKind, ConflictLine, HitKey, Owner, layer_conflicts};
pub use super::conflicts::TowerLines;
use super::plane::{Vec2, segment_meets_rect, segment_meets_ring};
use super::reader::{Extrusion, ExtrusionSink, Role, read};
use crate::geometry::{Point, Rect};

pub use super::part_edges::{
    PartEdgeHit, PartEdgeIssue, SNAPMAKER_SPIRAL_LIFT_BAND_MM, part_edge_hits, placed_part_boxes,
};

/// The GUIs' bed tolerance (three times the geometry epsilon): toolpath centre lines may reach
/// this far past the printable bbox.
pub const GUI_BED_EPSILON_MM: f64 = 3e-4;

/// Tower lines that stay this far clear of the tower body are its brim or chamfer rings.
const TOWER_BODY_TOLERANCE_MM: f64 = 0.05;

/// What the plate is checked against, plate frame.
#[derive(Clone, Debug, PartialEq)]
pub struct GuiRules {
    /// Bbox of the printable area.
    pub printable: Rect,
    /// Exclusion areas as closed rings (BambuStudio's rectangles, Orca's polygon).
    pub exclusions: Vec<Vec<Point>>,
    /// Which tower lines the target build's path-conflict rule compares with the objects'.
    pub tower_lines: TowerLines,
}

/// Who printed a line.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Party {
    Object(String),
    Tower,
    /// An extrusion outside every object label.
    Unlabelled,
}

impl fmt::Display for Party {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Party::Object(label) => write!(f, "object {label}"),
            Party::Tower => f.write_str("the prime tower"),
            Party::Unlabelled => f.write_str("an unlabelled extrusion"),
        }
    }
}

/// Extrusions of one party inside one exclusion area.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExclusionHit {
    /// Index into `GuiRules::exclusions`.
    pub exclusion: usize,
    pub party: Party,
    /// 1-based; 0 = before the first layer marker.
    pub first_layer: usize,
    pub segments: usize,
    /// Middle of the first such segment.
    pub at: (f64, f64),
}

/// A conflicting pair of parties, aggregated over layers.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Conflict {
    pub first: Party,
    pub second: Party,
    /// Of the first conflicting lines on the first layer.
    pub kind: ConflictKind,
    pub first_layer: usize,
    pub last_layer: usize,
    pub layers: usize,
    /// Where the first conflicting lines meet on the first layer.
    pub at: (f64, f64),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GuiReport {
    pub printable: Rect,
    /// Centre-line extent of every non-`Custom` extrusion; `None` when nothing printed.
    pub toolpath_extent: Option<Rect>,
    /// The extent leaves the printable bbox by more than `GUI_BED_EPSILON_MM`.
    pub outside_plate: bool,
    pub exclusion_hits: Vec<ExclusionHit>,
    pub conflicts: Vec<Conflict>,
}

impl GuiReport {
    /// Every finding that makes the plate unacceptable, as a message.
    pub fn errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.outside_plate
            && let Some(extent) = self.toolpath_extent
        {
            errors.push(format!(
                "toolpaths reach {extent:?}, past the printable area {:?}",
                self.printable
            ));
        }
        errors.extend(self.exclusion_hits.iter().map(|hit| {
            format!(
                "{} prints into exclusion area {} from layer {} ({} segments, first at {:.2}, {:.2})",
                hit.party, hit.exclusion, hit.first_layer, hit.segments, hit.at.0, hit.at.1
            )
        }));
        errors.extend(self.conflicts.iter().map(|conflict| {
            format!(
                "G-code path conflict ({:?}) between {} and {} on layers {}-{} ({} layers), \
                 first at {:.2}, {:.2}",
                conflict.kind,
                conflict.first,
                conflict.second,
                conflict.first_layer,
                conflict.last_layer,
                conflict.layers,
                conflict.at.0,
                conflict.at.1
            )
        }));
        errors
    }

    pub fn require_accepted(&self) -> Result<(), String> {
        let errors = self.errors();
        if errors.is_empty() {
            return Ok(());
        }
        Err(format!(
            "the slicer GUI would refuse this plate: {}",
            errors.join("; ")
        ))
    }
}

/// Runs the G-code checks on a sliced plate whose coordinates are already in the plate frame.
/// Reads the G-code twice: the tower body must be known before its brim can be told apart.
pub fn check_plate(gcode: &[u8], rules: &GuiRules) -> GuiReport {
    let mut checks = PlateChecks::new(rules, tower_body(gcode));
    let labels = read(gcode, &mut checks);
    checks.report(&labels)
}

/// The tower body: the brim and the chamfer only ever make a layer of the tower wider than
/// the layers above it, so the body is the union of the tower's extents over the upper half of
/// its layers. (A layer-by-layer intersection fails on Orca towers, whose sparse layers print
/// only part of the body.)
fn tower_body(gcode: &[u8]) -> Option<Rect> {
    let mut layers = TowerLayers::default();
    read(gcode, &mut layers);
    let extents: Vec<Rect> = layers.extents.into_values().collect();
    extents[extents.len() / 2..]
        .iter()
        .copied()
        .reduce(Rect::union)
}

/// Centre-line extent of the tower's (non-brim) lines per layer.
#[derive(Default)]
struct TowerLayers {
    extents: BTreeMap<usize, Rect>,
}

impl ExtrusionSink for TowerLayers {
    fn extrusion(&mut self, extrusion: &Extrusion) {
        if extrusion.role == Role::Tower {
            let extent = extrusion.extent;
            self.extents
                .entry(extrusion.layer)
                .and_modify(|seen| *seen = seen.union(extent))
                .or_insert(extent);
        }
    }
}

/// Exclusion hits so far: first layer, segment count, where the first one was.
type ExclusionTally = (usize, usize, Vec2);

/// Conflicts so far: kind and place of the first, first layer, last layer, layers.
type ConflictTally = (ConflictKind, Vec2, usize, usize, usize);

struct PlateChecks<'a> {
    rules: &'a GuiRules,
    exclusion_boxes: Vec<Rect>,
    tower_body: Option<Rect>,
    extent: Option<Rect>,
    exclusion_hits: BTreeMap<(usize, Option<Owner>), ExclusionTally>,
    lines: Vec<ConflictLine>,
    conflicts: BTreeMap<HitKey, ConflictTally>,
}

impl ExtrusionSink for PlateChecks<'_> {
    fn extrusion(&mut self, extrusion: &Extrusion) {
        if extrusion.role == Role::Custom {
            return;
        }
        let reach = if extrusion.arc {
            extrusion.extent
        } else {
            let (x, y) = extrusion.end();
            Rect::new(x, y, x, y)
        };
        self.extent = Some(self.extent.map_or(reach, |seen| seen.union(reach)));
        let owner = owner_of(extrusion);
        for pair in extrusion.path.windows(2) {
            self.check_exclusions(extrusion.layer, owner, pair[0], pair[1]);
        }
        if extrusion.layer > 0
            && !extrusion.is_smear()
            && let Some(owner) = owner
        {
            for pair in extrusion.path.windows(2) {
                if self.takes_part(extrusion.role, pair[0], pair[1]) {
                    self.lines.push(ConflictLine {
                        owner,
                        from: pair[0],
                        to: pair[1],
                        support: extrusion.role.is_support(),
                    });
                }
            }
        }
    }

    fn layer_done(&mut self, layer: usize) {
        let lines = std::mem::take(&mut self.lines);
        for (key, hit) in layer_conflicts(&lines) {
            self.conflicts
                .entry(key)
                .and_modify(|tally| {
                    tally.3 = layer;
                    tally.4 += 1;
                })
                .or_insert((hit.kind, hit.at, layer, layer, 1));
        }
    }
}

impl<'a> PlateChecks<'a> {
    fn new(rules: &'a GuiRules, tower_body: Option<Rect>) -> Self {
        let exclusion_boxes = rules
            .exclusions
            .iter()
            .map(|ring| Rect::of_points(ring.iter()))
            .collect();
        Self {
            rules,
            exclusion_boxes,
            tower_body,
            extent: None,
            exclusion_hits: BTreeMap::new(),
            lines: Vec::new(),
            conflicts: BTreeMap::new(),
        }
    }

    fn check_exclusions(&mut self, layer: usize, owner: Option<Owner>, a: Vec2, b: Vec2) {
        for (index, ring) in self.rules.exclusions.iter().enumerate() {
            if !segment_meets_rect(a, b, self.exclusion_boxes[index])
                || !segment_meets_ring(a, b, ring)
            {
                continue;
            }
            let middle = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
            self.exclusion_hits
                .entry((index, owner))
                .and_modify(|tally| tally.1 += 1)
                .or_insert((layer, 1, middle));
        }
    }

    /// Whether the rule compares this segment: an object's walls, infill and supports always
    /// (its brim and the skirt never), the tower's lines as far as the build's `TowerLines`
    /// says. With `All` the tower's interior lines count too; that errs on the strict side and
    /// no support can reach them without entering the column anyway.
    fn takes_part(&self, role: Role, from: Vec2, to: Vec2) -> bool {
        match role {
            Role::Custom | Role::Skirt | Role::Brim => false,
            Role::TowerBrim => self.rules.tower_lines != TowerLines::None,
            Role::Tower => match self.rules.tower_lines {
                TowerLines::None => false,
                TowerLines::All => true,
                // BambuStudio-style G-code does not mark the brim: tell it by distance.
                TowerLines::BrimLoops => self.tower_body.is_some_and(|body| {
                    !segment_meets_rect(from, to, body.grown(TOWER_BODY_TOLERANCE_MM))
                }),
            },
            _ => true,
        }
    }

    fn report(self, labels: &[String]) -> GuiReport {
        let party = |owner: Option<Owner>| match owner {
            Some(Owner::Object(index)) => Party::Object(labels[index].clone()),
            Some(Owner::Tower) => Party::Tower,
            None => Party::Unlabelled,
        };
        let exclusion_hits = self.exclusion_hits.into_iter().map(
            |((exclusion, owner), (first_layer, segments, at))| ExclusionHit {
                exclusion,
                party: party(owner),
                first_layer,
                segments,
                at,
            },
        );
        let conflicts = self.conflicts.into_iter().map(
            |((first, second), (kind, at, first_layer, last_layer, layers))| Conflict {
                first: party(Some(first)),
                second: party(Some(second)),
                kind,
                first_layer,
                last_layer,
                layers,
                at,
            },
        );
        GuiReport {
            printable: self.rules.printable,
            toolpath_extent: self.extent,
            outside_plate: self.extent.is_some_and(|extent| {
                extent.sticks_out_of(self.rules.printable, GUI_BED_EPSILON_MM)
            }),
            exclusion_hits: exclusion_hits.collect(),
            conflicts: conflicts.collect(),
        }
    }
}

/// The tower, or the object a line belongs to; `None` for unlabelled object lines.
fn owner_of(extrusion: &Extrusion) -> Option<Owner> {
    if extrusion.role.is_tower() {
        return Some(Owner::Tower);
    }
    extrusion.object.map(Owner::Object)
}

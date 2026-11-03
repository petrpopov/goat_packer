//! The optional verification: slice the assembled plate and ask what the slicer GUI and the
//! plate checks say about its G-code. A failure teaches the plan what the slice really
//! printed (`learn`) and the plate is packed again, once; whatever is left is fatal, never a
//! successful report with remaining collisions.

use goat_packer::{
    PrintPlacement, Rect,
    gcode_collisions::{PlateAnalysis, SupportCollision, TowerLayer, analyze_plate, tower_layers},
    gcode_collisions::gui_checks::{GuiRules, check_plate},
    keepout::KeepoutShape,
    progress::{
        self, LayoutSource, RepackReason, Stage, expected_reslice_seconds, expected_slice_seconds,
    },
};

use crate::AppResult;
use crate::assembly::{build_colored_project, tower_rect};
use crate::job::Job;
use crate::learn::learn_supports;
use crate::plan::PackState;
use crate::prepare::arrange;
use crate::report::Verification;
use crate::slicing::{SliceOutput, slice_project};
use crate::tower::widened;

/// The G-code check is quick next to a slice; the progress bar expects this long.
const EXPECTED_GCODE_CHECK_SECONDS: f64 = 5.0;
/// Seconds a one-object slice takes on the server (design measures), when no probe ran.
const DEFAULT_PROBE_SECONDS: f64 = 22.0;
/// How far the sliced tower may reach past the reserved box: the tolerance of the tower
/// model (`TOWER_EDGE_TOLERANCE_MM`).
const TOWER_SLACK_MM: f64 = 0.1;

/// A packed plate on its way through verification. What a slice teaches is learned once:
/// one repack, and a second failure is fatal.
pub struct PlateRun<'a> {
    pub job: &'a Job,
    pub state: PackState,
    pub repacked: bool,
    /// (seconds, objects) of the last full-plate slice: later slices are expected to scale it.
    measured_slice: Option<(f64, usize)>,
    /// The webapp scene draws every winning layout; the first one is "pack", later ones "repack".
    layout_source: LayoutSource,
}

/// What the passing slice showed.
pub struct Verified {
    pub slice: SliceOutput,
    pub verification: Verification,
    pub support_collisions: Vec<SupportCollision>,
}

/// What a slice showed against the plate's rules.
#[derive(Debug, Default)]
pub struct Findings {
    /// Layers of the slice.
    layers: usize,
    /// Box of the tower the slice printed.
    tower_box: Option<Rect>,
    off_plate: Vec<String>,
    collisions: Vec<SupportCollision>,
    gui: Vec<String>,
    /// The sliced tower, when it reaches past the reserved box.
    outgrown_tower: Option<Vec<TowerLayer>>,
}

impl Findings {
    pub fn is_clean(&self) -> bool {
        self.off_plate.is_empty()
            && self.collisions.is_empty()
            && self.gui.is_empty()
            && self.outgrown_tower.is_none()
    }

    pub fn describe(&self) -> String {
        let mut lines = self.off_plate.clone();
        lines.extend(self.gui.clone());
        if !self.collisions.is_empty() {
            lines.push(format!("{} support/body collision(s)", self.collisions.len()));
        }
        if self.outgrown_tower.is_some() {
            lines.push("the prime tower reaches past the reserved box".to_owned());
        }
        lines.join("; ")
    }
}

impl<'a> PlateRun<'a> {
    pub fn new(job: &'a Job, state: PackState) -> Self {
        Self {
            job,
            state,
            repacked: false,
            measured_slice: None,
            layout_source: LayoutSource::Pack,
        }
    }

    /// Builds and slices the plate until the checks accept it.
    pub fn verify(&mut self) -> AppResult<Verified> {
        loop {
            self.build()?;
            let count = self.state.placements.len();
            let slice = self.slice(count)?;
            progress::stage(
                Stage::GcodeCheck,
                &format!("checking the plate layer by layer ({count} objects)..."),
                Some(EXPECTED_GCODE_CHECK_SECONDS),
                Some(count),
            );
            let findings = self.check(&slice.gcode, count)?;
            if findings.is_clean() {
                return Ok(self.verified(slice, findings));
            }
            println!("verification found: {}", findings.describe());
            if self.repacked {
                return Err(format!(
                    "the slice still shows problems after learning what it printed and packing \
                     again; refusing an unsafe result: {}",
                    findings.describe()
                )
                .into());
            }
            self.learn_and_repack(&slice.gcode, findings)?;
        }
    }

    /// Orders and publishes the layout, then writes the 3MF.
    pub fn build(&mut self) -> AppResult<()> {
        self.show_layout(self.layout_source);
        self.layout_source = LayoutSource::Repack;
        let count = self.state.placements.len();
        progress::stage(
            Stage::Assembling,
            &format!("packed {count} objects; assembling 3MF..."),
            None,
            Some(count),
        );
        self.build_plate(&self.state.placements)
    }

    /// Writes the 3MF of `placements` (the plate, or a window's prefix of it) after the exact
    /// check of the layout: whatever changed since packing (a resumed state, a prefix) is
    /// validated before any artifact is written.
    pub fn build_plate(&self, placements: &[PrintPlacement]) -> AppResult<()> {
        let state = &self.state;
        state.plan.validate(&self.job.args, placements, state.tower_anchor)?;
        build_colored_project(self.job, &state.plan, placements, state.tower_anchor)
    }

    /// Tells the webapp which layout is being built.
    pub fn show_layout(&self, source: LayoutSource) {
        crate::prepare::show_layout(&self.state, source);
    }

    /// How long a slice of `count` objects should take, for the progress bar.
    pub fn expected_slice_seconds(&self, count: usize) -> f64 {
        match self.measured_slice {
            Some((seconds, objects)) => expected_reslice_seconds(seconds, objects, count),
            None if self.state.probe_seconds > 0.0 => {
                expected_slice_seconds(self.state.probe_seconds, count)
            }
            None => expected_slice_seconds(DEFAULT_PROBE_SECONDS, count),
        }
    }

    fn slice(&mut self, count: usize) -> AppResult<SliceOutput> {
        progress::stage(
            Stage::FinalSlice,
            &format!("slicing final plate ({count} objects)..."),
            Some(self.expected_slice_seconds(count)),
            Some(count),
        );
        let label = if self.repacked { "final_repacked" } else { "final_first" };
        let started = std::time::Instant::now();
        let job = self.job;
        let slice = slice_project(&job.profile, &job.args.output, count, &job.args.work_dir, label)?;
        self.measured_slice = Some((started.elapsed().as_secs_f64(), count));
        Ok(slice)
    }

    /// Every check of one slice. Missing object labels or a missing or invented tower cannot
    /// be learned from and are errors on the spot.
    pub fn check(&self, gcode: &[u8], count: usize) -> AppResult<Findings> {
        let state = &self.state;
        let analysis = analyze_plate(gcode);
        analysis.require_object_coverage(count)?;
        let layers = tower_layers(gcode);
        let reserved = tower_rect(&state.plan, state.tower_anchor);
        match (reserved, layers.is_empty()) {
            (Some(_), true) => {
                return Err("the slice has no prime tower, though one was reserved".into());
            }
            (None, false) => {
                return Err(
                    "a prime tower appeared in the slice, though the probes had none and none \
                     was reserved"
                        .into(),
                );
            }
            _ => {}
        }
        let rules = &state.plan.rules;
        let gui = check_plate(gcode, &gui_rules(rules.printable_area, rules));
        Ok(Findings {
            layers: analysis.layers,
            tower_box: outer_box(&layers),
            off_plate: off_plate(&analysis, rules.printable_area),
            collisions: analysis.collisions.clone(),
            gui: gui.errors(),
            outgrown_tower: outgrown(&layers, reserved),
        })
    }

    /// Learns what the slice printed, then packs again and publishes the new layout.
    fn learn_and_repack(&mut self, gcode: &[u8], findings: Findings) -> AppResult<()> {
        let state = &mut self.state;
        if !findings.off_plate.is_empty() || !findings.collisions.is_empty() {
            let analysis = analyze_plate(gcode);
            learn_supports(&mut state.plan, gcode, &state.placements, &analysis)?;
        }
        if let (Some(layers), Some(spec), Some(anchor)) =
            (findings.outgrown_tower, &state.plan.tower, state.tower_anchor)
        {
            state.plan.tower = Some(widened(spec, &layers, anchor));
        }
        progress::repack(
            RepackReason::SupportContact,
            "re-packing against the geometry the slice printed...",
        );
        arrange(self.job, state)?;
        self.repacked = true;
        Ok(())
    }

    fn verified(&self, slice: SliceOutput, findings: Findings) -> Verified {
        Verified {
            verification: Verification {
                slicer: self.job.profile.slicer_path.clone(),
                layers: findings.layers,
                tower: findings.tower_box,
                gui_checks: "passed",
            },
            support_collisions: findings.collisions,
            slice,
        }
    }
}

/// Objects whose printed material (line width included) leaves the printable area.
fn off_plate(analysis: &PlateAnalysis, printable: Rect) -> Vec<String> {
    analysis
        .off_plate(printable)
        .into_iter()
        .map(|(label, extent)| {
            format!(
                "object {label} prints x [{:.2}, {:.2}] y [{:.2}, {:.2}], past the printable area",
                extent.min_x, extent.max_x, extent.min_y, extent.max_y
            )
        })
        .collect()
}

/// The layers when their box leaves `reserved` by more than the tower tolerance.
fn outgrown(layers: &[TowerLayer], reserved: Option<Rect>) -> Option<Vec<TowerLayer>> {
    let reserved = reserved?.grown(TOWER_SLACK_MM);
    let printed = outer_box(layers)?;
    printed.sticks_out_of(reserved, 0.0).then(|| layers.to_vec())
}

fn outer_box(layers: &[TowerLayer]) -> Option<Rect> {
    layers.iter().map(|layer| layer.extent).reduce(Rect::union)
}

/// The GUI's printable bbox and the plate's hard rectangles and polygons as exclusion areas
/// (strips and edge bands are tested by the matrix rows, not by the GUI).
pub fn gui_rules(printable: Rect, rules: &goat_packer::PlateRules) -> GuiRules {
    let ring = |rect: Rect| {
        [(rect.min_x, rect.min_y), (rect.max_x, rect.min_y), (rect.max_x, rect.max_y), (rect.min_x, rect.max_y)]
            .map(|(x, y)| goat_packer::Point { x, y })
            .to_vec()
    };
    let exclusions = rules
        .keepouts
        .iter()
        .filter(|keepout| keepout.class == goat_packer::KeepoutClass::Hard)
        .filter_map(|keepout| match &keepout.shape {
            KeepoutShape::Rect(rect) => Some(ring(*rect)),
            KeepoutShape::Polygon(points) => Some(points.clone()),
            KeepoutShape::Strips(_) | KeepoutShape::EdgeBand(_) => None,
        })
        .collect();
    GuiRules { printable, exclusions }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepare::arrange;
    use crate::test_support::{test_job, test_plan, test_state};
    use goat_packer::{Point, pack::TowerSpec};

    const COLLISION: &str = "; CHANGE_LAYER\n; Z_HEIGHT: 0.2\n; start printing object, unique label id: 1\n; FEATURE: Support\nG1 X4 Y5\nG1 X7 Y5 E1\n; stop printing object, unique label id: 1\n; start printing object, unique label id: 2\n; FEATURE: Outer wall\nG1 X4 Y5.1\nG1 X7 Y5.1 E1\n";

    fn run<'a>(job: &'a Job, tower: Option<TowerSpec>) -> PlateRun<'a> {
        let mut plan = test_plan(job, 2);
        plan.tower = tower;
        let mut state = test_state(plan);
        state.placements = vec![PrintPlacement { print: 0, x_mm: 5.0, y_mm: 5.0 }];
        PlateRun::new(job, state)
    }

    #[test]
    fn a_support_touching_a_foreign_body_is_a_finding() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let findings = run(&job, None).check(COLLISION.as_bytes(), 2).unwrap();
        assert_eq!(findings.collisions.len(), 1);
        assert!(!findings.is_clean());
        assert!(findings.describe().contains("collision"));
    }

    #[test]
    fn material_past_the_printable_area_is_a_finding() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let gcode = COLLISION.replace("X7 Y5 E1", "X70 Y5 E1");
        let findings = run(&job, None).check(gcode.as_bytes(), 2).unwrap();
        assert_eq!(findings.off_plate.len(), 1, "{findings:?}");
    }

    #[test]
    fn a_slice_that_cannot_be_attributed_to_objects_is_an_error_not_a_pass() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let error = run(&job, None)
            .check(b"; CHANGE_LAYER\nG1 X1 Y1\nG1 X2 Y1 E1\n", 2)
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot verify slice"), "{error}");
    }

    #[test]
    fn a_reserved_tower_that_the_slice_lacks_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let spec = TowerSpec {
            body: Rect::new(0.0, 0.0, 10.0, 10.0),
            brim_l0_mm: 0.0,
            chamfer: Vec::new(),
            top_z_mm: 5.0,
            anchor_range: Rect::new(0.0, 0.0, 40.0, 40.0),
            source_anchor: Point { x: 20.0, y: 20.0 },
        };
        let mut plate = run(&job, Some(spec));
        plate.state.tower_anchor = Some(Point { x: 20.0, y: 20.0 });
        let error = plate.check(COLLISION.as_bytes(), 2).unwrap_err().to_string();
        assert!(error.contains("no prime tower"), "{error}");
    }

    #[test]
    fn what_the_slice_printed_is_learned_and_the_plate_is_packed_again_once() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let mut plate = run(&job, None);
        arrange(&job, &mut plate.state).unwrap();
        // One copy at the body's position, so the second object of the slice matches it.
        plate.state.placements = vec![PrintPlacement { print: 0, x_mm: 5.5, y_mm: 5.1 }];
        let before = plate.state.plan.prints[0].support_base.clone();
        let findings = plate.check(COLLISION.as_bytes(), 2).unwrap();
        // The support object has no first-layer body: only the body object is matched, so
        // there is nothing to learn from the support's own lines, and the repack still runs.
        plate.learn_and_repack(COLLISION.as_bytes(), findings).unwrap();
        assert!(plate.repacked);
        assert!(plate.state.placements.len() > 1);
        assert_eq!(plate.state.plan.prints[0].support_base, before);
    }
}

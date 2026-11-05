//! Moving one node down one layer (spec 02 §3.3-§3.7): the areas it may reach at each speed,
//! then the settings of `settings` tried in order; the first whose reachable area (outside
//! the avoidance) is not empty wins, and the node that results stores everything the merge
//! and placement stages need.

use super::Moved;
use super::settings::{Setting, Speed, settings_of};
use super::soi::{StepParams, grow_through};
use super::speeds::{Speeds, speeds};
use crate::polygon_ops::{Grid, IntBox, IntPoint, Rings, area, bbox, difference, union};
use crate::support_model::clearance::Avoidance;
use crate::support_model::flavour::Flavour;
use crate::support_model::node::Node;
use crate::support_model::stage_a::StageA;

/// The box around a candidate's areas is padded by this (mm, spec 02 §3.7).
const BOX_PADDING_MM: f64 = 0.0001;

/// Half width of the ribbons that keep a degenerate area growable (mm, spec 02 §3.4).
const RIBBON_HALF_WIDTH_MM: f64 = 0.005;

/// 1.5 times the fast speed is allowed for the last-resort jump.
const ERROR_SPEED_FACTOR: f64 = 1.5;

/// The state shared by the settings tried for one parent.
pub(super) struct Mover<'a> {
    stage: &'a StageA,
    parent: &'a Node,
    layer: usize,
    speeds: Speeds,
    walls: std::sync::Arc<Rings>,
    safe_step: i64,
    epsilon: f64,
    slow: Option<Rings>,
    fast: Option<Rings>,
    error: Option<Rings>,
}

/// Moves `parent` (on `layer`) down; `None` when no setting is valid.
pub(super) fn step_down(stage: &StageA, parent: &Node, layer: usize) -> Option<Moved> {
    let mut mover = Mover::new(stage, parent, layer);
    for setting in settings_of(&stage.derived, parent) {
        if let Some(moved) = mover.try_setting(&setting) {
            return Some(moved);
        }
    }
    None
}

impl<'a> Mover<'a> {
    fn new(stage: &'a StageA, parent: &'a Node, layer: usize) -> Mover<'a> {
        let derived = &stage.derived;
        let radius = parent.collision_radius(derived);
        let walls = stage
            .clearance
            .wall_restriction(radius, layer, parent.tight_xy);
        let mut safe_step = if parent.tight_xy {
            derived.xy_min
        } else {
            derived.xy
        };
        if derived.z_top_layers.min(derived.z_bottom_layers) > 0 {
            safe_step += derived.min_feature_size;
        }
        if safe_step == 0 && matches!(derived.flavour, Flavour::Orca242 | Flavour::Orca250) {
            safe_step = derived.grid.length(0.1);
        }
        let tiny = derived.grid.scaled(0.001);
        Mover {
            stage,
            parent,
            layer,
            speeds: speeds(stage, parent, layer),
            walls,
            safe_step,
            epsilon: tiny * tiny,
            slow: None,
            fast: None,
            error: None,
        }
    }

    fn params(&self, careless: i64, parts: i64) -> StepParams {
        StepParams {
            grid: self.stage.derived.grid,
            strict: self.stage.flavour.strictly_simple(),
            safe_step: self.safe_step,
            careless,
            parts,
        }
    }

    fn reach(&mut self, speed: Speed, error: bool) -> &Rings {
        let radius = self.parent.collision_radius(&self.stage.derived);
        let careless = self.safe_step + radius;
        if error {
            if self.error.is_none() {
                let distance = (ERROR_SPEED_FACTOR * self.speeds.fast as f64) as i64;
                let base = with_ribbons(&self.parent.area, &self.stage.derived.grid);
                self.error = Some(grow_through(
                    &base,
                    distance,
                    &self.walls,
                    &self.params(careless, 1),
                ));
            }
            return self.error.as_ref().expect("computed above");
        }
        if self.slow.is_none() {
            self.slow = Some(grow_through(
                &self.parent.area,
                self.speeds.slow,
                &self.walls,
                &self.params(0, 2),
            ));
        }
        if speed == Speed::Slow {
            return self.slow.as_ref().expect("computed above");
        }
        if self.fast.is_none() {
            let slow = self.slow.as_ref().expect("computed above");
            self.fast = Some(grow_through(
                slow,
                self.speeds.fast - self.speeds.slow,
                &self.walls,
                &self.params(careless, 1),
            ));
        }
        self.fast.as_ref().expect("computed above")
    }

    fn try_setting(&mut self, setting: &Setting) -> Option<Moved> {
        let stage = self.stage;
        let derived = &stage.derived;
        let mut child = self.parent.clone();
        child.layer -= 1;
        child.depth += 1;
        child.centre = None;
        child.keep_round = false;
        if setting.grow_radius {
            child.rdepth += 1;
        }
        let below = self.layer - 1;
        let reachable: Rings = if setting.moves {
            self.reach(setting.speed, setting.error).clone()
        } else {
            self.parent.area.clone()
        };
        let avoid = |radius: i64| {
            stage
                .clearance
                .avoidance(radius, below, setting.avoidance, setting.tight_xy)
        };
        let mut radius = child.collision_radius(derived);
        let mut plate = difference(&reachable, &avoid(radius));
        if !child.to_bed && area(&plate) > self.epsilon {
            child.to_bed = true;
        }
        let keys = stage.clearance.keys();
        let valid_radius = |candidate: i64, plate_radius: i64| {
            keys.key(candidate, setting.tight_xy) <= keys.key(plate_radius, setting.tight_xy)
                || area(&difference(&reachable, &avoid(candidate))) > self.epsilon
        };
        if setting.grow_radius && area(&plate) > self.epsilon {
            let key_before = keys.key(radius, setting.tight_xy);
            self.catch_up(&mut child, setting, &valid_radius);
            self.grow_foot(&mut child, below, &valid_radius);
            radius = child.collision_radius(derived);
            if keys.key(radius, setting.tight_xy) != key_before {
                plate = difference(&reachable, &avoid(radius));
            }
        }
        if area(&plate) <= self.epsilon {
            return None;
        }
        Some(self.finish(child, setting, &reachable, plate))
    }

    /// A radius that lags behind its depth catches up while the bigger avoidance still
    /// leaves room (spec 02 §3.6 step 6).
    fn catch_up(&self, child: &mut Node, setting: &Setting, valid: &impl Fn(i64, i64) -> bool) {
        let derived = &self.stage.derived;
        let keys = self.stage.clearance.keys();
        let radius = child.collision_radius(derived);
        let real = child.real_radius(derived);
        if radius >= derived.increase_until_radius || radius >= real {
            return;
        }
        let target = real.min(derived.increase_until_radius);
        let mut chain = keys.next_key(radius, setting.tight_xy);
        while chain < target {
            let next = keys.next_key(chain + 1, setting.tight_xy);
            if !valid(next, radius) {
                break;
            }
            chain = next;
        }
        let limit = chain.min(real);
        let best = (child.rdepth..child.depth)
            .rev()
            .find(|&candidate| derived.radius(candidate, child.foot) <= limit);
        if let Some(found) = best {
            child.rdepth = found;
        }
    }

    /// Elephant-foot growth toward the recommended minimum radius near the bed.
    fn grow_foot(&self, child: &mut Node, below: usize, valid: &impl Fn(i64, i64) -> bool) {
        let derived = &self.stage.derived;
        if !derived.foot_in_descent || derived.foot_increment_units <= 0 {
            return;
        }
        let real = child.real_radius(derived);
        let wanted = derived.recommended_min_radius(below as i64) - real;
        let planned = (wanted as f64 / derived.foot_increment_units as f64).min(1.0);
        let ready = planned > 0.0
            && child.to_bed
            && real >= derived.branch_radius
            && real >= derived.increase_until_radius;
        if ready {
            let radius = child.collision_radius(derived);
            if valid(derived.radius(child.rdepth, child.foot + planned), radius) {
                child.foot += planned;
            }
        }
    }

    /// Spec 02 §3.7: what a valid setting leaves behind.
    fn finish(&self, mut child: Node, setting: &Setting, reachable: &Rings, plate: Rings) -> Moved {
        let stage = self.stage;
        let derived = &stage.derived;
        let bypass = !setting.moves || (setting.tight_xy && child.depth < derived.tip_layers);
        if setting.moves {
            child.hold_until = 0;
        } else {
            child.centre = self.parent.centre;
        }
        child.past_holes = setting.avoidance != Avoidance::Fast;
        if !setting.tight_xy {
            child.tight_xy = false;
        }
        let radius = child.collision_radius(derived);
        let collision = stage
            .clearance
            .collision(radius, self.layer - 1, child.tight_xy);
        let stored = union(&difference(reachable, &collision), &plate);
        let pad = derived.grid.scaled(BOX_PADDING_MM) as i32;
        let bounds = (!bypass)
            .then(|| bbox(&stored))
            .flatten()
            .map(|b| padded(b, pad));
        child.area = stored;
        Moved {
            node: child,
            bypass,
            plate: if bypass { Rings::new() } else { plate },
            bounds,
        }
    }
}

fn padded(found: IntBox, pad: i32) -> IntBox {
    IntBox {
        min: IntPoint::new(found.min.x - pad, found.min.y - pad),
        max: IntPoint::new(found.max.x + pad, found.max.y + pad),
    }
}

/// The area plus every ring boundary as a thin ribbon, so a line-like area stays growable.
fn with_ribbons(rings: &Rings, grid: &Grid) -> Rings {
    let half = grid.scaled(RIBBON_HALF_WIDTH_MM);
    let mut all = rings.clone();
    for ring in rings {
        for index in 0..ring.len() {
            let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
            all.push(ribbon(a, b, half));
        }
    }
    union(&all, &Rings::new())
}

fn ribbon(a: IntPoint, b: IntPoint, half: f64) -> Vec<IntPoint> {
    let (dx, dy) = (
        f64::from(b.x) - f64::from(a.x),
        f64::from(b.y) - f64::from(a.y),
    );
    let length = libm::sqrt(dx * dx + dy * dy).max(1.0);
    let (nx, ny) = (-dy / length * half, dx / length * half);
    let corner = |p: IntPoint, sx: f64, sy: f64| {
        IntPoint::new((f64::from(p.x) + sx) as i32, (f64::from(p.y) + sy) as i32)
    };
    vec![
        corner(a, nx, ny),
        corner(b, nx, ny),
        corner(b, -nx, -ny),
        corner(a, -nx, -ny),
    ]
}

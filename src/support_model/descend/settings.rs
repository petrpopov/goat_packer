//! The ordered list of ways a branch may step down one layer (spec 02 §3.5). The first valid
//! one wins, so the order is the behaviour: stay put if that is still safe, otherwise prefer
//! the slow angle, then the fast one; a branch above a hole it could be trapped in first
//! tests the unmoved area against the safe avoidance.

use crate::support_model::clearance::Avoidance;
use crate::support_model::node::Node;
use crate::support_model::settings::Derived;

/// How far the influence area is allowed to grow for a setting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Speed {
    /// Not moving: the unmoved area is tested.
    Still,
    Slow,
    Fast,
}

/// One way of stepping down.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Setting {
    pub avoidance: Avoidance,
    pub speed: Speed,
    /// The radius may grow by one layer's worth.
    pub grow_radius: bool,
    /// The last resort: a longer, unchecked jump.
    pub error: bool,
    /// Use the reduced xy distance.
    pub tight_xy: bool,
    /// The area grows by the speed; otherwise the unmoved area is the candidate.
    pub moves: bool,
}

const fn setting(avoidance: Avoidance, speed: Speed, grow_radius: bool, moves: bool) -> Setting {
    Setting {
        avoidance,
        speed,
        grow_radius,
        error: false,
        tight_xy: false,
        moves,
    }
}

/// The settings of `parent` stepping down, in trial order.
pub fn settings_of(derived: &Derived, parent: &Node) -> Vec<Setting> {
    let depth = parent.depth + 1;
    let mut base = Vec::new();
    if parent.past_holes {
        base.push(setting(Avoidance::Slow, Speed::Slow, true, true));
        base.push(setting(Avoidance::Slow, Speed::Slow, false, true));
        if depth < derived.tip_layers {
            base.push(setting(Avoidance::FastSafe, Speed::Slow, true, true));
        }
        base.push(setting(Avoidance::FastSafe, Speed::Fast, true, true));
        base.push(setting(Avoidance::FastSafe, Speed::Fast, false, true));
    } else {
        base.push(setting(Avoidance::Slow, Speed::Slow, true, false));
        if depth < (derived.tip_layers + 1) / 2 {
            base.push(setting(Avoidance::Fast, Speed::Slow, true, false));
        }
        base.push(setting(Avoidance::FastSafe, Speed::Fast, true, false));
        base.push(setting(Avoidance::FastSafe, Speed::Fast, false, true));
        base.push(setting(Avoidance::Fast, Speed::Fast, false, true));
    }
    let mut list: Vec<Setting> = Vec::new();
    for regular in base {
        push_new(&mut list, regular);
        if parent.tight_xy {
            push_new(
                &mut list,
                Setting {
                    tight_xy: true,
                    ..regular
                },
            );
        }
    }
    if parent.to_bed {
        let mut last = setting(Avoidance::Fast, Speed::Fast, false, true);
        last.error = true;
        last.tight_xy = parent.tight_xy;
        push_new(&mut list, last);
    }
    if depth < parent.hold_until && parent.past_holes {
        list.insert(0, setting(Avoidance::Slow, Speed::Still, true, false));
    }
    list
}

fn push_new(list: &mut Vec<Setting>, candidate: Setting) {
    if !list.contains(&candidate) {
        list.push(candidate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::polygon_ops::IntPoint;
    use crate::support_model::flavour::Flavour;
    use crate::support_model::settings::tests_support::settings;

    fn derived_for(flavour: Flavour) -> Derived {
        Derived::new(&settings(), flavour)
    }

    fn node(past_holes: bool, tight: bool, depth: i64, hold: i64) -> Node {
        Node {
            layer: 10,
            depth,
            rdepth: depth,
            foot: 0.0,
            origin_layer: 20,
            origin_xy: IntPoint::new(0, 0),
            hint: IntPoint::new(0, 0),
            centre: None,
            to_bed: true,
            graceful: false,
            tight_xy: tight,
            under_roof: false,
            past_holes,
            keep_round: false,
            hold_until: hold,
            model_growth: 0,
            parents: Vec::new(),
            area: Vec::new(),
            deleted: false,
            marked: false,
        }
    }

    #[test]
    fn a_branch_past_holes_tries_slow_before_fast_and_ends_with_the_error_jump() {
        let derived = derived_for(Flavour::Orca242);
        let list = settings_of(&derived, &node(true, false, 6, 0));
        assert_eq!(list.first().unwrap().avoidance, Avoidance::Slow);
        assert!(
            list.iter()
                .any(|s| s.avoidance == Avoidance::FastSafe && s.speed == Speed::Fast)
        );
        let last = list.last().unwrap();
        assert!(last.error && last.moves);
    }

    #[test]
    fn a_tip_column_holds_still_first_and_tries_each_setting_in_both_xy_modes() {
        let derived = derived_for(Flavour::Bambu0208);
        let list = settings_of(&derived, &node(true, true, 0, 3));
        assert_eq!(list[0].speed, Speed::Still);
        let slow: Vec<_> = list
            .iter()
            .filter(|s| s.speed == Speed::Slow && s.grow_radius)
            .collect();
        assert!(slow.iter().any(|s| s.tight_xy) && slow.iter().any(|s| !s.tight_xy));
    }

    #[test]
    fn above_a_hole_the_first_entries_do_not_move() {
        let derived = derived_for(Flavour::Orca242);
        let list = settings_of(&derived, &node(false, false, 0, 0));
        assert!(!list[0].moves && !list[1].moves);
        assert_eq!(list[1].avoidance, Avoidance::Fast);
    }
}

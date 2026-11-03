//! The stepped safe offset (spec 02 §3.4): grow an area by `distance` through free space
//! without jumping across a wall set in one step. Growth happens in equal round steps no
//! longer than the safe step, the walls are subtracted after each, and a final wall
//! subtraction applies when the caller asked for a hard result.

use crate::polygon_ops::{Grid, Join, Rings, difference, offset, simplify};

/// Round-join arc tolerance of every step, mm.
const ARC_TOLERANCE_MM: f64 = 0.01;

/// Douglas-Peucker tolerance of the intermediate and final simplification, mm.
const SIMPLIFY_MM: f64 = 0.015;

/// A simplification runs after every ... step starting with the 8th.
const SIMPLIFY_EVERY: i64 = 10;
const SIMPLIFY_FIRST: i64 = 8;

/// A remainder shorter than this is not grown (mm).
const NEGLIGIBLE_MM: f64 = 0.0001;

/// What `grow_through` needs besides the area.
pub struct StepParams {
    pub grid: Grid,
    pub strict: bool,
    /// The largest step, grid units.
    pub safe_step: i64,
    /// Walls closer than this are only subtracted at the end (the caller's `c`).
    pub careless: i64,
    /// Number of parts the growth is cut into when it is shorter than the safe step.
    pub parts: i64,
}

/// `area` grown by `distance` units around `walls`.
pub fn grow_through(area: &Rings, distance: i64, walls: &Rings, params: &StepParams) -> Rings {
    let grid = params.grid;
    if distance <= 0 {
        return if params.careless == 0 {
            difference(area, walls)
        } else {
            area.clone()
        };
    }
    let plan = plan(distance, params);
    let round = Join::Round(grid.scaled(ARC_TOLERANCE_MM));
    let mut grown = area.clone();
    for step in 1..=plan.steps {
        grown = difference(&offset(&grown, plan.length as f64, round), walls);
        if step >= SIMPLIFY_FIRST && (step - SIMPLIFY_FIRST) % SIMPLIFY_EVERY == 0 {
            grown = simplify(&grown, grid.scaled(SIMPLIFY_MM), params.strict);
        }
    }
    let rest = distance - plan.steps * plan.length;
    if rest as f64 > grid.scaled(NEGLIGIBLE_MM) {
        grown = offset(&grown, rest as f64, round);
    }
    grown = simplify(&grown, grid.scaled(SIMPLIFY_MM), params.strict);
    if plan.finish {
        grown = difference(&grown, walls);
    }
    grown
}

struct Plan {
    steps: i64,
    length: i64,
    finish: bool,
}

/// Step count and length (spec 02 §3.4 maths).
fn plan(distance: i64, params: &StepParams) -> Plan {
    let safe = params.safe_step.max(1);
    let careless = params.careless;
    let whole = (distance - careless).max(0) / safe;
    let more = i64::from(distance - whole * safe > careless && (whole + 1) * safe <= distance);
    let first_steps = whole + more;
    let finish =
        careless == 0 || (distance - whole * safe > careless && (whole + 1) * safe > distance);
    let remainder = i64::from(distance < careless || distance % safe != 0);
    if params.parts > 1 && first_steps + remainder < params.parts {
        let share = distance / params.parts;
        let length = share.min(safe);
        let steps = if share >= safe {
            params.parts
        } else if length > 0 {
            distance / length
        } else {
            0
        };
        return Plan {
            steps,
            length,
            finish,
        };
    }
    Plan {
        steps: first_steps,
        length: safe,
        finish,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::polygon_ops::{IntPoint, Ring, area};

    fn square(half: i32) -> Rings {
        vec![vec![
            IntPoint::new(-half, -half),
            IntPoint::new(half, -half),
            IntPoint::new(half, half),
            IntPoint::new(-half, half),
        ]]
    }

    fn params(careless: i64, parts: i64) -> StepParams {
        StepParams {
            grid: Grid::Nanometre,
            strict: false,
            safe_step: 25_000_000,
            careless,
            parts,
        }
    }

    #[test]
    fn a_short_slow_growth_is_two_half_steps_and_a_final_wall_cut() {
        let plan = plan(131_000, &params(0, 2));
        assert_eq!((plan.steps, plan.length, plan.finish), (2, 65_500, true));
    }

    #[test]
    fn a_far_wall_distance_means_one_unchecked_offset() {
        let plan = plan(150_000, &params(25_500_000, 1));
        assert_eq!((plan.steps, plan.finish), (0, false));
    }

    #[test]
    fn walls_are_subtracted_at_the_end() {
        let wall: Rings = vec![vec![
            IntPoint::new(150_000, -1_000_000),
            IntPoint::new(1_000_000, -1_000_000),
            IntPoint::new(1_000_000, 1_000_000),
            IntPoint::new(150_000, 1_000_000),
        ]];
        let grown = grow_through(&square(100_000), 131_000, &wall, &params(0, 2));
        let reference: Ring = square(100_000)[0].clone();
        assert!(area(&grown) > area(&[reference]));
        assert!(area(&grown) < area(&square(100_000 + 131_000)));
    }
}

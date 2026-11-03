//! The exact refinement on squares: a 1 mm raster rounds a 9.3 mm square up to a 10 mm pitch,
//! the continuous lattice goes down to 9.3, and the plate is cut so that the finer pitch fits
//! one more column and row than the raster pitch does.

use super::lattice::{Contacts, PHASE, Params, det};
use super::oracle::Oracle;
use super::search::search;
use super::sites::Field;
use crate::geometry::{Point, Rect};
use crate::object_print::ObjectPrint;
use crate::pack::bound::tests::{options, plate};
use crate::pack::select::Member;
use crate::pack::{PackOutcome, pack_members};
use crate::validation::{LayoutRules, VectorValidator};

const SIDE: f64 = 9.3;
/// Seven columns and five rows of 9.3 mm squares fit; at a 10 mm pitch only six by four do.
const WIDTH: f64 = 7.0 * SIDE + 0.01;
const HEIGHT: f64 = 5.0 * SIDE + 0.01;

fn ring(points: &[(f64, f64)]) -> Vec<Point> {
    let mut ring: Vec<Point> = points.iter().map(|&(x, y)| Point { x, y }).collect();
    ring.push(ring[0]);
    ring
}

fn square() -> ObjectPrint {
    let h = SIDE / 2.0;
    let shape = ring(&[(-h, -h), (h, -h), (h, h), (-h, h)]);
    ObjectPrint {
        silhouette: vec![shape.clone()],
        part_hulls: vec![shape.clone()],
        part_boxes: vec![Rect::of_points(shape.iter())],
        landing: vec![shape.clone()],
        support_base: vec![shape.clone()],
        body: vec![vec![shape]],
        support: vec![vec![]],
        interface: vec![vec![]],
        ..ObjectPrint::default()
    }
}

fn pack(members: &[Member], effort: usize) -> PackOutcome {
    let rules = plate(WIDTH, HEIGHT);
    let options = crate::options::PackOptions {
        attempts: effort,
        ..options(&rules)
    };
    pack_members(&[square()], &rules, None, &options, members).expect("packs")
}

fn validator() -> VectorValidator {
    let rules = LayoutRules::new(&plate(WIDTH, HEIGHT), None, 0.0, Default::default(), None)
        .expect("rules");
    VectorValidator::new(&[square()], &rules).expect("validator")
}

/// Box of the origins the square may take.
fn origins() -> Rect {
    let h = SIDE / 2.0;
    Rect::new(h, h, WIDTH - h, HEIGHT - h)
}

fn raster_lattice() -> Params {
    [10.0, 0.0, 0.0, 10.0, 0.0, 0.0, SIDE / 2.0, SIDE / 2.0]
}

#[test]
fn contacts_reject_overlap_and_accept_the_pitch() {
    let validator = validator();
    let oracle = Oracle::new(&validator, 1);
    let mut contacts = Contacts::new(&[0]);
    assert!(contacts.admissible(&oracle, &raster_lattice()));
    let mut tight = raster_lattice();
    tight[0] = SIDE + 1e-4;
    tight[3] = SIDE + 1e-4;
    assert!(contacts.admissible(&oracle, &tight));
    tight[0] = SIDE - 0.05;
    assert!(!contacts.admissible(&oracle, &tight));
}

#[test]
fn the_search_goes_below_the_raster_cell_and_fits_more_copies() {
    let validator = validator();
    let oracle = Oracle::new(&validator, 1);
    let regions = [origins()];
    let field = Field {
        oracle: &oracle,
        classes: &[0],
        regions: &regions,
    };
    let start = raster_lattice();
    let before = field.sites(&start).usable.len();
    let mut contacts = Contacts::new(&[0]);
    let (found, score) = search(&field, &mut contacts, start, 4);
    assert_eq!(before, 24);
    assert!(det(&found) < det(&start) - 1.0, "cell {}", det(&found));
    assert!(score.count > before, "{} copies", score.count);
    assert_eq!(score.count, 35);
    assert!(contacts.admissible(&oracle, &found));
}

#[test]
fn the_search_never_loses_sites_and_is_repeatable() {
    let validator = validator();
    let oracle = Oracle::new(&validator, 1);
    let regions = [origins()];
    let field = Field {
        oracle: &oracle,
        classes: &[0],
        regions: &regions,
    };
    let mut start = raster_lattice();
    start[PHASE] += 0.7;
    let run = || search(&field, &mut Contacts::new(&[0]), start, 4);
    let (first, score) = run();
    let (second, again) = run();
    assert!(score.count >= field.sites(&start).usable.len());
    assert_eq!(first.map(f64::to_bits), second.map(f64::to_bits));
    assert_eq!(score.count, again.count);
}

#[test]
fn the_member_publishes_more_copies_than_the_raster_lattice() {
    let raster = pack(&[Member::Lattice], 16);
    let exact = pack(&[Member::Lattice, Member::Exact], 16);
    assert_eq!(raster.placements.len(), 24);
    assert_eq!(exact.member, Member::Exact);
    // A skewed raster seed morphs less freely than the handcrafted square start above.
    assert!(
        exact.placements.len() >= 30,
        "{} copies",
        exact.placements.len()
    );
    // The published layout passes the exact gate, checked here on its own.
    validator().check(&exact.placements).expect("vector valid");
}

#[test]
fn the_member_never_returns_fewer_copies() {
    for effort in [1, 8, 64] {
        let raster = pack(&[Member::Lattice], effort).placements.len();
        let exact = pack(&[Member::Lattice, Member::Exact], effort)
            .placements
            .len();
        assert!(exact >= raster, "effort {effort}: {exact} < {raster}");
    }
}

#[test]
fn more_effort_never_loses_copies() {
    let counts: Vec<usize> = [1, 4, 16, 64]
        .into_iter()
        .map(|effort| {
            pack(&[Member::Lattice, Member::Exact], effort)
                .placements
                .len()
        })
        .collect();
    assert!(counts.windows(2).all(|w| w[0] <= w[1]), "{counts:?}");
}

#[test]
fn the_layout_does_not_depend_on_the_thread_count() {
    let run = |threads| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .expect("pool");
        let outcome = pool.install(|| pack(&[Member::Lattice, Member::Exact], 64));
        outcome
            .placements
            .iter()
            .map(|p| (p.print, p.x_mm.to_bits(), p.y_mm.to_bits()))
            .collect::<Vec<_>>()
    };
    assert_eq!(run(1), run(8));
}

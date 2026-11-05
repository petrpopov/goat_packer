//! Support model stages B and S4 on generated data only: the branch graph of a cantilever
//! (tips under the arm, merging into trunks, roots on the bed), the realization (flange wider
//! than the roots, bands under the band rule), the exact thread-class walk, flavour switches,
//! the unsupported settings, the cache and determinism across thread counts.

use goat_packer::embedded_mesh::{ObjectMesh, Part};
use goat_packer::object_print::{Loops, Margins, Realization};
use goat_packer::support_model::bands::realization;
use goat_packer::support_model::cache::Ensemble;
use goat_packer::support_model::ensemble::{class_of, distinct, run_members};
use goat_packer::support_model::envelope::envelope;
use goat_packer::support_model::exceedance::area_mm2;
use goat_packer::support_model::flavour::Flavour;
use goat_packer::support_model::graph::branch_graph;
use goat_packer::support_model::merge::{MergeOrder, Overlap, ThreadClass};
use goat_packer::support_model::predict::{PredictInput, predict};
use goat_packer::support_model::settings::{ProjectConfig, SupportSettings};
use goat_packer::support_model::tips::ArcFamily;
use goat_packer::support_model::{StageAInput, SupportError, stage_a};

const BOTH: [MergeOrder; 2] = [MergeOrder::Along, MergeOrder::Against];

const COMMON: &str = r#""layer_height": "0.28", "min_layer_height": ["0.08"],
    "slice_closing_radius": "0.049", "resolution": "0.03", "slicing_mode": "regular",
    "elefant_foot_compensation": "0.15", "nozzle_diameter": ["0.4"], "line_width": "0.42",
    "support_line_width": "0.42", "outer_wall_line_width": "0.42", "enable_support": "1",
    "support_type": "tree(auto)", "support_style": "default",
    "support_critical_regions_only": "1", "support_remove_small_overhang": "1",
    "tree_support_branch_angle": "45", "tree_support_branch_distance": "5",
    "tree_support_branch_diameter": "2", "tree_support_branch_diameter_angle": "5",
    "tree_support_branch_angle_organic": "40", "tree_support_branch_distance_organic": "1",
    "tree_support_branch_diameter_organic": "2", "tree_support_angle_slow": "25",
    "tree_support_tip_diameter": "0.8", "tree_support_top_rate": "30%",
    "support_object_xy_distance": "0.75", "support_on_build_plate_only": "1",
    "support_interface_top_layers": "3", "support_interface_bottom_layers": "3",
    "support_interface_spacing": "0.5", "support_base_pattern": "rectilinear",
    "support_base_pattern_spacing": "2.5", "min_feature_size": "25%",
    "tree_support_wall_count": "0", "support_threshold_angle": "30",
    "support_object_first_layer_gap": "0.75", "support_top_z_distance": "0.24",
    "support_bottom_z_distance": "0.24", "max_bridge_length": "10",
    "initial_layer_print_height": "0.25", "initial_layer_line_width": "0.5",
    "raft_first_layer_expansion": "2""#;

fn settings(extra: &str) -> SupportSettings {
    let json = format!("{{{COMMON}{extra}}}");
    SupportSettings::from_config(&ProjectConfig::from_json(&json).unwrap()).unwrap()
}

/// Axis-aligned box; triangles wound outward.
fn cuboid(min: [f64; 3], size: [f64; 3]) -> Part {
    let vertices = (0..8)
        .map(|bits| {
            [0, 1, 2].map(|axis| {
                min[axis]
                    + if bits >> axis & 1 == 1 {
                        size[axis]
                    } else {
                        0.0
                    }
            })
        })
        .collect();
    let triangles = vec![
        [0, 2, 3],
        [0, 3, 1],
        [4, 5, 7],
        [4, 7, 6],
        [0, 1, 5],
        [0, 5, 4],
        [2, 6, 7],
        [2, 7, 3],
        [0, 4, 6],
        [0, 6, 2],
        [1, 3, 7],
        [1, 7, 5],
    ];
    Part {
        vertices,
        triangles,
        ..Part::default()
    }
}

/// A post with an arm: the arm's underside is a 24 mm cantilever 10 mm above the bed.
fn cantilever() -> ObjectMesh {
    let post = cuboid([0.0, 0.0, 0.0], [10.0, 10.0, 14.0]);
    let arm = cuboid([10.0, 0.0, 10.0], [24.0, 10.0, 4.0]);
    let mut merged = Part::default();
    for part in [post, arm] {
        let base = merged.vertices.len() as u32;
        merged.vertices.extend(part.vertices);
        merged
            .triangles
            .extend(part.triangles.iter().map(|t| t.map(|v| v + base)));
    }
    ObjectMesh::from_plate_parts(vec![merged]).unwrap()
}

fn model(flavour: Flavour) -> (goat_packer::support_model::StageA, SupportSettings) {
    let settings = settings("");
    let stage = stage_a(&StageAInput {
        mesh: &cantilever(),
        settings: &settings,
        flavour,
        bed: None,
    })
    .unwrap();
    (stage, settings)
}

fn same_ignoring_member(a: &Realization, b: &Realization) -> bool {
    Realization {
        member: 0,
        ..a.clone()
    } == Realization {
        member: 0,
        ..b.clone()
    }
}

#[test]
fn a_cantilever_gets_tips_trunks_and_a_flange_wider_than_its_roots() {
    let (stage, _) = model(Flavour::Orca242);
    let graph = branch_graph(&stage, ArcFamily::PerCall, ThreadClass::along(1));
    let tips: usize = graph
        .layers
        .iter()
        .map(|layer| layer.iter().filter(|n| n.parents.is_empty()).count())
        .sum();
    assert!(tips >= 10, "{tips} tips under a 24 mm arm");
    let roots = &graph.layers[0];
    assert!(
        !roots.is_empty() && roots.len() < tips,
        "tips merge into trunks"
    );
    // Every surviving node has a centre; a root's radius is the foot radius at most.
    assert!(graph.layers.iter().flatten().all(|n| n.centre.is_some()));
    let derived = &stage.derived;
    let widest = roots.iter().map(|n| n.real_radius(derived)).max().unwrap();
    assert!(widest <= derived.foot_radius + derived.grid.length(0.01));

    let print = realization(&stage, &graph, 0);
    let roots_area: f64 = roots
        .iter()
        .map(|n| {
            let r = derived.grid.mm(n.real_radius(derived) as f64);
            std::f64::consts::PI * r * r
        })
        .sum();
    assert!(
        area_mm2(&print.base) > roots_area,
        "the flange grows the base past the roots: {} vs {roots_area}",
        area_mm2(&print.base)
    );
    // The first layer is in band 0 as well; supports exist up to the roof.
    assert!(print.support.len() >= 5 && !print.support[0].is_empty());
    assert!(print.above_base[0].len() <= print.support[0].len() + 1);
    assert!(print.interface.iter().any(|band| !band.is_empty()));
}

#[test]
fn roots_stay_outside_the_object_and_the_flange_never_crosses_the_gap() {
    let (stage, _) = model(Flavour::Orca242);
    let graph = branch_graph(&stage, ArcFamily::PerCall, ThreadClass::along(1));
    let print = realization(&stage, &graph, 0);
    // The post stands at x, y in [-5, 5] (frame centred on the object's box, which is wider).
    // First-layer gap 0.75 mm to the first layer of the post: no base material inside it.
    let first = &stage.sections.area[0];
    let mask = goat_packer::polygon_ops::offset(
        first,
        stage.derived.grid.scaled(stage.derived.first_layer_gap_mm) * 0.99,
        goat_packer::polygon_ops::Join::Square,
    );
    let base = stage.derived.grid.rings(&print.base);
    let shifted: Vec<_> = base
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|p| {
                    let (dx, dy) = (
                        stage.derived.grid.scaled(stage.frame_shift[0]) as i32,
                        stage.derived.grid.scaled(stage.frame_shift[1]) as i32,
                    );
                    goat_packer::polygon_ops::IntPoint::new(p.x - dx, p.y - dy)
                })
                .collect()
        })
        .collect();
    let inside = goat_packer::polygon_ops::intersection(&shifted, &mask);
    assert!(
        stage
            .derived
            .grid
            .mm2(goat_packer::polygon_ops::area(&inside))
            < 1.0,
        "the base keeps the gap from the model's first layer"
    );
}

#[test]
fn flavour_switches_change_the_roots_and_the_bucket_rule() {
    let (orca, _) = model(Flavour::Orca242);
    let (snap, _) = model(Flavour::Snapmaker240);
    let flat = realization(
        &orca,
        &branch_graph(&orca, ArcFamily::PerCall, ThreadClass::along(1)),
        0,
    );
    let round = realization(
        &snap,
        &branch_graph(&snap, ArcFamily::PerCall, ThreadClass::along(1)),
        0,
    );
    // A flat foot slices to the full radius, a hemisphere to a hair less at mid-layer.
    assert!(area_mm2(&flat.base) >= area_mm2(&round.base) - 1e-6);
    // R pins the bucket rule: every thread count is one class (one member per overlap
    // reading); O walks the classes.
    let (r_stage, _) = model(Flavour::Orca250);
    let r_runs = run_members(&r_stage, &BOTH);
    assert!(
        r_runs
            .iter()
            .all(|run| run.class.1.is_none() || run.threads == 1)
    );
    assert_eq!(
        r_runs
            .iter()
            .filter(|run| run.order == MergeOrder::Along)
            .count(),
        Overlap::ALL.len()
    );
    let o_runs = run_members(&orca, &BOTH);
    assert!(o_runs.len() >= 2);
}

#[test]
fn the_thread_classes_tile_every_thread_count_and_a_class_repeats_its_run() {
    let (stage, _) = model(Flavour::Orca242);
    let all = run_members(&stage, &BOTH);
    for (family, overlap) in [ArcFamily::Roof, ArcFamily::Base]
        .into_iter()
        .flat_map(|family| {
            stage
                .flavour
                .overlap_readings()
                .iter()
                .map(move |&overlap| (family, overlap))
        })
    {
        let runs: Vec<_> = all
            .iter()
            .filter(|run| {
                run.family == family && run.order == MergeOrder::Along && run.overlap == overlap
            })
            .collect();
        assert!(!runs.is_empty(), "{family:?} {overlap:?} has members");
        let mut covered = 1;
        for run in runs {
            assert_eq!(run.threads, covered, "no gap between classes: {family:?} {overlap:?}");
            match run.class.1 {
                Some(upper) => covered = upper + 1,
                None => covered = usize::MAX,
            }
            // Another thread count of the class gives the same realization.
            let mid = run
                .class
                .1
                .map_or(run.threads + 3, |upper| (run.threads + upper) / 2);
            let class = ThreadClass {
                overlap,
                ..ThreadClass::along(mid)
            };
            let graph = branch_graph(&stage, run.family, class);
            assert_eq!(class_of(&graph.merge_values, mid).1, run.class.1);
            if let Some(drawn) = &run.realization {
                assert!(same_ignoring_member(&realization(&stage, &graph, 0), drawn));
            }
        }
        assert_eq!(covered, usize::MAX, "the last class is unbounded");
    }
}

#[test]
fn unsupported_settings_are_named_before_any_geometry() {
    let raft = settings(r#", "raft_layers": "2""#);
    let error = predict(&PredictInput {
        mesh: &cantilever(),
        settings: &raft,
        flavour: Flavour::Orca242,
        rotation_deg: 0.0,
        cache_dir: None,
        orders: &BOTH,
    })
    .err();
    assert_eq!(
        error.as_ref().and_then(SupportError::setting),
        Some("raft_layers")
    );
}

#[test]
fn a_prediction_is_cached_and_the_envelope_grows_by_the_margins() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings("");
    let input = PredictInput {
        mesh: &cantilever(),
        settings: &settings,
        flavour: Flavour::Orca250,
        rotation_deg: 0.0,
        cache_dir: Some(dir.path()),
        orders: &BOTH,
    };
    let first = predict(&input).unwrap();
    let second = predict(&input).unwrap();
    assert!(!first.cached && second.cached);
    assert_eq!(first.ensemble, second.ensemble);
    let members: Vec<&Realization> = first.ensemble.realizations.iter().collect();
    let plain = envelope(&members, Margins::default());
    let grown = envelope(
        &members,
        Margins {
            first_layer_mm: 1.0,
            band_mm: 2.0,
        },
    );
    assert!(area_mm2(&grown.base) > area_mm2(&plain.base) + 10.0);
    assert!(area_mm2(&grown.above_base[2]) > area_mm2(&plain.above_base[2]));
    // Another rotation is another key.
    let turned = predict(&PredictInput {
        rotation_deg: 90.0,
        ..input
    })
    .unwrap();
    assert!(!turned.cached && turned.cache_key != first.cache_key);
}

fn ensemble_of(threads: usize) -> Ensemble {
    let settings = settings("");
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    pool.install(|| {
        predict(&PredictInput {
            mesh: &cantilever(),
            settings: &settings,
            flavour: Flavour::Bambu0208,
            rotation_deg: 0.0,
            cache_dir: None,
            orders: &BOTH,
        })
        .unwrap()
        .ensemble
    })
}

#[test]
fn the_ensemble_is_identical_for_one_and_eight_threads_and_across_runs() {
    let one = ensemble_of(1);
    let eight = ensemble_of(8);
    assert_eq!(one, eight);
    assert_eq!(one, ensemble_of(3));
    assert!(!one.realizations.is_empty());
    assert!(one.members.len() >= one.realizations.len());
}

#[test]
fn every_distinct_graph_has_exactly_one_realization() {
    let (stage, _) = model(Flavour::Orca242);
    let runs = run_members(&stage, &BOTH);
    let unique = distinct(&runs);
    let mut hashes: Vec<u64> = runs.iter().map(|run| run.hash).collect();
    hashes.sort_unstable();
    hashes.dedup();
    assert_eq!(unique.len(), hashes.len());
    // Exactly the first member of each graph... or a later one drew it: one drawer each.
    let drawn = runs.iter().filter(|run| run.realization.is_some()).count();
    assert_eq!(drawn, hashes.len());
}

#[test]
fn loops_of_a_member_are_closed_millimetre_rings() {
    let (stage, _) = model(Flavour::Orca242);
    let graph = branch_graph(&stage, ArcFamily::PerCall, ThreadClass::along(1));
    let print = realization(&stage, &graph, 0);
    let all: Vec<&Loops> = std::iter::once(&print.base)
        .chain(print.support.iter())
        .collect();
    for loops in all {
        for ring in loops {
            assert!(ring.len() >= 4 && ring.first() == ring.last());
        }
    }
}

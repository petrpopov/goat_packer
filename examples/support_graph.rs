//! Real-data check of support-model stage B1-B4 (tips, descent, merge, placement) on a
//! project: `support_graph <3mf> [rotation_deg] [threads]`. Prints timing, tips, nodes per
//! layer and the roots (layer 0) with their radii in the plate-aligned frame.
use goat_packer::embedded_mesh::load_object_mesh;
use goat_packer::polygon_ops::distance_to_boundary;
use goat_packer::support_model::flavour::Flavour;
use goat_packer::support_model::graph::branch_graph;
use goat_packer::support_model::merge::ThreadClass;
use goat_packer::support_model::settings::{ProjectConfig, SupportSettings};
use goat_packer::support_model::smooth::smooth;
use goat_packer::support_model::tips::{ArcFamily, seed_tips};
use goat_packer::support_model::{StageAInput, stage_a};
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = Path::new(&args[0]);
    let rotation: f64 = args.get(1).map_or(0.0, |text| text.parse().unwrap());
    let threads: usize = args.get(2).map_or(1, |text| text.parse().unwrap());
    let tips_out = args.get(5).cloned();
    let trace: Option<(f64, f64)> = args
        .get(3)
        .zip(args.get(4))
        .map(|(x, y)| (x.parse().unwrap(), y.parse().unwrap()));
    let loaded = load_object_mesh(path).unwrap();
    let mesh = loaded.rotated(rotation);
    let config = ProjectConfig::load(path).unwrap();
    let settings = SupportSettings::from_config(&config).unwrap();
    let flavour = Flavour::resolve(None, config.application.as_deref(), None).unwrap();
    let start = Instant::now();
    let stage = stage_a(&StageAInput {
        mesh: &mesh,
        settings: &settings,
        flavour: flavour.flavour,
        bed: None,
    })
    .unwrap();
    println!("stage A {:?}", start.elapsed());
    let family = if flavour.flavour.static_arc_thresholds() {
        ArcFamily::Base
    } else {
        ArcFamily::PerCall
    };
    let start = Instant::now();
    let seeds = seed_tips(&stage, family);
    let tips: usize = seeds.tips.iter().map(Vec::len).sum();
    println!(
        "tips {tips} on {} layers, roofs {} ({:?})",
        seeds.tips.iter().filter(|l| !l.is_empty()).count(),
        seeds.roofs.len(),
        start.elapsed()
    );
    let start = Instant::now();
    let graph = branch_graph(&stage, family, ThreadClass::along(threads));
    println!(
        "graph {:?} (tips, moves, merges, placement: {:.2?})",
        start.elapsed(),
        graph.seconds
    );
    let counts: Vec<usize> = graph.layers.iter().map(Vec::len).collect();
    println!("nodes per layer {counts:?}");
    println!("merge values {:?}", graph.merge_values);
    let derived = &stage.derived;
    let units = derived.grid;
    let start = Instant::now();
    let smoothed = smooth(&stage, &graph.layers);
    println!(
        "smoothing {:?}, {} iterations",
        start.elapsed(),
        smoothed.iterations
    );
    let body = &stage.sections.area[0];
    for (index, node) in graph.layers[0].iter().enumerate() {
        let centre = node.centre.unwrap();
        let moved = smoothed.centres[0][index];
        let at = |p: goat_packer::polygon_ops::IntPoint| {
            (
                units.mm(f64::from(p.x)) + stage.frame_shift[0],
                units.mm(f64::from(p.y)) + stage.frame_shift[1],
            )
        };
        println!(
            "root placed ({:.2}, {:.2}) smoothed ({:.2}, {:.2}) radius {:.2} body distance {:.2} -> {:.2} parents {}",
            at(centre).0,
            at(centre).1,
            at(moved).0,
            at(moved).1,
            units.mm(node.real_radius(derived) as f64),
            units.mm(distance_to_boundary(body, centre)),
            units.mm(distance_to_boundary(body, moved)),
            node.parents.len()
        );
    }
    if let Some((x, y)) = trace {
        let at = |p: goat_packer::polygon_ops::IntPoint| {
            (
                units.mm(f64::from(p.x)) + stage.frame_shift[0],
                units.mm(f64::from(p.y)) + stage.frame_shift[1],
            )
        };
        let mut index = (0..graph.layers[0].len())
            .min_by(|&a, &b| {
                let da = at(graph.layers[0][a].centre.unwrap());
                let db = at(graph.layers[0][b].centre.unwrap());
                let fa = (da.0 - x).hypot(da.1 - y);
                let fb = (db.0 - x).hypot(db.1 - y);
                fa.total_cmp(&fb)
            })
            .unwrap();
        for layer in 0..graph.layers.len() {
            let node = &graph.layers[layer][index];
            let (cx, cy) = at(node.centre.unwrap());
            println!(
                "L{layer:3} ({cx:7.2},{cy:7.2}) depth {:3} rdepth {:3} foot {:.2} c {:.2} rho {:.2} parents {} tight {} past {} hold {}",
                node.depth,
                node.rdepth,
                node.foot,
                units.mm(node.collision_radius(derived) as f64),
                units.mm(node.real_radius(derived) as f64),
                node.parents.len(),
                node.tight_xy,
                node.past_holes,
                node.hold_until
            );
            if layer <= 14 {
                let far = node
                    .area
                    .iter()
                    .flatten()
                    .map(|&p| units.mm(distance_to_boundary(body, p)))
                    .fold(0.0, f64::max);
                let near = node
                    .area
                    .iter()
                    .flatten()
                    .map(|&p| units.mm(distance_to_boundary(body, p)))
                    .fold(f64::MAX, f64::min);
                println!(
                    "      IA vertices {} body distance {near:.2}..{far:.2}",
                    node.area.iter().map(Vec::len).sum::<usize>()
                );
            }
            match node.parents.first() {
                Some(&parent) => index = parent,
                None => break,
            }
        }
    }
    if let Some(file) = tips_out {
        let mut tips = Vec::new();
        for (layer, nodes) in graph.layers.iter().enumerate() {
            for node in nodes.iter().filter(|node| node.parents.is_empty()) {
                let centre = node.centre.unwrap();
                tips.push(serde_json::json!({
                    "layer": layer,
                    "x": units.mm(f64::from(centre.x)) + stage.frame_shift[0],
                    "y": units.mm(f64::from(centre.y)) + stage.frame_shift[1],
                }));
            }
        }
        std::fs::write(file, serde_json::to_vec(&tips).unwrap()).unwrap();
    }
}

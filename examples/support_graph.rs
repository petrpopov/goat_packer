//! Real-data check of support-model stage B1-B4 (tips, descent, merge, placement) on a
//! project: `support_graph <3mf> [rotation_deg] [threads]`. Prints timing, tips, nodes per
//! layer and the roots (layer 0) with their radii in the plate-aligned frame.
use goat_packer::embedded_mesh::load_object_mesh;
use goat_packer::support_model::flavour::Flavour;
use goat_packer::support_model::graph::branch_graph;
use goat_packer::support_model::merge::ThreadClass;
use goat_packer::support_model::settings::{ProjectConfig, SupportSettings};
use goat_packer::support_model::tips::{ArcFamily, seed_tips};
use goat_packer::support_model::{StageAInput, stage_a};
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = Path::new(&args[0]);
    let rotation: f64 = args.get(1).map_or(0.0, |text| text.parse().unwrap());
    let threads: usize = args.get(2).map_or(1, |text| text.parse().unwrap());
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
    let graph = branch_graph(&stage, family, ThreadClass { threads });
    println!("graph {:?}", start.elapsed());
    let counts: Vec<usize> = graph.layers.iter().map(Vec::len).collect();
    println!("nodes per layer {counts:?}");
    println!("merge values {:?}", graph.merge_values);
    let derived = &stage.derived;
    let units = derived.grid;
    for node in &graph.layers[0] {
        let centre = node.centre.unwrap();
        println!(
            "root ({:.2}, {:.2}) radius {:.2} parents {}",
            units.mm(f64::from(centre.x)) + stage.frame_shift[0],
            units.mm(f64::from(centre.y)) + stage.frame_shift[1],
            units.mm(node.real_radius(derived) as f64),
            node.parents.len()
        );
    }
}

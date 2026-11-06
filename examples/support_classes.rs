//! Lists the thread classes of the support model's ensemble for a project:
//! `support_classes <3mf> [rotation_deg]`. Each line is one member: arc family, merge order, the
//! thread counts (a, b] that print it and whether its graph is new.
use goat_packer::embedded_mesh::load_object_mesh;
use goat_packer::support_model::ensemble::run_members;
use goat_packer::support_model::flavour::Flavour;
use goat_packer::support_model::merge::MergeOrder;
use goat_packer::support_model::settings::{ProjectConfig, SupportSettings};
use goat_packer::support_model::{StageAInput, stage_a};
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = Path::new(&args[0]);
    let rotation: f64 = args.get(1).map_or(0.0, |text| text.parse().unwrap());
    let mesh = load_object_mesh(path).unwrap().rotated(rotation);
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
    let members = run_members(&stage, &[MergeOrder::Along]);
    println!("{} members in {:.1?}", members.len(), start.elapsed());
    for member in &members {
        println!(
            "{:?} {:?} threads ({}, {}] graph {:016x}{}",
            member.family,
            member.order,
            member.class.0,
            member.class.1.map_or("inf".to_owned(), |end| end.to_string()),
            member.hash,
            if member.graph.is_some() { "" } else { " (same as an earlier member)" }
        );
    }
}

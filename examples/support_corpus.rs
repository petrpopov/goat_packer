//! Fidelity of the support model against sliced realizations (design §11, WP S4 loop):
//! `support_corpus <project.3mf> [--threads T] [--family roof|base|call] <plate.gcode>...`
//!
//! The project is a corpus project at its rotation (the item transform carries the rotation,
//! so the model runs at rotation 0 in the loaded frame); every G-code is a single-object slice
//! of it. Prints, for the member and for each G-code, the first-layer base and every band:
//! model area, slice area, slice area outside the model, and the largest distance outside.
use goat_packer::embedded_mesh::load_object_mesh;
use goat_packer::object_print::{Loops, Realization};
use goat_packer::realization_from_gcode;
use goat_packer::support_model::bands::realization;
use goat_packer::support_model::exceedance::{area_mm2, exceedance};
use goat_packer::support_model::flavour::Flavour;
use goat_packer::support_model::graph::branch_graph;
use goat_packer::support_model::merge::ThreadClass;
use goat_packer::support_model::settings::{ProjectConfig, SupportSettings};
use goat_packer::support_model::tips::ArcFamily;
use goat_packer::support_model::{StageAInput, stage_a};
use std::path::Path;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let project = args.next().expect("project.3mf");
    let mut threads = 14;
    let mut family = None;
    let mut gcodes = Vec::new();
    let mut dump = None;
    let mut printed_all = Vec::new();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--threads" => threads = args.next().unwrap().parse().unwrap(),
            "--family" => family = args.next(),
            "--dump" => dump = args.next(),
            other => gcodes.push(other.to_owned()),
        }
    }
    let path = Path::new(&project);
    let mesh = load_object_mesh(path).unwrap();
    let origin = (mesh.frame_origin[0], mesh.frame_origin[1]);
    let config = ProjectConfig::load(path).unwrap();
    let settings = SupportSettings::from_config(&config).unwrap();
    let flavour = Flavour::resolve(None, config.application.as_deref(), None)
        .unwrap()
        .flavour;
    let family = match family.as_deref() {
        Some("roof") => ArcFamily::Roof,
        Some("call") => ArcFamily::PerCall,
        Some("base") => ArcFamily::Base,
        _ if flavour.static_arc_thresholds() => ArcFamily::Base,
        _ => ArcFamily::PerCall,
    };
    let start = Instant::now();
    let stage = stage_a(&StageAInput {
        mesh: &mesh,
        settings: &settings,
        flavour,
        bed: None,
    })
    .unwrap();
    println!("stage A {:?}", start.elapsed());
    let start = Instant::now();
    let graph = branch_graph(&stage, family, ThreadClass { threads });
    let graph_time = start.elapsed();
    let start = Instant::now();
    let model = realization(&stage, &graph, 0);
    println!(
        "{flavour:?} family {family:?} T {threads}: graph {graph_time:?}, drawing {:?}, roots {}",
        start.elapsed(),
        graph.layers[0].len()
    );
    for gcode in &gcodes {
        let bytes = std::fs::read(gcode).unwrap();
        let printed = realization_from_gcode(&bytes, None, origin, 0).unwrap();
        println!("--- {gcode}");
        report(&model, &printed);
        printed_all.push(printed);
    }
    if let Some(file) = dump {
        let units = stage.derived.grid;
        let body: Vec<Vec<[f64; 2]>> = stage.sections.area[0]
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|p| {
                        [
                            units.mm(f64::from(p.x)) + stage.frame_shift[0],
                            units.mm(f64::from(p.y)) + stage.frame_shift[1],
                        ]
                    })
                    .collect()
            })
            .collect();
        let value = serde_json::json!({"model": model, "slices": printed_all, "body0": body});
        std::fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
    }
}

fn report(model: &Realization, printed: &Realization) {
    line("L1", &model.base, &printed.base);
    let bands = model.support.len().max(printed.support.len());
    let empty = Loops::new();
    let mut worst_band: f64 = 0.0;
    let mut outside = 0.0;
    for band in 0..bands {
        let ours = model.support.get(band).unwrap_or(&empty);
        let theirs = printed.support.get(band).unwrap_or(&empty);
        let miss = exceedance(theirs, ours);
        worst_band = worst_band.max(miss.max_mm);
        outside += miss.outside_mm2;
        line(&format!("b{band:02}"), ours, theirs);
    }
    println!("bands: worst exceedance {worst_band:.2} mm, outside area total {outside:.1} mm²");
}

fn line(name: &str, ours: &Loops, theirs: &Loops) {
    let miss = exceedance(theirs, ours);
    let extra = exceedance(ours, theirs);
    println!(
        "{name}: model {:7.1} slice {:7.1} | slice outside model {:6.1} mm² ({:.2} mm) | model outside slice {:6.1} mm² ({:.2} mm)",
        area_mm2(ours),
        area_mm2(theirs),
        miss.outside_mm2,
        miss.max_mm,
        extra.outside_mm2,
        extra.max_mm
    );
}

//! Fidelity campaign of the support model against sliced realizations (design §11, WP S4):
//!
//! `support_corpus <project.3mf> [--flavour F] [--cache-dir D] [--json OUT] [--dump OUT]
//!  <plate.gcode>[@object]...`
//!
//! The project is a corpus project at its rotation (the item transform carries the rotation, so
//! the model runs at rotation 0 in the loaded frame). Every G-code is a slice of that rotation
//! class; `@object` picks the object of a multi-object plate by its G-code order (default 0).
//! The object's placement is found by matching its printed first layer to the model's, so any
//! plate works. For each observation the tool prints and writes (JSON lines) how far the
//! printed first-layer base, the supports above it and the interfaces lie outside the
//! ensemble union (δ = 0): area outside and the largest distance, per band. `--dump` writes
//! the loops for plotting.
use goat_packer::Point;
use goat_packer::all_realizations_from_gcode;
use goat_packer::embedded_mesh::{ObjectMesh, load_object_mesh};
use goat_packer::first_layer::{ProjectSettings, SlicerBuild, landing};
use goat_packer::object_print::{Loops, Margins, Realization};
use goat_packer::orca_project::project_settings;
use goat_packer::support_model::bands::realization;
use goat_packer::support_model::envelope::envelope;
use goat_packer::support_model::exceedance::{area_mm2, exceedance};
use goat_packer::support_model::flavour::Flavour;
use goat_packer::support_model::graph::branch_graph;
use goat_packer::support_model::merge::{MergeOrder, ThreadClass};
use goat_packer::support_model::predict::{PredictInput, predict};
use goat_packer::support_model::settings::{ProjectConfig, SupportSettings};
use goat_packer::support_model::tips::ArcFamily;
use goat_packer::support_model::{StageAInput, stage_a};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

struct Options {
    project: PathBuf,
    flavour: Option<String>,
    cache_dir: Option<PathBuf>,
    json: Option<PathBuf>,
    dump: Option<PathBuf>,
    orders: String,
    single: Option<(usize, String)>,
    observations: Vec<(PathBuf, usize)>,
}

fn options() -> Options {
    let mut args = std::env::args().skip(1);
    let mut found = Options {
        project: PathBuf::from(args.next().expect("project.3mf")),
        flavour: None,
        cache_dir: None,
        json: None,
        dump: None,
        orders: "along".to_owned(),
        single: None,
        observations: Vec::new(),
    };
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--flavour" => found.flavour = args.next(),
            "--orders" => found.orders = args.next().unwrap(),
            "--cache-dir" => found.cache_dir = args.next().map(PathBuf::from),
            "--json" => found.json = args.next().map(PathBuf::from),
            "--dump" => found.dump = args.next().map(PathBuf::from),
            "--single" => {
                let threads = args.next().unwrap().parse().unwrap();
                found.single = Some((threads, args.next().unwrap()));
            }
            other => {
                let (path, index) = other.split_once('@').unwrap_or((other, "0"));
                found
                    .observations
                    .push((PathBuf::from(path), index.parse().unwrap()));
            }
        }
    }
    found
}

fn main() {
    let options = options();
    let orders = parse_orders(&options.orders);
    let mesh = load_object_mesh(&options.project).unwrap();
    let config = ProjectConfig::load(&options.project).unwrap();
    let settings = SupportSettings::from_config(&config).unwrap();
    let flavour = Flavour::resolve(
        options.flavour.as_deref(),
        config.application.as_deref(),
        None,
    )
    .unwrap()
    .flavour;
    let model_landing = model_landing(&options.project, &config, &mesh);
    let ensemble = match &options.single {
        Some((threads, family)) => vec![single(&mesh, &settings, flavour, *threads, family)],
        None => {
            let prediction = predict(&PredictInput {
                mesh: &mesh,
                settings: &settings,
                flavour,
                rotation_deg: 0.0,
                cache_dir: options.cache_dir.as_deref(),
                orders: &orders,
            })
            .unwrap();
            println!(
                "{flavour:?}: {} members, {} distinct, {:.1} s{}",
                prediction.runs,
                prediction.ensemble.realizations.len(),
                prediction.seconds,
                if prediction.cached { " (cache)" } else { "" }
            );
            prediction.ensemble.realizations
        }
    };
    let members: Vec<&Realization> = ensemble.iter().collect();
    let union = envelope(&members, Margins::default());
    println!(
        "union: base {:.1} mm², bands {:?}",
        area_mm2(&union.base),
        union
            .above_base
            .iter()
            .map(|b| area_mm2(b).round())
            .collect::<Vec<_>>()
    );
    let mut out = options
        .json
        .as_ref()
        .map(|path| std::fs::File::create(path).unwrap());
    let mut printed_all = Vec::new();
    for (path, index) in &options.observations {
        let started = Instant::now();
        let printed = observation(path, *index, &model_landing);
        let line = compare(&union, &printed, path, *index);
        println!(
            "{} ({:.1} s)",
            summary(&line),
            started.elapsed().as_secs_f64()
        );
        if let Some(file) = &mut out {
            writeln!(file, "{line}").unwrap();
        }
        printed_all.push(printed);
    }
    if let Some(path) = &options.dump {
        let value = json!({"model": union, "members": members, "slices": printed_all});
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
}

/// One member of the generator alone, for debugging it against deterministic slices.
fn single(
    mesh: &ObjectMesh,
    settings: &SupportSettings,
    flavour: Flavour,
    threads: usize,
    family: &str,
) -> Realization {
    let stage = stage_a(&StageAInput {
        mesh,
        settings,
        flavour,
        bed: None,
    })
    .unwrap();
    let family = match family {
        "roof" => ArcFamily::Roof,
        "call" => ArcFamily::PerCall,
        _ => ArcFamily::Base,
    };
    let graph = branch_graph(&stage, family, ThreadClass::along(threads));
    realization(&stage, &graph, 0)
}

/// The first-layer model's loops (frame origin 0), the placement reference.
fn model_landing(project: &Path, config: &ProjectConfig, mesh: &ObjectMesh) -> Loops {
    let settings = ProjectSettings::new(project_settings(project).unwrap());
    let build = config
        .application
        .as_deref()
        .and_then(SlicerBuild::from_name)
        .unwrap_or(SlicerBuild::BambuStudio);
    landing(mesh, &settings, build, 0.0).unwrap().loops
}

/// The printed realization of one object, moved into the model's frame by matching its
/// first layer.
fn observation(path: &Path, index: usize, model_landing: &Loops) -> Realization {
    let bytes = std::fs::read(path).unwrap();
    let mut all = all_realizations_from_gcode(&bytes, 0);
    let (_, mut printed) = all.swap_remove(index.min(all.len() - 1));
    let (px, py) = centre(&printed.landing);
    let (mx, my) = centre(model_landing);
    translate(&mut printed, (px - mx, py - my));
    printed
}

fn centre(loops: &Loops) -> (f64, f64) {
    let (mut low, mut high) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for p in loops.iter().flatten() {
        low = (low.0.min(p.x), low.1.min(p.y));
        high = (high.0.max(p.x), high.1.max(p.y));
    }
    ((low.0 + high.0) * 0.5, (low.1 + high.1) * 0.5)
}

fn translate(realization: &mut Realization, shift: (f64, f64)) {
    let move_loops = |loops: &mut Loops| {
        for p in loops.iter_mut().flatten() {
            *p = Point {
                x: p.x - shift.0,
                y: p.y - shift.1,
            };
        }
    };
    move_loops(&mut realization.landing);
    move_loops(&mut realization.base);
    for bands in [
        &mut realization.above_base,
        &mut realization.support,
        &mut realization.interface,
    ] {
        bands.iter_mut().for_each(move_loops);
    }
}

fn compare(model: &Realization, printed: &Realization, path: &Path, index: usize) -> Value {
    let one = |ours: &Loops, theirs: &Loops| {
        let miss = exceedance(theirs, ours);
        let extra = exceedance(ours, theirs);
        json!({"outside_mm2": miss.outside_mm2, "max_mm": miss.max_mm,
               "model_outside_mm2": extra.outside_mm2, "model_max_mm": extra.max_mm,
               "area_observed": area_mm2(theirs), "area_model": area_mm2(ours)})
    };
    let empty = Loops::new();
    let bands = |ours: &[Loops], theirs: &[Loops]| -> Vec<Value> {
        (0..ours.len().max(theirs.len()))
            .map(|k| {
                one(
                    ours.get(k).unwrap_or(&empty),
                    theirs.get(k).unwrap_or(&empty),
                )
            })
            .collect()
    };
    json!({
        "observation": format!("{}@{index}", path.display()),
        "l1": one(&model.base, &printed.base),
        "above_base": bands(&model.above_base, &printed.above_base),
        "interface": bands(&model.interface, &printed.interface),
    })
}

fn summary(line: &Value) -> String {
    let worst = |key: &str| {
        line[key]
            .as_array()
            .map(|bands| {
                bands
                    .iter()
                    .map(|b| b["max_mm"].as_f64().unwrap())
                    .fold(0.0, f64::max)
            })
            .unwrap_or(0.0)
    };
    let total = |key: &str| {
        line[key]
            .as_array()
            .map(|bands| {
                bands
                    .iter()
                    .map(|b| b["outside_mm2"].as_f64().unwrap())
                    .sum::<f64>()
            })
            .unwrap_or(0.0)
    };
    format!(
        "{}: L1 {:.2} mm ({:.1} mm², observed {:.1}, model {:.1}) | bands {:.2} mm ({:.1} mm²) | interface {:.2} mm",
        line["observation"]
            .as_str()
            .unwrap()
            .rsplit('/')
            .nth(1)
            .unwrap_or(""),
        line["l1"]["max_mm"].as_f64().unwrap(),
        line["l1"]["outside_mm2"].as_f64().unwrap(),
        line["l1"]["area_observed"].as_f64().unwrap(),
        line["l1"]["area_model"].as_f64().unwrap(),
        worst("above_base"),
        total("above_base"),
        worst("interface"),
    )
}

fn parse_orders(text: &str) -> Vec<MergeOrder> {
    text.split(',')
        .map(|name| match name {
            "along" => MergeOrder::Along,
            "against" => MergeOrder::Against,
            other => panic!("unknown merge order {other}"),
        })
        .collect()
}

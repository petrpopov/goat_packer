//! Is a plate the owner printed fine still admissible against the model's envelope?
//!
//! `printed_ok_check <profile.json> <project.3mf> <plate> [--flavour F] [--cache-dir D]
//!  [--deltas a,b,c] [--printed-ok FILE] [--pairs-only]`
//!
//! `plate` is a key of `/tmp/goat_calib/printed_ok.json` (`p1s_plate3_cli`, `a2l_plate3_cli`,
//! `u1_koza48_aux50_65t`, ...). The prints are built the way `colored_3mf_packer
//! --support-source model` builds them (body kinds, first-layer landing, supports from the
//! ensemble union grown by δ) and the plate's printed placements are validated exactly
//! (`validate_vector`) under the default foreign gaps, for each δ in the list. The largest δ
//! that still validates is what the margin may be before it breaks a layout known to print.
use goat_packer::colored_packer::bed_rules::{GuiFlavour, ProjectBed, plate_rules};
use goat_packer::colored_packer::load_printer_profile;
use goat_packer::first_layer::{ProjectSettings, SlicerBuild, landing};
use goat_packer::mesh_section::{BodyBands, body_print};
use goat_packer::object_print::Margins;
use goat_packer::orca_project::project_settings;
use goat_packer::support_model::envelope::envelope;
use goat_packer::support_model::flavour::Flavour;
use goat_packer::support_model::merge::MergeOrder;
use goat_packer::support_model::predict::{PredictInput, predict};
use goat_packer::support_model::settings::{ProjectConfig, SupportSettings};
use goat_packer::{ForeignGaps, LayoutRules, PrintPlacement, validate_vector};
use std::path::{Path, PathBuf};

fn main() {
    let mut args = std::env::args().skip(1);
    let profile_path = PathBuf::from(args.next().expect("profile.json"));
    let project = PathBuf::from(args.next().expect("project.3mf"));
    let plate = args.next().expect("plate key");
    let mut flavour_name = None;
    let mut cache_dir = None;
    let mut orders_text = "along".to_owned();
    let mut deltas = vec![0.0, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0];
    let mut printed_ok = PathBuf::from("/tmp/goat_calib/printed_ok.json");
    let mut pairs_only = false;
    while let Some(option) = args.next() {
        match option.as_str() {
            "--flavour" => flavour_name = args.next(),
            "--cache-dir" => cache_dir = args.next().map(PathBuf::from),
            "--orders" => orders_text = args.next().unwrap(),
            "--deltas" => {
                deltas = args
                    .next()
                    .unwrap()
                    .split(',')
                    .map(|v| v.parse().unwrap())
                    .collect()
            }
            "--printed-ok" => printed_ok = PathBuf::from(args.next().unwrap()),
            // Neighbour pairs only: the printed plate sits on a bed whose edge and cutter the
            // slicer clipped its supports against, which an unclipped envelope cannot show.
            "--pairs-only" => pairs_only = true,
            other => panic!("unknown option {other}"),
        }
    }
    let orders = parse_orders(&orders_text);
    let mesh = goat_packer::embedded_mesh::load_object_mesh(&project).unwrap();
    let config = ProjectConfig::load(&project).unwrap();
    let settings = SupportSettings::from_config(&config).unwrap();
    let flavour = Flavour::resolve(flavour_name.as_deref(), config.application.as_deref(), None)
        .unwrap()
        .flavour;
    let prediction = predict(&PredictInput {
        mesh: &mesh,
        settings: &settings,
        flavour,
        rotation_deg: 0.0,
        cache_dir: cache_dir.as_deref(),
        orders: &orders,
    })
    .unwrap();
    println!(
        "{flavour:?}: {} distinct realizations in {:.1} s{}",
        prediction.ensemble.realizations.len(),
        prediction.seconds,
        if prediction.cached { " (cache)" } else { "" }
    );
    let members: Vec<_> = prediction.ensemble.realizations.iter().collect();
    let value = serde_json::Value::Object(project_settings(&project).unwrap());
    let project_settings = ProjectSettings::from_json(&value);
    let build = SlicerBuild::from_name(flavour.name()).unwrap();
    let mut profile = load_printer_profile(&profile_path).unwrap();
    profile.gui_flavour = Some(GuiFlavour::try_from(flavour.name().to_owned()).unwrap());
    let bed = ProjectBed::from_settings(&value).unwrap();
    let mut rules = plate_rules(&profile, &bed).unwrap();
    if pairs_only {
        rules.printable_area = rules.printable_area.grown(30.0);
        rules.body_edge_band_mm = 0.0;
        rules.print_edge_reserve_mm = 0.0;
        rules.keepouts.clear();
    }
    let layout = LayoutRules::new(&rules, None, 0.7, ForeignGaps::default(), None).unwrap();
    let placements = placements(&printed_ok, &plate);
    println!("{plate}: {} printed placements", placements.len());
    let layer_height = project_settings.number("layer_height").unwrap();
    let first_height = project_settings
        .number("initial_layer_print_height")
        .unwrap_or(0.0);
    let bands = BodyBands::printed(layer_height.max(first_height));
    let mut last_ok = None;
    for delta in deltas {
        let mut print = body_print(&mesh, 0.0, &bands);
        print.landing = landing(&mesh, &project_settings, build, 0.0).unwrap().loops;
        let grown = envelope(
            &members,
            Margins {
                first_layer_mm: delta,
                band_mm: delta,
            },
        );
        print.support_base = grown.base;
        print.support = grown.above_base;
        print.interface = grown.interface;
        let started = std::time::Instant::now();
        let verdict = validate_vector(std::slice::from_ref(&print), &placements, &layout);
        match &verdict {
            Ok(_) => last_ok = Some(delta),
            Err(error) => {
                let text = error.to_string();
                println!(
                    "delta {delta:4.2}: FAILS ({:.1} s) {}",
                    started.elapsed().as_secs_f64(),
                    text.chars().take(160).collect::<String>()
                );
                continue;
            }
        }
        println!(
            "delta {delta:4.2}: admissible ({:.1} s)",
            started.elapsed().as_secs_f64()
        );
    }
    println!("largest admissible delta in the list: {last_ok:?}");
}

fn placements(file: &Path, plate: &str) -> Vec<PrintPlacement> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    value["plates"][plate]["placements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| PrintPlacement {
            print: 0,
            x_mm: p["x"].as_f64().unwrap(),
            y_mm: p["y"].as_f64().unwrap(),
        })
        .collect()
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

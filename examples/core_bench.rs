//! Benchmark of the packing portfolio (`goat_packer::pack`) on real prints: the count each
//! member reaches alone, the member the full portfolio publishes, and the time per stage.
//!
//! Usage:
//! ```text
//! cargo run --release --example core_bench -- <print.json>... --rules <rules.json> \
//!   [--tower <tower.json>] [--members sweep,rows] [--threads N] [--out <layout.json>] \
//!   [--resolution 0.25] [--clearance 0.7] [--attempts 64] [--max-objects 0] \
//!   [--support-overlap <mm>]
//! ```
//! Each `<print.json>` is an `ObjectPrint` (one per rotation class, already in the
//! plate-aligned frame of its rotation); `--rules` is a `PlateRules`. `--tower` takes a
//! `TowerSpec`, or a measured tower in the goat bench format (a `measured` block), converted
//! by `tower_from_measured`. `--out` gets the published `PackOutcome` as JSON: two runs with
//! different `--threads` must write identical files.

use std::{path::PathBuf, process::exit, time::Instant};

use goat_packer::pack::{Member, PackOutcome, TowerSpec, pack_members};
use goat_packer::{ObjectPrint, PackOptions, PlateRules, Point, Rect};
use serde_json::Value;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        exit(1);
    }
}

struct Args {
    prints: Vec<PathBuf>,
    rules: PathBuf,
    tower: Option<PathBuf>,
    members: Vec<Member>,
    threads: Option<usize>,
    out: Option<PathBuf>,
    options: PackOptions,
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let prints: Vec<ObjectPrint> = args
        .prints
        .iter()
        .map(read_json)
        .collect::<Result<_, _>>()?;
    let rules: PlateRules = read_json(&args.rules)?;
    let tower = args.tower.as_ref().map(read_tower).transpose()?;
    let mut builder = rayon::ThreadPoolBuilder::new();
    if let Some(threads) = args.threads {
        builder = builder.num_threads(threads);
    }
    let pool = builder.build().map_err(|e| e.to_string())?;
    let options = PackOptions {
        bed: rules.printable_area,
        ..args.options.clone()
    };
    println!(
        "{} class(es), {} thread(s), tower: {}",
        prints.len(),
        pool.current_num_threads(),
        tower.as_ref().map_or("none".to_owned(), describe_tower)
    );
    let pack = |members: &[Member]| {
        let started = Instant::now();
        let outcome =
            pool.install(|| pack_members(&prints, &rules, tower.as_ref(), &options, members));
        (outcome, started.elapsed().as_secs_f64())
    };
    for &member in &args.members {
        let (outcome, seconds) = pack(&[member]);
        match outcome {
            Ok(outcome) => println!(
                "member {member:?}: {} objects, {}",
                outcome.placements.len(),
                times(&outcome, seconds)
            ),
            Err(error) => println!("member {member:?}: error: {error}"),
        }
    }
    let (outcome, seconds) = pack(&args.members);
    let outcome = outcome?;
    report(&outcome, seconds);
    if let Some(out) = &args.out {
        let json = serde_json::to_string_pretty(&outcome).map_err(|e| e.to_string())?;
        std::fs::write(out, json).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    Ok(())
}

fn parse_args() -> Result<Args, String> {
    let mut prints = Vec::new();
    let mut rules = None;
    let mut tower = None;
    let mut members = vec![Member::Sweep, Member::Rows];
    let mut threads = None;
    let mut out = None;
    let mut options = default_options();
    let mut args = std::env::args().skip(1);
    while let Some(option) = args.next() {
        if !option.starts_with("--") {
            prints.push(PathBuf::from(option));
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        let number = |value: &str| value.parse::<f64>().map_err(|e| format!("{option}: {e}"));
        let count = |value: &str| value.parse::<usize>().map_err(|e| format!("{option}: {e}"));
        match option.as_str() {
            "--rules" => rules = Some(PathBuf::from(value)),
            "--tower" => tower = Some(PathBuf::from(value)),
            "--members" => members = parse_members(&value)?,
            "--threads" => threads = Some(count(&value)?),
            "--out" => out = Some(PathBuf::from(value)),
            "--resolution" => options.resolution = number(&value)?,
            "--clearance" => options.clearance = number(&value)?,
            "--attempts" => options.attempts = count(&value)?,
            "--max-objects" => options.max_objects = count(&value)?,
            "--support-overlap" => options.support_overlap_mm = Some(number(&value)?),
            _ => return Err(format!("unknown option {option}")),
        }
    }
    if prints.is_empty() {
        return Err("give at least one print JSON".to_owned());
    }
    Ok(Args {
        prints,
        rules: rules.ok_or("--rules is required")?,
        tower,
        members,
        threads,
        out,
        options,
    })
}

/// The production defaults of `colored_3mf_packer` (resolution, clearance, effort).
fn default_options() -> PackOptions {
    PackOptions {
        bed: Rect::new(0.0, 0.0, 1.0, 1.0),
        body_bed: None,
        exclusions: Vec::new(),
        body_exclusions: Vec::new(),
        resolution: 0.25,
        clearance: 0.7,
        angle_step_deg: 2.0,
        attempts: 16_000,
        candidates_per_step: 0,
        stall_attempts: 0,
        max_objects: 0,
        lock_orientation: true,
        seed: 0,
        support_overlap_mm: None,
        plate_rules: None,
    }
}

fn parse_members(list: &str) -> Result<Vec<Member>, String> {
    list.split(',')
        .map(|name| {
            serde_json::from_value(Value::String(name.trim().to_owned()))
                .map_err(|_| format!("unknown member {name}"))
        })
        .collect()
}

fn read_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_tower(path: &PathBuf) -> Result<TowerSpec, String> {
    let value: Value = read_json(path)?;
    if value.get("measured").is_some() {
        tower_from_measured(&value).ok_or_else(|| format!("{}: bad measured tower", path.display()))
    } else {
        serde_json::from_value(value).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// A measured goat-bench tower as a `TowerSpec`: the body relative to the anchor, the widest
/// side of the L0 brim and of every chamfer ring, and as the anchor range the intersection of
/// the GUI's clamp (estimated size) with the range of the measured body (the GUI keeps the
/// estimated tower inside its margin, the printed one must stay there too).
fn tower_from_measured(value: &Value) -> Option<TowerSpec> {
    let numbers =
        |v: &Value| -> Option<Vec<f64>> { v.as_array()?.iter().map(Value::as_f64).collect() };
    let rect = |v: &Value| {
        numbers(v)
            .filter(|n| n.len() == 4)
            .map(|n| Rect::new(n[0], n[1], n[2], n[3]))
    };
    let widest = |v: &Value| numbers(v).map(|n| n.into_iter().fold(0.0, f64::max));
    let measured = &value["measured"];
    let chamfer = measured["chamfer"]
        .as_array()?
        .iter()
        .map(|ring| Some([ring["z"].as_f64()?, widest(&ring["beyond_body_mm"])?]))
        .collect::<Option<Vec<_>>>()?;
    let gui = rect(&value["gui_anchor_range"]["repo_formula"])?;
    let printed = rect(&value["gui_anchor_range"]["measured_size"])?;
    let source = numbers(&value["anchor"]["source"])?;
    Some(TowerSpec {
        body: rect(&value["relative_to_anchor"]["body_rect"])?,
        brim_l0_mm: widest(&measured["l0_brim_beyond_body_mm"])?,
        chamfer,
        top_z_mm: measured["top_z"].as_f64()?,
        anchor_range: Rect::new(
            gui.min_x.max(printed.min_x),
            gui.min_y.max(printed.min_y),
            gui.max_x.min(printed.max_x),
            gui.max_y.min(printed.max_y),
        ),
        source_anchor: Point {
            x: *source.first()?,
            y: *source.get(1)?,
        },
    })
}

fn describe_tower(tower: &TowerSpec) -> String {
    let b = tower.body;
    format!(
        "body {:.2} x {:.2}, brim {:.2}, {} chamfer rings, source ({:.2}, {:.2})",
        b.max_x - b.min_x,
        b.max_y - b.min_y,
        tower.brim_l0_mm,
        tower.chamfer.len(),
        tower.source_anchor.x,
        tower.source_anchor.y
    )
}

fn times(outcome: &PackOutcome, seconds: f64) -> String {
    let t = outcome.timings;
    format!(
        "{seconds:.3} s (context {:.3}, members {:.3}, select {:.3})",
        t.context_s, t.members_s, t.select_s
    )
}

fn report(outcome: &PackOutcome, seconds: f64) {
    let counts: Vec<String> = outcome
        .member_counts
        .iter()
        .map(|m| format!("{:?} {}", m.member, m.count))
        .collect();
    println!("portfolio: {}", counts.join(", "));
    let anchor = outcome
        .tower_anchor
        .map_or("none".to_owned(), |a| format!("({:.2}, {:.2})", a.x, a.y));
    println!(
        "chosen: {:?} {:?}, {} objects, tower anchor {anchor}, box {} px, {}",
        outcome.member,
        outcome.key.tie.detail,
        outcome.placements.len(),
        outcome.key.bbox_area_px,
        times(outcome, seconds)
    );
    let v = &outcome.validation;
    println!(
        "validation: {} shortlisted, {} raster-valid, {} vector failures, winner rank {}",
        v.shortlisted,
        v.raster_passed,
        v.vector_failures.len(),
        v.winner_rank
    );
}

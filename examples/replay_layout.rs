//! Replays a packed layout onto another single-object source 3MF: identical placements,
//! different model — e.g. packing the magnet twin of a goat plate whose layout was found on
//! the hole-punched twin. The placement frame is the mesh bbox center, and both twins share
//! it, so the positions carry over unchanged.
//!
//! Usage:
//! ```text
//! cargo run --release --example replay_layout -- \
//!   --source goat1_magnet_single_pla.3mf \
//!   --placements work/placements.json \
//!   --profile profiles/bambu_a2l.json \
//!   --output packet_31.3mf \
//!   --tower-keepout 228.56,157.59,243.37,190.41   # optional, preview-only
//! ```

use std::{path::PathBuf, process::exit};

use goat_packer::{
    PackResult, Rect, extract_footprint,
    assemble::{AssembleOptions, assemble_project},
    colored_packer::load_printer_profile,
    embedded_mesh::{embedded_mesh_triangles, write_binary_stl},
};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut source: Option<PathBuf> = None;
    let mut placements_path: Option<PathBuf> = None;
    let mut profile_path: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut tower_keepout: Option<Rect> = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{flag} expects a value"))
        };
        match flag.as_str() {
            "--source" => source = Some(value()?.into()),
            "--placements" => placements_path = Some(value()?.into()),
            "--profile" => profile_path = Some(value()?.into()),
            "--output" => output = Some(value()?.into()),
            "--tower-keepout" => {
                let raw = value()?;
                let numbers: Vec<f64> = raw
                    .split(',')
                    .map(|piece| {
                        piece
                            .trim()
                            .parse::<f64>()
                            .map_err(|_| format!("--tower-keepout: «{piece}» is not a number"))
                    })
                    .collect::<Result<_, _>>()?;
                if numbers.len() != 4 {
                    return Err("--tower-keepout expects x0,y0,x1,y1".to_owned());
                }
                tower_keepout = Some(Rect::new(
                    numbers[0], numbers[1], numbers[2], numbers[3],
                ));
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    let source = source.ok_or("--source is required (single-object source 3MF)")?;
    let placements_path =
        placements_path.ok_or("--placements is required (the packed run's placements.json)")?;
    let profile_path = profile_path.ok_or("--profile is required (printer profile json)")?;
    let output = output.ok_or("--output is required")?;

    let profile = load_printer_profile(&profile_path)
        .map_err(|error| format!("cannot load profile {}: {error}", profile_path.display()))?;
    let placements: PackResult = serde_json::from_slice(
        &std::fs::read(&placements_path)
            .map_err(|error| format!("cannot read {}: {error}", placements_path.display()))?,
    )
    .map_err(|error| format!("cannot parse {}: {error}", placements_path.display()))?;
    if placements.placements.is_empty() {
        return Err("placements.json holds no placements".to_owned());
    }

    // The same collision STL the packer derives: the source 3MF's own embedded mesh.
    let stl_path = output.with_extension("replay.stl");
    let triangles = embedded_mesh_triangles(&source)
        .map_err(|error| format!("failed to derive collision STL from embedded mesh: {error}"))?;
    if triangles.is_empty() {
        return Err("embedded mesh has no triangles".to_owned());
    }
    write_binary_stl(&stl_path, &triangles)
        .map_err(|error| format!("failed to write {}: {error}", stl_path.display()))?;

    // A layout replay keeps the original placements, so the keepouts only feed the preview —
    // no probe slice exists here to measure a purge tip overshoot from.
    let mut keepouts = profile
        .fixed_keepouts(0.0)
        .map_err(|error| format!("profile keepouts: {error}"))?;
    if let Some(rect) = tower_keepout {
        keepouts.push(rect);
    }

    let footprint = extract_footprint(&stl_path)
        .map_err(|error| format!("cannot extract the footprint from {}: {error}", stl_path.display()))?;
    let options = AssembleOptions {
        placements: placements.placements.clone(),
        stl_path: stl_path.clone(),
        edge_margin: profile.edge_margin_mm,
        purge_lines: profile.purge_lines.clone(),
        keepouts,
        duplicate_mesh_per_object: false,
        reuse_source_object_instances: false,
        inline_colored_mesh_objects: false,
        footprint_override: Some(footprint),
    };
    assemble_project(&source, &output, &options)
        .map_err(|error| format!("3MF assembly failed: {error}"))?;

    println!(
        "replayed {} placements onto {}; wrote {} (collision STL: {})",
        placements.placements.len(),
        source.display(),
        output.display(),
        stl_path.display()
    );
    Ok(())
}

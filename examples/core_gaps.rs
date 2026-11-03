//! Gap statistics of a published layout (a `core_bench --out` file) on the prints it used.
//!
//! Usage: `cargo run --release --example core_gaps -- <print.json>... --layout <layout.json>`
//! (prints in the order given to `core_bench`: the placements' `print` is an index into them).
//!
//! Reports, over every pair of copies, the smallest distance between
//! - the support bands of one copy and the body bands of another in the same 2 mm band,
//! - the first-layer support base of one copy and the landing (first-layer body) of another,
//! - the silhouettes (body against body), for reference.
//!
//! Distances are between ring boundaries (a layout that passed the checks has no overlap),
//! in mm, so a negative or zero number would mean a bug in the packer.

use geo::{Distance, Euclidean, LineString};
use goat_packer::{ObjectPrint, Point};
use serde_json::Value;
use std::{path::PathBuf, process::exit};

/// Pairs of copies farther apart than this (bounding boxes) cannot hold a gap of interest.
const NEAR_MM: f64 = 12.0;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        exit(1);
    }
}

type Rings = Vec<LineString<f64>>;

fn rings(loops: &[Vec<Point>], dx: f64, dy: f64) -> Rings {
    loops
        .iter()
        .map(|l| l.iter().map(|p| (p.x + dx, p.y + dy)).collect())
        .collect()
}

fn gap(a: &Rings, b: &Rings) -> f64 {
    let mut best = f64::INFINITY;
    for x in a {
        for y in b {
            best = best.min(Euclidean.distance(x, y));
        }
    }
    best
}

struct Placed {
    print: usize,
    x: f64,
    y: f64,
}

fn run() -> Result<(), String> {
    let mut prints: Vec<ObjectPrint> = Vec::new();
    let mut layout = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--layout" {
            layout = args.next().map(PathBuf::from);
        } else {
            prints.push(read(&PathBuf::from(arg))?);
        }
    }
    let layout: Value = read(&layout.ok_or("--layout is required")?)?;
    let copies: Vec<Placed> = layout["placements"]
        .as_array()
        .ok_or("no placements")?
        .iter()
        .map(|p| Placed {
            print: p["print"].as_u64().unwrap_or(0) as usize,
            x: p["x_mm"].as_f64().unwrap_or(0.0),
            y: p["y_mm"].as_f64().unwrap_or(0.0),
        })
        .collect();
    let (mut support, mut base, mut body) = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
    for (i, a) in copies.iter().enumerate() {
        for b in &copies[i + 1..] {
            let (pa, pb) = (&prints[a.print], &prints[b.print]);
            let near = rings(&pa.silhouette, a.x, a.y);
            let far = rings(&pb.silhouette, b.x, b.y);
            if gap(&near, &far) > NEAR_MM {
                continue;
            }
            body = body.min(gap(&near, &far));
            for (from, to) in [(a, b), (b, a)] {
                let (pf, pt) = (&prints[from.print], &prints[to.print]);
                base = base.min(gap(
                    &rings(&pf.support_base, from.x, from.y),
                    &rings(&pt.landing, to.x, to.y),
                ));
                for (k, supports) in pf.support.iter().enumerate() {
                    let Some(bodies) = pt.body.get(k) else {
                        continue;
                    };
                    support = support.min(gap(
                        &rings(supports, from.x, from.y),
                        &rings(bodies, to.x, to.y),
                    ));
                }
            }
        }
    }
    println!("{} copies", copies.len());
    println!("min support -> foreign body gap (same band): {support:.3} mm");
    println!("min support base -> foreign landing gap:     {base:.3} mm");
    println!("min silhouette -> silhouette gap:            {body:.3} mm");
    Ok(())
}

fn read<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

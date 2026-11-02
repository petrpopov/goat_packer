//! Real-data check of support-model stage A (WP S2) on a project:
//! `support_stage_a <3mf> [rotation_deg] [interface.json]`
//!
//! Prints the flavour, layer grid, derived constants, timing and per-layer overhangs. With a
//! G-code extraction (`/tmp/goat_calib/json/<id>.json.gz`, gunzipped) of the same project at
//! the same rotation, it compares the hand-off overhang H_k with the printed support
//! interface of layer k − z_delta (spec 01 §9 A4 cross-check). G-code points map to the
//! plate-aligned frame by the source item's frame origin (a pure translation), so pass the
//! rotated corpus project itself with rotation 0. `ROOF_DEPTH` / `Z_DELTA` override the
//! interface stack depth and tip offset of the comparison (B prints a 4-layer stack).
use goat_packer::embedded_mesh::load_object_mesh;
use goat_packer::polygon_ops::{
    IntPoint, Join, Ring, Rings, area, difference, intersection, offset, union_all,
};
use goat_packer::support_model::clearance::Avoidance;
use goat_packer::support_model::flavour::Flavour;
use goat_packer::support_model::settings::{ProjectConfig, SupportSettings};
use goat_packer::support_model::{StageAInput, stage_a};
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = Path::new(&args[0]);
    let rotation: f64 = args.get(1).map_or(0.0, |text| text.parse().unwrap());
    let start = Instant::now();
    let loaded = load_object_mesh(path).unwrap();
    let origin = [loaded.frame_origin[0], loaded.frame_origin[1]];
    let mesh = loaded.rotated(rotation);
    let config = ProjectConfig::load(path).unwrap();
    let settings = SupportSettings::from_config(&config).unwrap();
    let flavour = Flavour::resolve(None, config.application.as_deref(), None).unwrap();
    println!(
        "{path:?} rotation {rotation}: flavour {:?}, load {:?}",
        flavour,
        start.elapsed()
    );
    let start = Instant::now();
    let stage = stage_a(&StageAInput {
        mesh: &mesh,
        settings: &settings,
        flavour: flavour.flavour,
        bed: None,
    })
    .unwrap();
    println!("stage A {:?}", start.elapsed());
    let grid = &stage.grid;
    let first = grid.layers[0];
    let last = grid.layers[grid.len() - 1];
    println!(
        "layers {} first [{:.3}, {:.3}] last [{:.3}, {:.3}] frame shift {:?}",
        grid.len(),
        first.bottom,
        first.top,
        last.bottom,
        last.top,
        stage.frame_shift
    );
    let d = &stage.derived;
    println!(
        "derived: lh {} w_s {} r_b {} r_min {} V_f {} V_s {} tip_layers {} xy {} xy_min {} z_delta {} connect {} res {} keys {:?}",
        d.layer_height,
        d.line_width,
        d.branch_radius,
        d.tip_radius,
        d.move_fast,
        d.move_slow,
        d.tip_layers,
        d.xy,
        d.xy_min,
        d.z_delta,
        d.connect_length,
        d.resolution,
        stage.clearance.precalculated_keys(d, 8)
    );
    // Volume cost for stage B: every precalculated key, all three avoidances, every layer.
    let start = Instant::now();
    let keys = stage.clearance.precalculated_keys(d, 8);
    let top = stage.overhangs.highest_layer().unwrap_or(0);
    for &key in &keys {
        for kind in [Avoidance::Slow, Avoidance::FastSafe, Avoidance::Fast] {
            stage.clearance.avoidance(key, top, kind, true);
        }
        stage.clearance.wall_restriction(key, top.max(1), true);
    }
    println!(
        "volumes for {} keys up to layer {top}: {:?}",
        keys.len(),
        start.elapsed()
    );
    let units = stage.derived.grid;
    let mm2 = |rings: &Rings| units.mm2(area(rings));
    let overhangs = &stage.overhangs;
    println!(
        "max islands {} clusters {} small {} bridge skipped {}",
        overhangs.stats.islands.iter().max().unwrap_or(&0),
        overhangs.stats.clusters,
        overhangs.stats.small_clusters,
        overhangs.stats.bridge_removal_skipped
    );
    let interface = args
        .get(2)
        .map(|json| interface_layers(json, origin, units, &stage.frame_shift));
    let count = overhangs.handoff.len();
    let roofs = std::env::var("ROOF_DEPTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d.roof_layers.max(1));
    let mut totals = [0.0f64; 6];
    for printed_layer in 0..count {
        // Interface printed on layer j comes from the roof stacks of H_{j+z_delta+t}, t < R.
        let z_delta = std::env::var("Z_DELTA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d.z_delta);
        let sources: Vec<usize> = (0..roofs)
            .map(|t| printed_layer + z_delta + t)
            .filter(|&k| k < count)
            .collect();
        let predicted = union_all(sources.iter().map(|&k| &overhangs.handoff[k]));
        let printed = interface
            .as_ref()
            .and_then(|layers| layers.get(printed_layer).cloned())
            .unwrap_or_default();
        let top = &overhangs
            .handoff
            .get(printed_layer + d.z_delta)
            .cloned()
            .unwrap_or_default();
        if predicted.is_empty() && printed.is_empty() {
            continue;
        }
        let kinds: Vec<&str> = overhangs
            .layers
            .get(printed_layer + d.z_delta)
            .map(|typed| {
                typed
                    .iter()
                    .map(|t| {
                        if t.flags.cantilever {
                            "C"
                        } else if t.flags.sharp_tail {
                            "T"
                        } else if t.flags.enforced {
                            "E"
                        } else {
                            "N"
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut line = format!(
            "print layer {printed_layer:3} z {:6.2}: H(j+{}) {:7.2} roofs {:7.2} mm2 {:?}",
            grid.layers[printed_layer].top,
            d.z_delta,
            mm2(&union_all([top])),
            mm2(&predicted),
            kinds
        );
        totals[0] += mm2(&predicted);
        if interface.is_some() {
            let grow = |rings: &Rings, mm: f64| {
                offset(rings, units.scaled(mm), Join::Round(units.scaled(0.01)))
            };
            let inside = mm2(&intersection(&predicted, &printed));
            let outside1 = mm2(&difference(&printed, &grow(&predicted, 1.0)));
            let outside2 = mm2(&difference(&printed, &grow(&predicted, 2.0)));
            let excess = mm2(&difference(&predicted, &grow(&printed, 1.0)));
            line += &format!(
                " | I {:7.2} in {:7.2} out+1 {:6.2} out+2 {:6.2} | predicted beyond I+1 {:6.2}",
                mm2(&printed),
                inside,
                outside1,
                outside2,
                excess
            );
            totals[1] += mm2(&printed);
            totals[2] += inside;
            totals[3] += outside1;
            totals[4] += outside2;
            totals[5] += excess;
        }
        println!("{line}");
    }
    println!(
        "totals: roofs {:.1} mm2, I {:.1}, I in roofs {:.1}, I outside roofs+1 {:.1}, outside roofs+2 {:.1}, roofs beyond I+1 {:.1}",
        totals[0], totals[1], totals[2], totals[3], totals[4], totals[5]
    );
}

/// Printed support interface per layer (index = layer - 1 of the extraction), object frame,
/// lines thickened to their comment width with square ends.
fn interface_layers(
    json: &str,
    t: [f64; 2],
    units: goat_packer::polygon_ops::Grid,
    shift: &[f64; 2],
) -> Vec<Rings> {
    let text = std::fs::read_to_string(json).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let layers = value["layers"].as_array().unwrap();
    let mut out = vec![Rings::new(); layers.len()];
    for layer in layers {
        let index = layer["layer"].as_u64().unwrap() as usize - 1;
        let Some(lines) = layer["roles"]
            .get("support_interface")
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        let mut rings = Rings::new();
        for line in lines {
            let width = line["w"].as_f64().unwrap_or(0.42);
            let xy: Vec<f64> = line["xy"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            let points: Vec<[f64; 2]> = xy
                .chunks(2)
                .map(|p| {
                    // The plate-aligned frame of the rotation class is a pure translation of the
                    // plate (the source item's frame origin), then the slicer re-centring.
                    [p[0] - t[0] - shift[0], p[1] - t[1] - shift[1]]
                })
                .collect();
            for pair in points.windows(2) {
                rings.push(segment(pair[0], pair[1], width / 2.0, units));
            }
        }
        out[index] = union_all([&rings]);
    }
    out
}

fn segment(a: [f64; 2], b: [f64; 2], half: f64, units: goat_packer::polygon_ops::Grid) -> Ring {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = (dx * dx + dy * dy).sqrt().max(1e-9);
    let (nx, ny) = (-dy / length * half, dx / length * half);
    let (ex, ey) = (dx / length * half, dy / length * half);
    let p = |x: f64, y: f64| -> IntPoint { units.point(x, y) };
    vec![
        p(a[0] - ex + nx, a[1] - ey + ny),
        p(a[0] - ex - nx, a[1] - ey - ny),
        p(b[0] + ex - nx, b[1] + ey - ny),
        p(b[0] + ex + nx, b[1] + ey + ny),
    ]
}

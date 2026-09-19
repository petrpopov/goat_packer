//! One-off diagnostic: run `patch_within_silhouette` on a real job's probe gcode + embedded
//! STL and print exactly which patch pixels fall outside the tolerated silhouette.
//! Usage: cargo run --release --example debug_containment -- <work_dir>

use std::path::Path;

use goat_packer::{extract_footprint, first_layer_footprint, patch_within_silhouette, point_in_footprint};

fn main() {
    let work_arg = std::env::args().nth(1).expect("work dir argument");
    let work = Path::new(&work_arg);
    let stl = work.join("embedded_source.stl");
    let gcode = std::fs::read(work.join("probe_slice/plate_1.gcode")).expect("probe gcode");

    let silhouette = extract_footprint(&stl).unwrap();
    println!(
        "silhouette: {} loops, {:.2}x{:.2}",
        silhouette.loops.len(),
        silhouette.width,
        silhouette.height
    );
    let patch = first_layer_footprint(&gcode, None).expect("patch");
    println!(
        "patch (self-centered): {} loops, {:.2}x{:.2}, bounds {:?}",
        patch.loops.len(),
        patch.width,
        patch.height,
        patch_bounds(&patch)
    );

    let verdict = patch_within_silhouette(&patch, &silhouette, 0.25, 1.2);
    println!("patch_within_silhouette(res 0.25, tol 1.2): {verdict}");

    // The binary centers the patch on the model's world center; None centers it on the
    // body's own bbox center. Compare both framings.
    for (cx, cy) in [(128.0f64, 128.0f64)] {
        let centered = first_layer_footprint(&gcode, Some((cx, cy))).expect("patch");
        let v = patch_within_silhouette(&centered, &silhouette, 0.25, 1.2);
        println!(
            "with model_center ({cx}, {cy}): containment = {v}, patch {}x{}",
            centered.width, centered.height
        );
    }

    // Failing pixels in the shared loop frame.
    let resolution = 0.25f64;
    let radius = (1.2 / resolution).round() as i32;
    let mut failures = 0;
    for iy in -400..=400 {
        for ix in -400..=400 {
            let x = (ix as f64 + 0.5) * resolution;
            let y = (iy as f64 + 0.5) * resolution;
            if point_in_footprint(&patch.loops, x, y) {
                let mut tolerated = false;
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        if point_in_footprint(
                            &silhouette.loops,
                            x + dx as f64 * resolution,
                            y + dy as f64 * resolution,
                        ) {
                            tolerated = true;
                            break;
                        }
                    }
                    if tolerated {
                        break;
                    }
                }
                if !tolerated {
                    if failures < 20 {
                        println!("  outside pixel at ({x:.2}, {y:.2})");
                    }
                    failures += 1;
                }
            }
        }
    }
    println!("failing patch pixels: {failures}");
}

fn patch_bounds(footprint: &goat_packer::Footprint) -> (f64, f64, f64, f64) {
    let mut b = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in footprint.loops.iter().flatten() {
        b.0 = b.0.min(p.x);
        b.1 = b.1.min(p.y);
        b.2 = b.2.max(p.x);
        b.3 = b.3.max(p.y);
    }
    b
}

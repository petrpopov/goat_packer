//! Real-data check of the S1 mesh kinds.
//!
//! `mesh_check summary <3mf>...` prints frame, parts, bands and timing.
//! `mesh_check frame <rot0.3mf> <rotN.3mf> <N>` compares a source item turned by N degrees
//! with the unturned object turned in the plate-aligned frame (pure translation apart).
//! `mesh_check export <3mf> <rotation_deg> <pad_mm> <bulge_mm> <out.json>` writes the `ObjectPrint`
//! body kinds (plate-aligned frame of that rotation) for comparison with sliced G-code.
use goat_packer::embedded_mesh::load_object_mesh;
use goat_packer::mesh_section::{BodyBands, SectionOptions, body_print, object_section};
use goat_packer::{Rect, polygon_area};
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("summary") => args[1..].iter().for_each(|path| summary(path)),
        Some("frame") => frame(&args[1], &args[2], args[3].parse().unwrap()),
        Some("export") => export(
            &args[1],
            args[2].parse().unwrap(),
            args[3].parse().unwrap(),
            args[4].parse().unwrap(),
            &args[5],
        ),
        _ => eprintln!(
            "usage: mesh_check summary <3mf>... | export <3mf> <rotation> <pad> <bulge> <out.json>"
        ),
    }
}

fn summary(path: &str) {
    let start = Instant::now();
    let mesh = load_object_mesh(Path::new(path)).unwrap();
    let (min, max) = mesh.normal_bounds();
    println!(
        "{path}: load {:?} origin {:?}",
        start.elapsed(),
        mesh.frame_origin
    );
    println!(
        "  normal bounds {min:?} .. {max:?}, height {:.4}",
        mesh.height()
    );
    for part in &mesh.parts {
        println!(
            "  part {} {:?}: {} triangles, {} painted",
            part.source_id,
            part.kind,
            part.triangles.len(),
            part.paint_supports.len()
        );
    }
    let start = Instant::now();
    let print = body_print(&mesh, 0.0, &BodyBands::printed(0.28));
    let bbox = Rect::of_points(print.silhouette.iter().flatten());
    println!(
        "  body_print {:?}: {} bands, silhouette bbox {bbox:?}",
        start.elapsed(),
        print.body.len()
    );
    for (band, loops) in print.body.iter().enumerate() {
        let area: f64 = loops.iter().map(|ring| polygon_area(ring)).sum();
        println!("    band {band}: {} loops, {area:.2} mm2", loops.len());
    }
    let start = Instant::now();
    let section = object_section(&mesh, 10.0, &SectionOptions::slicer(0.049));
    let area: f64 = section.iter().map(|ring| polygon_area(ring)).sum();
    println!(
        "  object section z = 10: {:?}, {} loops, {area:.3} mm2",
        start.elapsed(),
        section.len()
    );
}

fn export(path: &str, rotation: f64, pad: f64, bulge: f64, out: &str) {
    let mesh = load_object_mesh(Path::new(path)).unwrap();
    let bands = BodyBands {
        pad_mm: pad,
        bulge_mm: bulge,
        ..BodyBands::mesh_only()
    };
    let print = body_print(&mesh, rotation, &bands);
    let json = serde_json::json!({
        "frame_origin": mesh.frame_origin,
        "rotation_deg": rotation,
        "pad_mm": pad,
        "body": print.body,
        "silhouette": print.silhouette,
        "part_hulls": print.part_hulls,
        "part_boxes": print.part_boxes,
    });
    std::fs::write(out, json.to_string()).unwrap();
}

fn frame(unturned: &str, turned: &str, degrees: f64) {
    let base = load_object_mesh(Path::new(unturned))
        .unwrap()
        .rotated(degrees);
    let item = load_object_mesh(Path::new(turned)).unwrap();
    // The turned item's frame is centred on its own bbox; ours stays on the unturned centre.
    let (min, max) = base.normal_bounds();
    let shift = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
    let mut worst = 0.0f64;
    for (a, b) in base.parts.iter().zip(&item.parts) {
        for (p, q) in a.vertices.iter().zip(&b.vertices) {
            worst = worst
                .max((p[0] - shift[0] - q[0]).abs())
                .max((p[1] - shift[1] - q[1]).abs());
            worst = worst.max((p[2] - q[2]).abs());
        }
    }
    println!(
        "{turned}: slicer-frame offset in the plate-aligned frame ({:.4}, {:.4}); max vertex difference {worst:.2e} mm",
        shift[0], shift[1]
    );
}

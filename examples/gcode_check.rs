//! Runs the packer's final-slice support check on a sliced plate G-code and prints what it
//! finds: `cargo run --release --example gcode_check -- plate_1.gcode`.
use goat_packer::gcode_collisions::analyze_plate;

fn main() {
    let path = std::env::args().nth(1).expect("usage: gcode_check PLATE.gcode");
    let gcode = std::fs::read(&path).expect("readable G-code");
    let started = std::time::Instant::now();
    let analysis = analyze_plate(&gcode);
    println!(
        "{} layers, {} labelled lines, {} objects, {:.1} s",
        analysis.layers,
        analysis.labelled_lines,
        analysis.first_layer.len(),
        started.elapsed().as_secs_f64()
    );
    for collision in &analysis.collisions {
        println!("{}", serde_json::to_string(collision).unwrap());
    }
}

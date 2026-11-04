//! Runs the GUI-equivalent checks on a sliced plate G-code (G-code frame, no exclusions) and
//! prints every finding: `cargo run --release --example gui_check -- plate_1.gcode W D [all|brim|none]`.
use goat_packer::gcode_collisions::gui_checks::{GuiRules, TowerLines, check_plate};
use goat_packer::Rect;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: gui_check PLATE.gcode W D [all|brim|none]");
    let width: f64 = args.next().map_or(256.0, |value| value.parse().expect("width"));
    let depth: f64 = args.next().map_or(256.0, |value| value.parse().expect("depth"));
    let tower_lines = match args.next().as_deref() {
        None | Some("all") => TowerLines::All,
        Some("brim") => TowerLines::BrimLoops,
        Some("none") => TowerLines::None,
        Some(other) => panic!("unknown tower lines {other}"),
    };
    let gcode = std::fs::read(&path).expect("readable G-code");
    let rules = GuiRules {
        printable: Rect::new(0.0, 0.0, width, depth),
        exclusions: Vec::new(),
        tower_lines,
    };
    let report = check_plate(&gcode, &rules);
    println!("toolpath extent {:?}", report.toolpath_extent);
    for error in report.errors() {
        println!("{error}");
    }
}

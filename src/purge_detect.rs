//! Start-G-code purge («prime/load line») from a probe slice the catalog tool produced itself:
//! extruding moves before the first layer marker, as the axis-aligned `[x0,y0,x1,y1,width]`
//! strips `PrinterProfile::purge_lines` expects. Off-bed purges (Prusa, Creality) are dropped —
//! they never collide with placements.

const LAYER_MARKERS: [&str; 3] = [";LAYER_CHANGE", "; CHANGE_LAYER", ";LAYER:0"];
const FILAMENT_AREA_MM2: f64 = std::f64::consts::PI * 0.875 * 0.875; // 1.75 mm filament

fn word(line: &str, letter: char) -> Option<f64> {
    line.split_whitespace().find_map(|w| w.strip_prefix(letter)?.parse().ok())
}

pub fn purge_lines(gcode: &str, printable: [f64; 4]) -> Vec<[f64; 5]> {
    let (mut x, mut y, mut z, mut e) = (0.0_f64, 0.0_f64, 0.2_f64, 0.0_f64);
    let (mut absolute_xyz, mut absolute_e) = (true, true);
    let mut out = Vec::new();
    for raw in gcode.lines() {
        if LAYER_MARKERS.iter().any(|m| raw.starts_with(m)) {
            break;
        }
        let line = raw.split(';').next().unwrap_or("").trim();
        let cmd = line.split_whitespace().next().unwrap_or("");
        match cmd {
            "G90" => absolute_xyz = true,
            "G91" => absolute_xyz = false,
            "M82" => absolute_e = true,
            "M83" => absolute_e = false,
            "G92" => {
                if let Some(v) = word(line, 'E') {
                    e = v;
                }
            }
            "G0" | "G1" => {
                let step = |cur: f64, v: Option<f64>| match v { Some(v) if absolute_xyz => v, Some(v) => cur + v, None => cur };
                let (nx, ny) = (step(x, word(line, 'X')), step(y, word(line, 'Y')));
                z = step(z, word(line, 'Z'));
                let de = match word(line, 'E') {
                    Some(v) if absolute_e => { let d = v - e; e = v; d }
                    Some(v) => v,
                    None => 0.0,
                };
                let len = ((nx - x).powi(2) + (ny - y).powi(2)).sqrt();
                if de > 0.0 && len > 0.5 {
                    if let Some(strip) = strip(x, y, nx, ny, de, len, z.max(0.05), printable) {
                        out.push(strip);
                    }
                }
                x = nx;
                y = ny;
            }
            _ => {}
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn strip(x0: f64, y0: f64, x1: f64, y1: f64, de: f64, len: f64, z: f64, printable: [f64; 4]) -> Option<[f64; 5]> {
    let [ax0, ay0, ax1, ay1] = printable;
    // Fully outside the printable area → cannot collide with a placement.
    if x0.max(x1) < ax0 || x0.min(x1) > ax1 || y0.max(y1) < ay0 || y0.min(y1) > ay1 {
        return None;
    }
    let width = (de * FILAMENT_AREA_MM2 / (len * z)).clamp(0.3, 3.0);
    let (cx0, cx1, cy0, cy1) = (x0.clamp(ax0, ax1), x1.clamp(ax0, ax1), y0.clamp(ay0, ay1), y1.clamp(ay0, ay1));
    let (dx, dy) = ((cx1 - cx0).abs(), (cy1 - cy0).abs());
    if dx < 1e-6 || dy < 1e-6 {
        return Some([cx0, cy0, cx1, cy1, width]);
    }
    // Diagonal: centre line along the longer span, the shorter span widens the strip.
    if dx >= dy {
        let ym = (cy0 + cy1) / 2.0;
        Some([cx0.min(cx1), ym, cx0.max(cx1), ym, width + dy])
    } else {
        let xm = (cx0 + cx1) / 2.0;
        Some([xm, cy0.min(cy1), xm, cy0.max(cy1), width + dx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P1S_LIKE: &str = "G90\nM83\nG1 X18.0 Y1.0 Z0.8 F18000\nG1 Z0.2\nG0 X240 E15 F6033\nG0 Y11 E0.700\nG0 X239.5\nG0 E0.2\nG0 Y1.5 E0.700\nG0 X18 E15\n;LAYER_CHANGE\n;Z:0.2\nG1 X100 Y100 E5\n";

    #[test]
    fn segments_before_the_first_layer_are_the_purge() {
        let lines = purge_lines(P1S_LIKE, [0.0, 0.0, 256.0, 256.0]);
        let rounded: Vec<[f64; 4]> = lines.iter().map(|l| [l[0], l[1], l[2], l[3]].map(|v| (v * 10.0).round() / 10.0)).collect();
        assert_eq!(rounded, vec![[18.0, 1.0, 240.0, 1.0], [240.0, 1.0, 240.0, 11.0], [239.5, 11.0, 239.5, 1.5], [239.5, 1.5, 18.0, 1.5]]);
        assert!(lines.iter().all(|l| l[4] >= 0.3 && l[4] <= 3.0));
    }

    #[test]
    fn off_bed_purge_is_ignored_and_relative_moves_are_tracked() {
        // Prusa-style: purge at Y-4 (off the printable area), relative XY moves via G91.
        let gcode = "G90\nM83\nG0 X0 Y-4 Z0.2\nG91\nG1 X15 E7\nG1 X10 E4\nG90\n; CHANGE_LAYER\n";
        assert!(purge_lines(gcode, [0.0, 0.0, 250.0, 210.0]).is_empty());
    }

    #[test]
    fn absolute_extrusion_and_g92_reset() {
        let gcode = "G90\nM82\nG92 E0\nG1 X10 Y5 Z0.2\nG1 X60 Y5 E2.5\nG92 E0\nG1 X60 Y15 E0.5\n;LAYER:0\n";
        let lines = purge_lines(gcode, [0.0, 0.0, 200.0, 200.0]);
        assert_eq!(lines.len(), 2);
        assert_eq!([lines[1][0], lines[1][1], lines[1][2], lines[1][3]], [60.0, 5.0, 60.0, 15.0]);
    }

    #[test]
    fn diagonal_segment_becomes_its_bounding_strip() {
        let gcode = "G90\nM83\nG1 X10 Y10 Z0.3\nG1 X20 Y12 E1\n;LAYER_CHANGE\n";
        let line = purge_lines(gcode, [0.0, 0.0, 200.0, 200.0])[0];
        // Wider of the two spans is the centre line; the narrow span becomes extra width.
        assert_eq!([line[0], line[1], line[2], line[3]], [10.0, 11.0, 20.0, 11.0]);
        assert!(line[4] >= 2.0);
    }
}

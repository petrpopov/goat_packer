//! Start-G-code purge («prime/load line») from a probe slice the catalog tool produced itself:
//! extruding moves before the first layer marker, as the axis-aligned `[x0,y0,x1,y1,width]`
//! strips `PrinterProfile::purge_lines` expects (G-code frame), each with the height it was
//! printed at. Off-bed purges (Prusa, Creality) are dropped — they never collide with
//! placements.

const LAYER_MARKERS: [&str; 3] = [";LAYER_CHANGE", "; CHANGE_LAYER", ";LAYER:0"];
/// The usual filament; a probe without a `filament_diameter` setting is assumed to use it.
pub const DEFAULT_FILAMENT_DIAMETER_MM: f64 = 1.75;
/// Moves shorter than this are primes and retract recoveries, not lines.
const MIN_LINE_MM: f64 = 0.5;
/// A purge "at z = 0" still squeezes a real line; this floor keeps the width finite.
const MIN_PURGE_Z_MM: f64 = 0.05;

/// One purge line and the height it was printed at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PurgeStrip {
    /// `[x0, y0, x1, y1, width]`, clipped to the printable area.
    pub line: [f64; 5],
    pub z_mm: f64,
}

fn word(line: &str, letter: char) -> Option<f64> {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(letter)?.parse().ok())
}

/// Purge strips of a probe G-code. The width is what the extruded volume fills, not the
/// nominal line width: w = E·A_filament / (L·h). Purges are squeezed far wider than a normal
/// line (P1S 0.81-0.89 mm, U1 1.8 mm, where the hand-made profiles assumed 0.5 mm).
pub fn purge_strips(
    gcode: &str,
    printable: [f64; 4],
    filament_diameter_mm: f64,
) -> Vec<PurgeStrip> {
    let filament_area = std::f64::consts::PI * (filament_diameter_mm / 2.0).powi(2);
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
                let step = |cur: f64, v: Option<f64>| match v {
                    Some(v) if absolute_xyz => v,
                    Some(v) => cur + v,
                    None => cur,
                };
                let (nx, ny) = (step(x, word(line, 'X')), step(y, word(line, 'Y')));
                z = step(z, word(line, 'Z'));
                let de = match word(line, 'E') {
                    Some(v) if absolute_e => {
                        let d = v - e;
                        e = v;
                        d
                    }
                    Some(v) => v,
                    None => 0.0,
                };
                let extrusion = Extrusion {
                    from: (x, y),
                    to: (nx, ny),
                    z: z.max(MIN_PURGE_Z_MM),
                    volume: de * filament_area,
                };
                if de > 0.0
                    && extrusion.length() > MIN_LINE_MM
                    && let Some(line) = extrusion.strip(printable)
                {
                    out.push(PurgeStrip {
                        line,
                        z_mm: extrusion.z,
                    });
                }
                x = nx;
                y = ny;
            }
            _ => {}
        }
    }
    out
}

/// One extruding move.
struct Extrusion {
    from: (f64, f64),
    to: (f64, f64),
    z: f64,
    /// Filament volume pushed, mm³.
    volume: f64,
}

impl Extrusion {
    fn length(&self) -> f64 {
        (self.to.0 - self.from.0).hypot(self.to.1 - self.from.1)
    }

    fn strip(&self, printable: [f64; 4]) -> Option<[f64; 5]> {
        let [ax0, ay0, ax1, ay1] = printable;
        let ((x0, y0), (x1, y1)) = (self.from, self.to);
        // Fully outside the printable area → cannot collide with a placement.
        if x0.max(x1) < ax0 || x0.min(x1) > ax1 || y0.max(y1) < ay0 || y0.min(y1) > ay1 {
            return None;
        }
        let width = (self.volume / (self.length() * self.z)).clamp(0.3, 3.0);
        let (cx0, cx1, cy0, cy1) = (
            x0.clamp(ax0, ax1),
            x1.clamp(ax0, ax1),
            y0.clamp(ay0, ay1),
            y1.clamp(ay0, ay1),
        );
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The P1S start G-code's load line (G-code frame).
    const P1S_LIKE: &str = "G90\nM83\nG1 X18.0 Y1.0 Z0.8 F18000\nG1 Z0.2\nG0 E2 F300\nG0 X240 E15 F6033\nG0 Y11 E0.700\nG0 X239.5\nG0 E0.2\nG0 Y1.5 E0.700\nG0 X18 E15\n;LAYER_CHANGE\n;Z:0.2\nG1 X100 Y100 E5\n";

    fn lines(gcode: &str, printable: [f64; 4]) -> Vec<[f64; 5]> {
        purge_strips(gcode, printable, DEFAULT_FILAMENT_DIAMETER_MM)
            .iter()
            .map(|strip| strip.line)
            .collect()
    }

    #[test]
    fn segments_before_the_first_layer_are_the_purge() {
        let strips = purge_strips(P1S_LIKE, [0.0, 0.0, 256.0, 256.0], 1.75);
        let rounded: Vec<[f64; 5]> = strips
            .iter()
            .map(|s| s.line.map(|v| (v * 100.0).round() / 100.0))
            .collect();
        // Widths from the extruded volume, E·π·0.875² / (L·0.2): E 15 over 222 mm is 0.81,
        // the E 0.7 legs over 10 and 9.5 mm are 0.84 and 0.89.
        assert_eq!(
            rounded,
            vec![
                [18.0, 1.0, 240.0, 1.0, 0.81],
                [240.0, 1.0, 240.0, 11.0, 0.84],
                [239.5, 11.0, 239.5, 1.5, 0.89],
                [239.5, 1.5, 18.0, 1.5, 0.81]
            ]
        );
        assert!(strips.iter().all(|s| s.z_mm == 0.2));
    }

    #[test]
    fn width_follows_the_filament_area() {
        let thin = purge_strips(P1S_LIKE, [0.0, 0.0, 256.0, 256.0], 1.75);
        let thick = purge_strips(P1S_LIKE, [0.0, 0.0, 256.0, 256.0], 2.85);
        let ratio = thick[0].line[4] / thin[0].line[4];
        assert!((ratio - (2.85_f64 / 1.75).powi(2)).abs() < 1e-9, "{ratio}");
    }

    #[test]
    fn off_bed_purge_is_ignored_and_relative_moves_are_tracked() {
        // Prusa-style: purge at Y-4 (off the printable area), relative XY moves via G91.
        let gcode = "G90\nM83\nG0 X0 Y-4 Z0.2\nG91\nG1 X15 E7\nG1 X10 E4\nG90\n; CHANGE_LAYER\n";
        assert!(lines(gcode, [0.0, 0.0, 250.0, 210.0]).is_empty());
    }

    #[test]
    fn absolute_extrusion_and_g92_reset() {
        let gcode =
            "G90\nM82\nG92 E0\nG1 X10 Y5 Z0.2\nG1 X60 Y5 E2.5\nG92 E0\nG1 X60 Y15 E0.5\n;LAYER:0\n";
        let lines = lines(gcode, [0.0, 0.0, 200.0, 200.0]);
        assert_eq!(lines.len(), 2);
        assert_eq!(
            [lines[1][0], lines[1][1], lines[1][2], lines[1][3]],
            [60.0, 5.0, 60.0, 15.0]
        );
    }

    #[test]
    fn diagonal_segment_becomes_its_bounding_strip() {
        let gcode = "G90\nM83\nG1 X10 Y10 Z0.3\nG1 X20 Y12 E1\n;LAYER_CHANGE\n";
        let strip = purge_strips(gcode, [0.0, 0.0, 200.0, 200.0], 1.75)[0];
        // Wider of the two spans is the centre line; the narrow span becomes extra width.
        let line = strip.line;
        assert_eq!(
            [line[0], line[1], line[2], line[3]],
            [10.0, 11.0, 20.0, 11.0]
        );
        assert!(line[4] >= 2.0);
        assert_eq!(strip.z_mm, 0.3);
    }
}

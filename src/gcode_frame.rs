//! G-code coordinates vs plate coordinates.
//!
//! BambuStudio writes G-code in plate coordinates minus the project's `extruder_offset`
//! (P1S: `["0x2"]`): a goat whose mesh is centred at (128, 128) on the plate prints its walls
//! centred at (128, 126) in the G-code. Its G-code viewer adds the offset back
//! (`GCodeProcessor::store_move_vertex`) and checks the result against the printable area —
//! that check rejected a packed P1S plate whose supports reached y = 254.76 in the G-code
//! (256.76 on the plate: "A G-code path goes beyond plate boundaries"), while a hand-made
//! plate reaching y = −1.51 in its G-code (0.49 on the plate) passed. Everything the packer
//! learns from a slice (first-layer patch, supports by height, tower) has to be read in plate
//! coordinates, the frame of the meshes, the bed and the build-item transforms.

/// The project's `extruder_offset` (`"XxY"` entries, one per extruder). `(0, 0)` when the
/// setting is absent. Different offsets per extruder are refused: one shift can't represent
/// them.
pub fn extruder_offset(settings: &serde_json::Value) -> Result<(f64, f64), String> {
    let Some(entries) = settings.get("extruder_offset") else {
        return Ok((0.0, 0.0));
    };
    let entries: Vec<&serde_json::Value> = match entries {
        serde_json::Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    let mut offset = None;
    for entry in entries {
        let text = entry
            .as_str()
            .ok_or_else(|| format!("extruder_offset entry is not a string: {entry}"))?;
        let (x, y) = text
            .split_once('x')
            .ok_or_else(|| format!("extruder_offset entry {text:?} is not XxY"))?;
        let parse = |value: &str| {
            value
                .trim()
                .parse::<f64>()
                .map_err(|error| format!("extruder_offset entry {text:?}: {error}"))
        };
        let parsed = (parse(x)?, parse(y)?);
        match offset {
            None => offset = Some(parsed),
            Some(first) if first != parsed => {
                return Err(format!(
                    "extruder_offset differs per extruder ({first:?} vs {parsed:?}); the packer \
                     reads G-code with one offset"
                ));
            }
            Some(_) => {}
        }
    }
    Ok(offset.unwrap_or((0.0, 0.0)))
}

fn format_coordinate(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_owned() } else { text.to_owned() }
}

/// Rewrites the absolute X/Y words of `G0`–`G3` and `G92` by `offset`, turning G-code
/// coordinates into plate coordinates. Relative moves (`G91`) and arc centres (`I`/`J`, relative
/// to the start point) stay as they are, comments too. A zero offset returns the input as is.
pub fn gcode_to_plate_frame(gcode: &[u8], offset: (f64, f64)) -> Vec<u8> {
    if offset == (0.0, 0.0) {
        return gcode.to_vec();
    }
    let mut absolute = true;
    let mut out = Vec::with_capacity(gcode.len() + gcode.len() / 50);
    for line in gcode.split_inclusive(|&byte| byte == b'\n') {
        let (code, rest) = match line.iter().position(|&byte| byte == b';') {
            Some(index) => line.split_at(index),
            None => {
                let end = line.len() - usize::from(line.ends_with(b"\n"));
                line.split_at(end)
            }
        };
        let Ok(code_text) = std::str::from_utf8(code) else {
            out.extend_from_slice(line);
            continue;
        };
        let mut words = code_text.split_whitespace();
        let command = words.next().unwrap_or("");
        match command {
            "G90" => absolute = true,
            "G91" => absolute = false,
            _ => {}
        }
        let moves = matches!(command, "G0" | "G1" | "G2" | "G3");
        if !(moves && absolute || command == "G92") {
            out.extend_from_slice(line);
            continue;
        }
        let mut rewritten = String::with_capacity(code_text.len() + 8);
        rewritten.push_str(command);
        for word in words {
            rewritten.push(' ');
            let shift = match word.as_bytes().first() {
                Some(b'X') => Some(offset.0),
                Some(b'Y') => Some(offset.1),
                _ => None,
            };
            match shift.and_then(|shift| word[1..].parse::<f64>().ok().map(|value| value + shift)) {
                Some(value) => {
                    rewritten.push_str(&word[..1]);
                    rewritten.push_str(&format_coordinate(value));
                }
                None => rewritten.push_str(word),
            }
        }
        // Keep the separator before a comment: `G1 X1 ; note` stays two tokens apart.
        if code_text.ends_with(char::is_whitespace) {
            rewritten.push(' ');
        }
        out.extend_from_slice(rewritten.as_bytes());
        out.extend_from_slice(rest);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p1s_offset_is_read_and_uniform_offsets_only() {
        let p1s = serde_json::json!({ "extruder_offset": ["0x2"] });
        assert_eq!(extruder_offset(&p1s).unwrap(), (0.0, 2.0));
        assert_eq!(extruder_offset(&serde_json::json!({})).unwrap(), (0.0, 0.0));
        let two = serde_json::json!({ "extruder_offset": ["0x0", "0x0"] });
        assert_eq!(extruder_offset(&two).unwrap(), (0.0, 0.0));
        let mixed = serde_json::json!({ "extruder_offset": ["0x0", "1x0"] });
        assert!(extruder_offset(&mixed).is_err());
    }

    #[test]
    fn absolute_moves_shift_relative_moves_and_arc_centres_do_not() {
        let gcode = b"G90\nG1 X10 Y20 E.5 ; wall\nG3 X188.668 Y-1.51 I5.176 J-1.234 E.36585\n\
G91\nG1 X3 F12000; move aside\nG90\nG92 E0\nM83\n";
        let shifted = String::from_utf8(gcode_to_plate_frame(gcode, (0.0, 2.0))).unwrap();
        assert_eq!(
            shifted,
            "G90\nG1 X10 Y22 E.5 ; wall\nG3 X188.668 Y0.49 I5.176 J-1.234 E.36585\n\
G91\nG1 X3 F12000; move aside\nG90\nG92 E0\nM83\n"
        );
    }

    #[test]
    fn zero_offset_is_byte_identical() {
        let gcode = b"G1 X1.50000 Y2 ; c\n";
        assert_eq!(gcode_to_plate_frame(gcode, (0.0, 0.0)), gcode.to_vec());
    }
}

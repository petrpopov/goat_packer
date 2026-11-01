//! The printable area and exclusion polygons of a project, read for the preview drawing.

use super::archive::PROJECT_SETTINGS_ENTRY;

pub(super) type BedPolygon = Vec<(f64, f64)>;

pub(super) fn parse_bed_geometry(
    settings_bytes: &[u8],
) -> Result<(BedPolygon, Vec<BedPolygon>), String> {
    let settings: serde_json::Value = serde_json::from_slice(settings_bytes)
        .map_err(|error| format!("{PROJECT_SETTINGS_ENTRY} is not valid JSON: {error}"))?;
    let printable_area = settings
        .get("printable_area")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("{PROJECT_SETTINGS_ENTRY} has no printable_area"))?;
    let printable = parse_xy_polygon(printable_area)?;
    // The Python collects one exclusion polygon from bed_exclude_area, drawing it only when a
    // polygon shape (>= 3 points) can be formed at all.
    let mut exclusions = Vec::new();
    if let Some(raw) = settings
        .get("bed_exclude_area")
        .and_then(serde_json::Value::as_array)
        && raw.len() >= 3
    {
        exclusions.push(parse_xy_polygon(raw)?);
    }
    Ok((printable, exclusions))
}

/// Parses `"XxY"` strings (`"200x0"`) into bed points, as `_parse_xy_polygon` does.
fn parse_xy_polygon(values: &[serde_json::Value]) -> Result<Vec<(f64, f64)>, String> {
    let mut polygon = Vec::with_capacity(values.len());
    for value in values {
        let Some(text) = value.as_str() else {
            return Err(format!("printable-area entry is not a string: {value}"));
        };
        let parse = |part: &str| {
            part.trim()
                .parse::<f64>()
                .map_err(|_| format!("printable-area entry is not in \"XxY\" form: {text:?}"))
        };
        let Some((x, y)) = text.split_once('x') else {
            return Err(format!(
                "printable-area entry is not in \"XxY\" form: {text:?}"
            ));
        };
        polygon.push((parse(x)?, parse(y)?));
    }
    Ok(polygon)
}

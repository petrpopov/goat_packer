//! The metadata parts of a project: model settings, layer heights, cut information and the
//! plate JSON, each rewritten for the packed clones.

use quick_xml::events::{BytesEnd, BytesStart, Event};

use super::archive::{CUT_INFORMATION_ENTRY, MODEL_SETTINGS_ENTRY, PLATE_JSON_ENTRY};
use super::format::{clone_object_id, number, stable_uuid};
use super::xml::{child_spans, element_is, parse_events, root_span, set_attribute, write_events};
use crate::{PackedPlacement, Point, Rect, polygon_area};

pub(super) fn rebuild_model_settings(
    data: &[u8],
    placements: &[PackedPlacement],
    z_offset: f64,
) -> Result<Vec<u8>, String> {
    let events = parse_events(data, MODEL_SETTINGS_ENTRY)?;
    let (root_start, root_end) = root_span(&events)?;
    let children = child_spans(&events, root_start, root_end);
    let template_span = children
        .iter()
        .copied()
        .find(|&span| element_is(&events[span.0], "object"))
        .ok_or_else(|| "model_settings has no object template".to_owned())?;
    if !children
        .iter()
        .copied()
        .any(|span| element_is(&events[span.0], "plate"))
    {
        return Err("model_settings has no plate".to_owned());
    }

    let mut rebuilt: Vec<Event<'static>> = events[..=root_start].to_vec();
    let mut child_ordinal = 0usize;
    let mut index = root_start + 1;
    while index < root_end {
        if children.get(child_ordinal).map(|span| span.0) == Some(index) {
            let span = children[child_ordinal];
            if element_is(&events[span.0], "object") {
                if span == template_span {
                    for object_index in 0..placements.len() {
                        let mut clone = events[template_span.0..template_span.1].to_vec();
                        rewrite_settings_object(&mut clone, object_index)?;
                        rebuilt.extend(clone);
                    }
                }
                // Every other template object is dropped; the clones take the template's
                // original position, keeping the surrounding children in order.
            } else if element_is(&events[span.0], "plate") {
                rebuilt.extend(rebuild_plate_events(&events, span, placements)?);
            } else if element_is(&events[span.0], "assemble") {
                rebuilt.extend(rebuild_assemble_events(
                    &events, span, placements, z_offset,
                )?);
            } else {
                rebuilt.extend(events[span.0..span.1].iter().cloned());
            }
            index = span.1;
            child_ordinal += 1;
        } else {
            rebuilt.push(events[index].clone());
            index += 1;
        }
    }
    rebuilt.extend(events[root_end..].iter().cloned());
    write_events(&rebuilt)
}

/// Duplicates the template's `object_id=<n>|<table>` lines across every packed clone.
/// The `object_id` here is the 1-based ordinal of the object in the model's object list —
/// OrcaSlicer indexes this file by `object index + 1` (`bbs_3mf.cpp`), not by the 3MF
/// object id used elsewhere in the archive. Lines without that prefix pass through.
pub(super) fn rebuild_layer_heights_profile(data: &[u8], clone_count: usize) -> Vec<u8> {
    let text = String::from_utf8_lossy(data);
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        match line
            .split_once("object_id=")
            .and_then(|(_, rest)| rest.split_once('|'))
        {
            Some((_, table)) => {
                for ordinal in 1..=clone_count {
                    lines.push(format!("object_id={ordinal}|{table}"));
                }
            }
            None => lines.push(line.to_owned()),
        }
    }
    let mut out = lines.join("\n").into_bytes();
    out.push(b'\n');
    out
}

/// Sets the clone's `id` and gives every direct `part` a deterministic uuid — BambuStudio
/// matches model_settings objects to core resources by these ids, so the object id numbering
/// must stay exactly `index + 2` (`clone_object_id`).
fn rewrite_settings_object(clone: &mut [Event<'static>], index: usize) -> Result<(), String> {
    if let Event::Start(start) | Event::Empty(start) = &mut clone[0] {
        set_attribute(start, "id", &clone_object_id(index).to_string())?;
    }
    if matches!(clone[0], Event::Start(_)) {
        let part_spans: Vec<(usize, usize)> = child_spans(clone, 0, clone.len())
            .into_iter()
            .filter(|&span| element_is(&clone[span.0], "part"))
            .collect();
        for (part_index, (part_lo, _)) in part_spans.into_iter().enumerate() {
            if let Event::Start(start) | Event::Empty(start) = &mut clone[part_lo] {
                set_attribute(
                    start,
                    "uuid",
                    &stable_uuid(&format!("part-{part_index}"), index),
                )?;
            }
        }
    }
    Ok(())
}

/// The opening event of the element at `events[lo]`, for a rewrite that adds children: a
/// self-closed `<name/>` becomes `<name>`, and the closing event to append is returned with it
/// (`None` when the element has its own End event).
fn open_element(
    events: &[Event<'static>],
    lo: usize,
    name: &'static str,
) -> (Event<'static>, Option<Event<'static>>) {
    match events[lo].clone() {
        Event::Empty(start) => (Event::Start(start), Some(Event::End(BytesEnd::new(name)))),
        open @ Event::Start(_) => (open, None),
        _ => unreachable!("{name} span starts with an element"),
    }
}

/// Replaces the plate's `model_instance` children with one per placement (the per-instance
/// metadata BambuStudio reads to map instances back to model_settings objects).
fn rebuild_plate_events(
    events: &[Event<'static>],
    span: (usize, usize),
    placements: &[PackedPlacement],
) -> Result<Vec<Event<'static>>, String> {
    let (lo, hi) = span;
    let (open, appended_end) = open_element(events, lo, "plate");
    let mut out: Vec<Event<'static>> = vec![open];
    // The interior copy must stop BEFORE the plate's own End event (index hi - 1 for a
    // Start..End pair): including it closed the plate early, made the new instances siblings
    // of <plate>, and appended a second `</plate>` — BambuStudio rejects that ("don't find
    // plate created before"). A self-closed `<plate/>` has no interior at all.
    let interior_end = appended_end.as_ref().map_or(hi - 1, |_| lo + 1);
    let instance_spans: Vec<(usize, usize)> = child_spans(events, lo, hi)
        .into_iter()
        .filter(|&instance_span| element_is(&events[instance_span.0], "model_instance"))
        .collect();
    let mut cursor = lo + 1;
    for (instance_lo, instance_hi) in instance_spans {
        out.extend(events[cursor..instance_lo].iter().cloned());
        cursor = instance_hi;
    }
    out.extend(events[cursor..interior_end].iter().cloned());
    for index in 0..placements.len() {
        out.extend(model_instance_events(index));
    }
    out.push(appended_end.unwrap_or_else(|| events[hi - 1].clone()));
    Ok(out)
}

fn model_instance_events(index: usize) -> Vec<Event<'static>> {
    let mut events = vec![Event::Start(BytesStart::new("model_instance"))];
    for (key, value) in [
        ("object_id", clone_object_id(index).to_string()),
        ("instance_id", "0".to_owned()),
        ("identify_id", (1000 + index * 10).to_string()),
    ] {
        let mut metadata = BytesStart::new("metadata");
        metadata.push_attribute(("key", key));
        metadata.push_attribute(("value", value.as_str()));
        events.push(Event::Empty(metadata));
    }
    events.push(Event::End(BytesEnd::new("model_instance")));
    events
}

/// Replaces the `assemble` element's children with one `assemble_item` per placement; the
/// item z translation reuses the source z offset so the assembled objects stay on the bed.
fn rebuild_assemble_events(
    events: &[Event<'static>],
    span: (usize, usize),
    placements: &[PackedPlacement],
    z_offset: f64,
) -> Result<Vec<Event<'static>>, String> {
    let (lo, hi) = span;
    let (open, appended_end) = open_element(events, lo, "assemble");
    let mut out: Vec<Event<'static>> = vec![open];
    let transform = format!("1 0 0 0 1 0 0 0 1 0 0 {}", number(z_offset));
    for index in 0..placements.len() {
        let object_id = clone_object_id(index).to_string();
        let mut item = BytesStart::new("assemble_item");
        item.push_attribute(("object_id", object_id.as_str()));
        item.push_attribute(("instance_id", "0"));
        item.push_attribute(("transform", transform.as_str()));
        item.push_attribute(("offset", "0 0 0"));
        out.push(Event::Empty(item));
    }
    out.push(appended_end.unwrap_or_else(|| events[hi - 1].clone()));
    Ok(out)
}

/// Rewrites cut_information.xml to N objects (`id = index + 1`) — one per packed object, each
/// with the empty cut metadata BambuStudio expects on pre-cut projects.
pub(super) fn rebuild_cut_information(data: &[u8], count: usize) -> Result<Vec<u8>, String> {
    let events = parse_events(data, CUT_INFORMATION_ENTRY)?;
    let (root_start, root_end) = root_span(&events)?;
    let mut rebuilt: Vec<Event<'static>> = events[..=root_start].to_vec();
    for index in 0..count {
        let object_id = (index + 1).to_string();
        let mut object = BytesStart::new("object");
        object.push_attribute(("id", object_id.as_str()));
        rebuilt.push(Event::Start(object));
        let mut cut_id = BytesStart::new("cut_id");
        cut_id.push_attribute(("id", "0"));
        cut_id.push_attribute(("check_sum", "1"));
        cut_id.push_attribute(("connectors_cnt", "0"));
        rebuilt.push(Event::Empty(cut_id));
        rebuilt.push(Event::End(BytesEnd::new("object")));
    }
    // `root_span`'s end is exclusive with the root's End event at `root_end - 1`: it MUST be
    // re-emitted, or BambuStudio fails loading the archive with "(2): unexpected end of data"
    // (seen on a real Goat 3 plate whose source carried a cut_information.xml).
    rebuilt.extend(events[root_end - 1..].iter().cloned());
    write_events(&rebuilt)
}

/// Unsigned footprint area with holes subtracted — shapely's `Polygon.area` for the union of
/// the bottom faces the Python measured.
fn polygon_printed_area(rings: &[Vec<Point>]) -> f64 {
    let mut area = polygon_area(&rings[0]).abs();
    for ring in rings.iter().skip(1) {
        area -= polygon_area(ring).abs();
    }
    area
}

/// Per-polygon bounds as `[min_x, min_y, max_x, max_y]` (shapely's `bounds` tuple).
fn polygon_bounds(rings: &[Vec<Point>]) -> [f64; 4] {
    let bounds = Rect::of_points(rings.iter().flatten());
    [bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y]
}

/// Rewrites plate_1.json's `bbox_all`/`bbox_objects`/`first_layer_time` from the world-space
/// footprints, serialized compact like `json.dumps(separators=(",", ":"))`.
pub(super) fn rebuild_plate_json(
    data: &[u8],
    world_polygons: &[Vec<Vec<Point>>],
) -> Result<Vec<u8>, String> {
    let mut plate: serde_json::Value = serde_json::from_slice(data)
        .map_err(|error| format!("{PLATE_JSON_ENTRY} is not valid JSON: {error}"))?;
    let bounds: Vec<[f64; 4]> = world_polygons
        .iter()
        .map(|rings| polygon_bounds(rings))
        .collect();
    plate["bbox_all"] = serde_json::json!([
        bounds.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min),
        bounds.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min),
        bounds
            .iter()
            .map(|b| b[2])
            .fold(f64::NEG_INFINITY, f64::max),
        bounds
            .iter()
            .map(|b| b[3])
            .fold(f64::NEG_INFINITY, f64::max),
    ]);
    plate["bbox_objects"] = serde_json::Value::Array(
        world_polygons
            .iter()
            .enumerate()
            .map(|(index, rings)| {
                serde_json::json!({
                    "area": polygon_printed_area(rings),
                    "bbox": bounds[index],
                    "id": 1002 + index * 10,
                    "layer_height": 0.2,
                    // The Python hardcodes the source STL name here (it is cosmetic metadata
                    // BambuStudio shows on the plate tab); keep it hardcoded.
                    "name": "goat4_v2.stl",
                })
            })
            .collect(),
    );
    plate["first_layer_time"] = serde_json::json!(0.0);
    serde_json::to_vec(&plate)
        .map_err(|error| format!("cannot serialize {PLATE_JSON_ENTRY}: {error}"))
}

//! `3D/3dmodel.model`: one object per packed clone and one build item per placement.

use quick_xml::events::{BytesEnd, BytesStart, Event};

use super::archive::CORE_MODEL_ENTRY;
use super::format::{clone_object_id, number, stable_uuid};
use super::xml::{
    attribute_value, child_spans, element_is, parse_events, root_span, set_attribute, write_events,
};
use crate::PackedPlacement;

/// Builds the build-item `transform` attribute for one placement.
///
/// A placement's (x, y) is where the STL mesh's XY bbox center must land in bed coordinates.
/// The build item, however, rotates the object's *local* geometry (embedded mesh plus the
/// source component's own transform — see `embedded_mesh`) about the local origin, so the
/// local footprint center has to be rotated out of the translation. Without this, a source
/// component carrying its own translation (Goat 3: (-35.38, +40.12)) displaces every object by
/// a rotation-dependent ~54 mm and objects fall off the heated bed.
pub(super) fn item_transform(
    placement: &PackedPlacement,
    z_offset: f64,
    footprint_center: (f64, f64),
) -> String {
    let angle = placement.angle_deg.to_radians();
    let (s, c) = angle.sin_cos();
    let (center_x, center_y) = footprint_center;
    let translation = [
        placement.x_mm - (center_x * c - center_y * s),
        placement.y_mm - (center_x * s + center_y * c),
        z_offset,
    ];
    let values = [c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0];
    values
        .iter()
        .chain(translation.iter())
        .map(|value| number(*value))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where the parts of the core model sit in its event list.
struct CoreLayout {
    resources_start: usize,
    /// Index of the `</resources>` event.
    resources_end: usize,
    build_start: usize,
    /// Index of `</build>`, `None` for a self-closed `<build/>`.
    build_end: Option<usize>,
    /// Direct `<object>` children of `<resources>`.
    object_spans: Vec<(usize, usize)>,
}

fn locate_core_layout(events: &[Event<'static>]) -> Result<CoreLayout, String> {
    let missing = || "3MF core model lacks resources/build".to_owned();
    let (root_start, root_end) = root_span(events)?;
    let children = child_spans(events, root_start, root_end);
    let resources = children
        .iter()
        .copied()
        .find(|&span| element_is(&events[span.0], "resources"))
        .ok_or_else(missing)?;
    let build = children
        .iter()
        .copied()
        .find(|&span| element_is(&events[span.0], "build"))
        .ok_or_else(missing)?;
    let build_end = match &events[build.0] {
        Event::Empty(_) => None,
        _ => Some(build.1 - 1),
    };
    let object_spans: Vec<(usize, usize)> = child_spans(events, resources.0, resources.1)
        .into_iter()
        .filter(|&span| element_is(&events[span.0], "object"))
        .collect();
    if object_spans.is_empty() {
        return Err("3MF core model has no source object".to_owned());
    }
    Ok(CoreLayout {
        resources_start: resources.0,
        resources_end: resources.1 - 1,
        build_start: build.0,
        build_end,
        object_spans,
    })
}

/// The 12 numbers of the first build item's transform in the source core model.
fn source_item_transform(core: &[u8]) -> Result<Vec<f64>, String> {
    let missing = || "3MF source build item has no transform".to_owned();
    let events = parse_events(core, CORE_MODEL_ENTRY)?;
    let (root_start, root_end) = root_span(&events)?;
    let build = child_spans(&events, root_start, root_end)
        .into_iter()
        .find(|&span| element_is(&events[span.0], "build"))
        .ok_or_else(missing)?;
    let item = child_spans(&events, build.0, build.1)
        .into_iter()
        .find(|&span| element_is(&events[span.0], "item"))
        .ok_or_else(missing)?;
    let (Event::Start(start) | Event::Empty(start)) = &events[item.0] else {
        unreachable!("item span starts with an element");
    };
    let transform = attribute_value(start, "transform")?.ok_or_else(missing)?;
    let values: Vec<f64> = transform
        .split_whitespace()
        .map(|token| {
            token
                .parse::<f64>()
                .map_err(|_| "3MF source transform is not a 3x4 matrix".to_owned())
        })
        .collect::<Result<_, _>>()?;
    if values.len() != 12 {
        return Err("3MF source transform is not a 3x4 matrix".to_owned());
    }
    Ok(values)
}

/// The source build item's transform applied to a mesh-bbox center: where the model's
/// geometric center actually sits on the bed in the source (and therefore probe) slice.
/// A probe-derived first-layer patch must be centered on this point — the assembler places
/// clones by that same center, and centering the patch on the contact strip instead shifts
/// the whole reserved area when the model is not prism-shaped (a lying animal's strip center
/// sits well below its bbox center).
pub fn source_model_center(core: &[u8], center: (f64, f64, f64)) -> Result<(f64, f64), String> {
    let values = source_item_transform(core)?;
    // 3MF matrices are column-major 3x4: [m00 m10 m20 m01 m11 m21 m02 m12 m22 tx ty tz].
    let (x, y, z) = center;
    Ok((
        values[0] * x + values[3] * y + values[6] * z + values[9],
        values[1] * x + values[4] * y + values[7] * z + values[10],
    ))
}

/// The source build item's z translation (transform value 11): the on-bed height the clones
/// keep.
pub(super) fn source_z_offset(core: &[u8]) -> Result<f64, String> {
    Ok(source_item_transform(core)?[11])
}

/// How the core model is rebuilt for the packed clones.
pub(super) struct CoreRebuild<'a> {
    pub placements: &'a [PackedPlacement],
    /// Per-clone mesh part paths (`duplicate_mesh_per_object`).
    pub component_paths: Option<&'a [String]>,
    /// The mesh object every clone carries inline (`inline_colored_mesh_objects`).
    pub inline_mesh_template: Option<&'a [Event<'static>]>,
    pub reuse_source_object_instances: bool,
    pub z_offset: f64,
    pub footprint_center: (f64, f64),
}

impl CoreRebuild<'_> {
    pub(super) fn apply(&self, core: &[u8]) -> Result<Vec<u8>, String> {
        let events = parse_events(core, CORE_MODEL_ENTRY)?;
        let layout = locate_core_layout(&events)?;
        let template_span = layout.object_spans[0];
        let source_object_id = match &events[template_span.0] {
            Event::Start(start) | Event::Empty(start) => attribute_value(start, "id")?
                .ok_or_else(|| "template object has no id".to_owned())?,
            _ => unreachable!("object span starts with an element"),
        };

        let mut rebuilt: Vec<Event<'static>> = events[..=layout.resources_start].to_vec();
        self.push_resources(&mut rebuilt, &events, template_span)?;
        rebuilt.extend(
            events[layout.resources_end..layout.build_start]
                .iter()
                .cloned(),
        );
        self.push_build(&mut rebuilt, &events, &layout, &source_object_id);
        write_events(&rebuilt)
    }

    /// The template's own resources are dropped (reuse mode keeps only the first object, in
    /// its original position); every packed object gets a fresh clone with deterministic
    /// identities so BambuStudio treats the clones as distinct painted objects.
    fn push_resources(
        &self,
        rebuilt: &mut Vec<Event<'static>>,
        events: &[Event<'static>],
        template_span: (usize, usize),
    ) -> Result<(), String> {
        if self.reuse_source_object_instances {
            rebuilt.extend(events[template_span.0..template_span.1].iter().cloned());
            return Ok(());
        }
        for index in 0..self.placements.len() {
            rebuilt.extend(self.clone_object(events, template_span, index)?);
        }
        Ok(())
    }

    fn clone_object(
        &self,
        events: &[Event<'static>],
        template_span: (usize, usize),
        index: usize,
    ) -> Result<Vec<Event<'static>>, String> {
        let inline = self.inline_mesh_template.is_some();
        let mut object_events: Vec<Event<'static>> = match self.inline_mesh_template {
            Some(template) => template.to_vec(),
            None => events[template_span.0..template_span.1].to_vec(),
        };
        if let Event::Start(start) | Event::Empty(start) = &mut object_events[0] {
            set_attribute(start, "id", &clone_object_id(index).to_string())?;
            let kind = if inline {
                "inline-resource"
            } else {
                "resource"
            };
            set_attribute(start, "p:UUID", &stable_uuid(kind, index))?;
        }
        if !inline {
            for event in object_events.iter_mut().skip(1) {
                if element_is(event, "component") {
                    if let Event::Start(start) | Event::Empty(start) = event {
                        set_attribute(start, "p:UUID", &stable_uuid("component", index))?;
                        if let Some(paths) = self.component_paths {
                            set_attribute(start, "p:path", &paths[index])?;
                        }
                    }
                    break; // the Python `find` rewrites only the first component
                }
            }
        }
        Ok(object_events)
    }

    /// The build element with one item per placement.
    fn push_build(
        &self,
        rebuilt: &mut Vec<Event<'static>>,
        events: &[Event<'static>],
        layout: &CoreLayout,
        source_object_id: &str,
    ) {
        let items: Vec<Event<'static>> = (0..self.placements.len())
            .map(|index| {
                let object_id = if self.reuse_source_object_instances {
                    source_object_id.to_owned()
                } else {
                    clone_object_id(index).to_string()
                };
                build_item_event(
                    index,
                    &object_id,
                    &self.placements[index],
                    self.z_offset,
                    self.footprint_center,
                )
            })
            .collect();
        match &events[layout.build_start] {
            Event::Empty(_) => {
                // A self-closed `<build/>` must grow children, so it is rewritten as an open tag.
                let Event::Empty(start) = events[layout.build_start].clone() else {
                    unreachable!("build span starts with an element");
                };
                rebuilt.push(Event::Start(start));
                rebuilt.extend(items);
                rebuilt.push(Event::End(BytesEnd::new("build")));
                rebuilt.extend(events[layout.build_start + 1..].iter().cloned());
            }
            Event::Start(_) => {
                rebuilt.push(events[layout.build_start].clone());
                rebuilt.extend(items);
                rebuilt.extend(
                    events[layout.build_end.expect("`<build>` has a matching end")..]
                        .iter()
                        .cloned(),
                );
            }
            _ => unreachable!("build span starts with an element"),
        }
    }
}

fn build_item_event(
    index: usize,
    object_id: &str,
    placement: &PackedPlacement,
    z_offset: f64,
    footprint_center: (f64, f64),
) -> Event<'static> {
    let mut item = BytesStart::new("item");
    item.push_attribute(("objectid", object_id));
    item.push_attribute(("p:UUID", stable_uuid("build-item", index).as_str()));
    item.push_attribute((
        "transform",
        item_transform(placement, z_offset, footprint_center).as_str(),
    ));
    item.push_attribute(("printable", "1"));
    Event::Empty(item)
}

//! 3MF plate assembly: clones contour-packed placements into a slicer 3MF template. Ports
//! `build_3mf.py`; contracts in
//! `docs/superpowers/specs/2026-09-01-eliminate-python-rust-port-design.md` (Task 4).
//!
//! The XML parts are rewritten with a quick-xml event round-trip: parse into owned events,
//! touch only the spans that must change, and write everything else back verbatim. quick-xml
//! keeps raw bytes, so untouched regions round-trip with their attribute order, quoting and
//! namespace prefixes intact — the reason it was chosen over xmltree, which structurally
//! cannot represent prefixed attributes (`p:UUID`, `p:path`).

use std::{
    borrow::Cow,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use quick_xml::{
    Reader, Writer,
    events::{
        BytesEnd, BytesStart, Event,
        attributes::Attribute,
    },
    name::QName,
};
use sha1::{Digest, Sha1};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{
    Footprint, PackedPlacement, Point, Rect, embedded_mesh, extract_footprint, plate_preview,
    polygon_area,
};

const CORE_MODEL_ENTRY: &str = "3D/3dmodel.model";
const MODEL_SETTINGS_ENTRY: &str = "Metadata/model_settings.config";
const PROJECT_SETTINGS_ENTRY: &str = "Metadata/project_settings.config";
const CUT_INFORMATION_ENTRY: &str = "Metadata/cut_information.xml";
const PLATE_JSON_ENTRY: &str = "Metadata/plate_1.json";
const RELATIONSHIPS_ENTRY: &str = "3D/_rels/3dmodel.model.rels";
const LAYER_HEIGHTS_ENTRY: &str = "Metadata/layer_heights_profile.txt";

/// The packed clones take object ids 2, 3, … — the source's resources keep id 1 and each
/// clone follows. `rebuild_core`, `rewrite_settings_object` and the layer-heights rewrite all
/// derive their ids from this so the metadata and the model can never disagree.
fn clone_object_id(index: usize) -> usize {
    index + 2
}

/// Everything the assembler needs besides the template itself.
#[derive(Clone, Debug)]
pub struct AssembleOptions {
    pub placements: Vec<PackedPlacement>,
    /// The collision STL: the source 3MF's embedded mesh with the component transform baked
    /// in (see `embedded_mesh`). The footprint and the compensation center are both derived
    /// from this exact file — a differently-exported STL would shift every object.
    pub stl_path: PathBuf,
    pub edge_margin: f64,
    /// `(x0, y0, x1, y1, width)` purge-line segments, drawn on the previews.
    pub purge_lines: Vec<[f64; 5]>,
    /// Keepout rectangles (prime tower, purge-line keepouts), drawn on the previews.
    pub keepouts: Vec<Rect>,
    /// Mesh strategies (mutually exclusive, spec contract 5). The packer binary uses the
    /// default mode only; the others exist for tests and manual recovery paths.
    pub duplicate_mesh_per_object: bool,
    pub reuse_source_object_instances: bool,
    pub inline_colored_mesh_objects: bool,
    /// The footprint the placements were packed against, when it differs from the mesh
    /// projection (the probe slice's first layer — body plus supports plus brim). The preview
    /// polygons must show what was packed, or the plate thumbnails lie about collisions.
    /// `None` derives the projection from `stl_path` as before.
    pub footprint_override: Option<Footprint>,
}

/// Clones `options.placements` into `template` and writes the packed project to `output`.
pub fn assemble_project(
    template: &Path,
    output: &Path,
    options: &AssembleOptions,
) -> Result<(), String> {
    if options.placements.is_empty() {
        return Err("placement list is empty".to_owned());
    }
    let mut entries = read_template_entries(template)?;

    // Contract 1: project_settings.config is read for the bed geometry only and copied
    // through byte-for-byte — it is never re-serialized by the assembler.
    let settings = entry_bytes(&entries, PROJECT_SETTINGS_ENTRY)?;
    let (printable, exclusions) = parse_bed_geometry(settings)?;

    let core = entry_bytes(&entries, CORE_MODEL_ENTRY)?.to_vec();
    // Contract 3: the collision STL is the embedded mesh with the component's transform baked
    // in, so its XY bbox center is exactly the point every build-item translation must
    // compensate (`placement − R(angle)·c_stl`). When the caller packed against the probe
    // slice's first layer (supports and brim included), that patch replaces the projection —
    // and its loops are already centered on the same bbox center, so the placement math is
    // unchanged.
    let footprint = match &options.footprint_override {
        Some(footprint) => footprint.clone(),
        None => extract_footprint(&options.stl_path).map_err(|error| {
            format!("cannot extract the footprint from {:?}: {error}", options.stl_path)
        })?,
    };
    // Canonical rings keep the assembled bytes deterministic across runs (spec acceptance 6).
    let loops: Vec<Vec<Point>> = footprint.loops.iter().map(|ring| canonical_ring(ring)).collect();
    let footprint_center = embedded_mesh::stl_xy_center(&options.stl_path)
        .map_err(|error| format!("cannot read {:?}: {error}", options.stl_path))?;
    // Contract 3: the source item's z translation becomes the clones' item z offset, keeping
    // the packed objects at the same on-bed height the source project sliced at.
    let z_offset = source_z_offset(&core)?;

    // Mirrors `_placed_polygons`: shapely `rotate(origin=(0, 0))` turns the centered footprint
    // counter-clockwise about the local origin, then `translate` moves it to the bed position.
    let world_polygons: Vec<Vec<Vec<Point>>> = options
        .placements
        .iter()
        .map(|placement| rotate_and_translate(&loops, placement))
        .collect();

    let component_paths = if options.duplicate_mesh_per_object {
        Some(duplicate_colored_mesh_parts(
            &mut entries,
            &core,
            options.placements.len(),
        )?)
    } else {
        None
    };
    let inline_mesh_template = if options.inline_colored_mesh_objects {
        Some(load_inline_mesh_template(&entries, &core)?)
    } else {
        None
    };

    set_entry(
        &mut entries,
        CORE_MODEL_ENTRY,
        rebuild_core(
            &core,
            &options.placements,
            component_paths.as_deref(),
            inline_mesh_template.as_deref(),
            options.reuse_source_object_instances,
            z_offset,
            footprint_center,
        )?,
    );
    if !options.reuse_source_object_instances {
        let model_settings = entry_bytes(&entries, MODEL_SETTINGS_ENTRY)?;
        let rebuilt = rebuild_model_settings(
            model_settings,
            &options.placements,
            z_offset,
        )?;
        set_entry(&mut entries, MODEL_SETTINGS_ENTRY, rebuilt);
    }
    // OrcaSlicer refuses to slice a prime tower "unless all objects have the same variable
    // layer height": the copied entry still points only at the source's ordinal, so every
    // clone slices without the source's layer-height table. Re-point the table at every
    // clone's ordinal so all objects carry identical variable heights again.
    if let Some(profile) = entry(&entries, LAYER_HEIGHTS_ENTRY)
        && !options.reuse_source_object_instances
    {
        let rebuilt = rebuild_layer_heights_profile(profile, options.placements.len());
        set_entry(&mut entries, LAYER_HEIGHTS_ENTRY, rebuilt);
    }
    // Contract 6: cut_information.xml and plate_1.json are rewritten only when the template
    // already carries them — Snapmaker projects lack them and Bambu-only metadata must never
    // be invented.
    if let Some(cut) = entry(&entries, CUT_INFORMATION_ENTRY)
        && !options.reuse_source_object_instances
    {
        let rebuilt = rebuild_cut_information(cut, options.placements.len())?;
        set_entry(&mut entries, CUT_INFORMATION_ENTRY, rebuilt);
    }
    if let Some(plate) = entry(&entries, PLATE_JSON_ENTRY)
        && !options.reuse_source_object_instances
    {
        let rebuilt = rebuild_plate_json(plate, &world_polygons)?;
        set_entry(&mut entries, PLATE_JSON_ENTRY, rebuilt);
    }

    // Contract 7: previews are cosmetic. The Python drew each layout through PIL; here each
    // size/pick combination renders once and feeds the entries BambuStudio expects.
    let purge_line_tuples: Vec<(f64, f64, f64, f64, f64)> = options
        .purge_lines
        .iter()
        .map(|line| (line[0], line[1], line[2], line[3], line[4]))
        .collect();
    let keepout_tuples: Vec<(f64, f64, f64, f64)> = options
        .keepouts
        .iter()
        .map(|rect| (rect.min_x, rect.min_y, rect.max_x, rect.max_y))
        .collect();
    let preview = |pick: bool| plate_preview::PreviewInput {
        polygons: &world_polygons,
        printable: &printable,
        exclusions: &exclusions,
        purge_lines: &purge_line_tuples,
        keepouts: &keepout_tuples,
        edge_margin: options.edge_margin,
        pick,
    };
    let plate_png = plate_preview::render(&preview(false), 512);
    let pick_png = plate_preview::render(&preview(true), 512);
    let small_png = plate_preview::render(&preview(false), 128);
    for name in [
        "Metadata/plate_1.png",
        "Metadata/plate_no_light_1.png",
        "Metadata/top_1.png",
    ] {
        set_entry(&mut entries, name, plate_png.clone());
    }
    set_entry(&mut entries, "Metadata/pick_1.png", pick_png);
    set_entry(&mut entries, "Metadata/plate_1_small.png", small_png);

    write_archive(output, &entries)
}

/// Formats a float exactly like the Python `_number`: near-zero collapses to "0", near-integers
/// print without a decimal point, everything else gets 10 decimals with trailing zeros (and a
/// trailing dot, were one to survive) stripped. The persisted XML/JSON must not drift from the
/// Python artifact's formatting, hence the explicit thresholds.
fn number(value: f64) -> String {
    if value.abs() < 1e-10 {
        return "0".to_owned();
    }
    let nearest = value.round();
    if (value - nearest).abs() < 1e-10 {
        let as_i64 = nearest as i64;
        if as_i64 as f64 == nearest {
            return as_i64.to_string();
        }
        // Beyond i64 the Python prints an exact big integer; unreachable for millimeter
        // coordinates, so the closest fixed-point form is an acceptable fallback.
        return format!("{nearest:.0}");
    }
    let mut text = format!("{value:.10}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

/// Deterministic `uuid5(NAMESPACE_URL, "goat4-contour-packing/{kind}/{index}")` — the same
/// identities the Python emitted via `uuid.uuid5`, so re-runs produce identical projects and
/// downstream tooling can rely on stable resource/component/build-item ids.
fn stable_uuid(kind: &str, index: usize) -> String {
    // RFC 4122's URL namespace: 6ba7b811-9dad-11d1-80b4-00c04fd430c8.
    const NAMESPACE_URL: [u8; 16] = [
        0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30,
        0xc8,
    ];
    let mut hasher = Sha1::new();
    hasher.update(NAMESPACE_URL);
    hasher.update(format!("goat4-contour-packing/{kind}/{index}"));
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50; // version 5 (name-based, SHA-1)
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8],
        bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// Builds the build-item `transform` attribute for one placement.
///
/// A placement's (x, y) is where the STL mesh's XY bbox center must land in bed coordinates.
/// The build item, however, rotates the object's *local* geometry (embedded mesh plus the
/// source component's own transform — see `embedded_mesh`) about the local origin, so the
/// local footprint center has to be rotated out of the translation. Without this, a source
/// component carrying its own translation (Goat 3: (-35.38, +40.12)) displaces every object by
/// a rotation-dependent ~54 mm and objects fall off the heated bed.
fn item_transform(
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

/// Canonicalizes one extracted boundary ring: it starts at the lexicographically smallest
/// vertex and runs counter-clockwise. `extract_footprint` walks the boundary through
/// hash-ordered adjacency, so the same STL can yield a ring with a different start vertex or
/// direction on every call — harmless for packing, but the previews (and therefore the output
/// bytes, spec acceptance 6: byte-identical projects on re-run) must not vary.
fn canonical_ring(ring: &[Point]) -> Vec<Point> {
    let mut closed = ring.to_vec();
    if closed.last().is_some_and(|last| *last == closed[0]) {
        closed.pop(); // the explicit closing point is re-appended after rotation
    }
    if closed.len() < 3 {
        return ring.to_vec();
    }
    // Direction first, then the start vertex: both orders must agree for the canonical form
    // to be unique.
    if polygon_area(&closed) < 0.0 {
        closed.reverse();
    }
    let pivot = closed
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.x.total_cmp(&b.1.x).then(a.1.y.total_cmp(&b.1.y)))
        .map(|(index, _)| index)
        .expect("non-empty ring");
    let mut rotated = closed[pivot..].to_vec();
    rotated.extend_from_slice(&closed[..pivot]);
    rotated.push(rotated[0]);
    rotated
}

/// Rotates the centered footprint loops counter-clockwise by the placement angle about the
/// origin, then translates them into bed coordinates (rotate first, then translate — the same
/// operation order as the shapely calls, so bounding boxes stay float-identical).
fn rotate_and_translate(loops: &[Vec<Point>], placement: &PackedPlacement) -> Vec<Vec<Point>> {    let (s, c) = placement.angle_deg.to_radians().sin_cos();
    loops
        .iter()
        .map(|loop_| {
            loop_
                .iter()
                .map(|point| {
                    let rotated_x = point.x * c - point.y * s;
                    let rotated_y = point.x * s + point.y * c;
                    Point {
                        x: placement.x_mm + rotated_x,
                        y: placement.y_mm + rotated_y,
                    }
                })
                .collect()
        })
        .collect()
}

type Entries = Vec<(String, Vec<u8>)>;

fn entry_index(entries: &Entries, name: &str) -> Option<usize> {
    entries.iter().position(|(entry_name, _)| entry_name == name)
}

fn entry<'a>(entries: &'a Entries, name: &str) -> Option<&'a [u8]> {
    entry_index(entries, name).map(|index| entries[index].1.as_slice())
}

fn entry_bytes<'a>(entries: &'a Entries, name: &str) -> Result<&'a [u8], String> {
    entry(entries, name).ok_or_else(|| format!("template does not contain {name:?}"))
}

/// Replaces an entry in place — preserving the template's entry order, which the zip writer
/// and downstream diffing rely on — or appends it when the template lacks it.
fn set_entry(entries: &mut Entries, name: &str, bytes: Vec<u8>) {
    match entry_index(entries, name) {
        Some(index) => entries[index].1 = bytes,
        None => entries.push((name.to_owned(), bytes)),
    }
}

/// Reads every template entry in archive order. The rewrite must keep that order (contract in
/// the spec: the level is cosmetic, the order is not — new entries are appended last).
fn read_template_entries(template: &Path) -> Result<Entries, String> {
    let mut archive = ZipArchive::new(
        File::open(template)
            .map_err(|error| format!("cannot open template {template:?}: {error}"))?,
    )
    .map_err(|error| format!("{template:?} is not a readable 3MF archive: {error}"))?;
    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("cannot read template entry {index}: {error}"))?;
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read template entry {name:?}: {error}"))?;
        entries.push((name, bytes));
    }
    Ok(entries)
}

fn write_archive(output: &Path, entries: &Entries) -> Result<(), String> {
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let mut archive =
        ZipWriter::new(File::create(output).map_err(|error| format!("cannot create {output:?}: {error}"))?);
    // Deflate level 6 matches the Python artifact's shape; the level itself is cosmetic, the
    // entry order is not (see `read_template_entries`).
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6));
    for (name, bytes) in entries {
        archive
            .start_file(name.as_str(), options)
            .map_err(|error| format!("cannot write zip entry {name:?}: {error}"))?;
        archive
            .write_all(bytes)
            .map_err(|error| format!("cannot write zip entry {name:?}: {error}"))?;
    }
    archive
        .finish()
        .map_err(|error| format!("cannot finish {output:?}: {error}"))?;
    Ok(())
}

type BedPolygon = Vec<(f64, f64)>;

fn parse_bed_geometry(settings_bytes: &[u8]) -> Result<(BedPolygon, Vec<BedPolygon>), String> {
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
            return Err(format!("printable-area entry is not in \"XxY\" form: {text:?}"));
        };
        polygon.push((parse(x)?, parse(y)?));
    }
    Ok(polygon)
}

// ---------------------------------------------------------------------------
// quick-xml event machinery
// ---------------------------------------------------------------------------

/// Parses an XML part into owned events (see the module docs for why the round-trip must be
/// byte-exact for untouched regions).
fn parse_events(xml: &[u8], part: &str) -> Result<Vec<Event<'static>>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut events = Vec::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("{part} is not well-formed XML: {error}"))?;
        match event {
            Event::Eof => return Ok(events),
            event => events.push(event.into_owned()),
        }
    }
}

fn element_is(event: &Event<'_>, name: &str) -> bool {
    match event {
        Event::Start(start) | Event::Empty(start) => start.local_name().into_inner() == name,
        _ => false,
    }
}

/// Serializes events back out; every untouched event is written with its raw parsed bytes.
fn write_events(events: &[Event<'static>]) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    let mut writer = Writer::new(&mut buffer);
    for event in events {
        writer
            .write_event(event.borrow())
            .map_err(|error| format!("cannot serialize XML: {error}"))?;
    }
    Ok(buffer)
}

/// Span of the document's root element: `(start event index, exclusive end)`.
fn root_span(events: &[Event<'static>]) -> Result<(usize, usize), String> {
    let start = events
        .iter()
        .position(|event| matches!(event, Event::Start(_)))
        .ok_or_else(|| "document has no root element".to_owned())?;
    let mut depth = 0usize;
    for (index, event) in events.iter().enumerate().skip(start + 1) {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                if depth == 0 {
                    return Ok((start, index + 1));
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    Err("document has no root element".to_owned())
}

/// Direct-child element spans of the element starting at `container_start`, whose exclusive
/// span end is `container_end` (its End event sits at `container_end - 1`). Returns
/// `(start, exclusive end)` pairs in document order; a Start..End subtree is covered whole so
/// a clone deep-copies children exactly like ElementTree's `copy.deepcopy`.
fn child_spans(
    events: &[Event<'static>],
    container_start: usize,
    container_end: usize,
) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let last = container_end.saturating_sub(1);
    let mut index = container_start + 1;
    while index < last {
        match &events[index] {
            Event::Empty(_) => {
                spans.push((index, index + 1));
                index += 1;
            }
            Event::Start(_) => {
                let mut depth = 0usize;
                let mut cursor = index + 1;
                let end = loop {
                    if cursor >= last {
                        break last; // malformed input: treat the rest as the child's body
                    }
                    match &events[cursor] {
                        Event::Start(_) => depth += 1,
                        Event::End(_) if depth == 0 => break cursor + 1,
                        Event::End(_) => depth -= 1,
                        _ => {}
                    }
                    cursor += 1;
                };
                spans.push((index, end));
                index = end;
            }
            _ => index += 1,
        }
    }
    spans
}

fn attribute_value(start: &BytesStart<'_>, key: &str) -> Result<Option<String>, String> {
    for attribute in start.attributes() {
        let attribute =
            attribute.map_err(|error| format!("malformed attribute in XML part: {error}"))?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.value.into_owned()));
        }
    }
    Ok(None)
}

/// Sets (or adds) an attribute while preserving the element's attribute order: quick-xml has
/// no in-place attribute editing, so the whole list is rebuilt. Raw values are kept in their
/// parsed (escaped) form; generated values (numbers, UUIDs, paths) contain no characters that
/// would need escaping.
fn set_attribute(start: &mut BytesStart<'static>, key: &str, value: &str) -> Result<(), String> {
    let mut attributes: Vec<(String, String)> = Vec::new();
    for attribute in start.attributes() {
        let attribute =
            attribute.map_err(|error| format!("malformed attribute in XML part: {error}"))?;
        attributes.push((attribute.key.as_ref().to_owned(), attribute.value.into_owned()));
    }
    match attributes.iter_mut().find(|(name, _)| name == key) {
        Some((_, existing)) => *existing = value.to_owned(),
        None => attributes.push((key.to_owned(), value.to_owned())),
    }
    start.clear_attributes();
    for (name, value) in &attributes {
        start.push_attribute(Attribute {
            key: QName(name.as_str()),
            value: Cow::Borrowed(value.as_str()),
        });
    }
    Ok(())
}

fn stack_ends_with(stack: &[String], suffix: &[&str]) -> bool {
    stack.len() >= suffix.len()
        && stack[stack.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(entry, expected)| entry.as_str() == *expected)
}

/// The first `resources/object/components/component` element's production path and optional
/// transform — the same strict `/`-separated ElementTree path the Python resolves for both
/// duplicate and inline modes.
struct SourceComponent {
    path: String,
    transform: Option<String>,
}

fn first_source_component(core: &[u8]) -> Result<SourceComponent, String> {
    let events = parse_events(core, CORE_MODEL_ENTRY)?;
    let mut stack: Vec<String> = Vec::new();
    for event in &events {
        match event {
            Event::Start(start) | Event::Empty(start) => {
                let name = start.local_name().into_inner().to_owned();
                if name == "component"
                    && stack_ends_with(&stack, &["resources", "object", "components"])
                {
                    let path = attribute_value(start, "p:path")?.unwrap_or_default();
                    if path.is_empty() {
                        return Err("source component has no production path".to_owned());
                    }
                    return Ok(SourceComponent {
                        path,
                        transform: attribute_value(start, "transform")?,
                    });
                }
                if matches!(event, Event::Start(_)) {
                    stack.push(name);
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            _ => {}
        }
    }
    Err("3MF core model has no source component".to_owned())
}

// ---------------------------------------------------------------------------
// 3D/3dmodel.model
// ---------------------------------------------------------------------------

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

/// The source build item's z translation (transform value 11) — see `assemble_project`.
/// The source build item's transform applied to a mesh-bbox center: where the model's
/// geometric center actually sits on the bed in the source (and therefore probe) slice.
/// A probe-derived first-layer patch must be centered on this point — the assembler places
/// clones by that same center, and centering the patch on the contact strip instead shifts
/// the whole reserved area when the model is not prism-shaped (a lying animal's strip center
/// sits well below its bbox center).
pub fn source_model_center(core: &[u8], center: (f64, f64, f64)) -> Result<(f64, f64), String> {
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
    // 3MF matrices are column-major 3x4: [m00 m10 m20 m01 m11 m21 m02 m12 m22 tx ty tz].
    let (x, y, z) = center;
    Ok((
        values[0] * x + values[3] * y + values[6] * z + values[9],
        values[1] * x + values[4] * y + values[7] * z + values[10],
    ))
}

fn source_z_offset(core: &[u8]) -> Result<f64, String> {
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
    Ok(values[11])
}

#[allow(clippy::too_many_arguments)]
fn rebuild_core(
    core: &[u8],
    placements: &[PackedPlacement],
    component_paths: Option<&[String]>,
    inline_mesh_template: Option<&[Event<'static>]>,
    reuse_source_object_instances: bool,
    z_offset: f64,
    footprint_center: (f64, f64),
) -> Result<Vec<u8>, String> {
    let events = parse_events(core, CORE_MODEL_ENTRY)?;
    let layout = locate_core_layout(&events)?;
    let template_span = layout.object_spans[0];
    let source_object_id = match &events[template_span.0] {
        Event::Start(start) | Event::Empty(start) => {
            attribute_value(start, "id")?.ok_or_else(|| "template object has no id".to_owned())?
        }
        _ => unreachable!("object span starts with an element"),
    };

    let mut rebuilt: Vec<Event<'static>> = events[..=layout.resources_start].to_vec();
    // The template's own resources are dropped (reuse mode keeps only the first object, in
    // its original position); every packed object gets a fresh clone with deterministic
    // identities so BambuStudio treats the clones as distinct painted objects.
    for span in child_spans(&events, layout.resources_start, layout.resources_end + 1) {
        if reuse_source_object_instances && span == template_span {
            rebuilt.extend(events[span.0..span.1].iter().cloned());
        }
    }
    if !reuse_source_object_instances {
        for index in 0..placements.len() {
            let inline = inline_mesh_template.is_some();
            let mut object_events: Vec<Event<'static>> = match inline_mesh_template {
                Some(template) => template.to_vec(),
                None => events[template_span.0..template_span.1].to_vec(),
            };
            if let Event::Start(start) | Event::Empty(start) = &mut object_events[0] {
                set_attribute(start, "id", &clone_object_id(index).to_string())?;
                let kind = if inline { "inline-resource" } else { "resource" };
                set_attribute(start, "p:UUID", &stable_uuid(kind, index))?;
            }
            if !inline {
                for event in object_events.iter_mut().skip(1) {
                    if element_is(event, "component") {
                        if let Event::Start(start) | Event::Empty(start) = event {
                            set_attribute(start, "p:UUID", &stable_uuid("component", index))?;
                            if let Some(paths) = component_paths {
                                set_attribute(start, "p:path", &paths[index])?;
                            }
                        }
                        break; // the Python `find` rewrites only the first component
                    }
                }
            }
            rebuilt.extend(object_events);
        }
    }
    rebuilt.extend(
        events[layout.resources_end..layout.build_start]
            .iter()
            .cloned(),
    );
    let items = build_item_events(
        placements,
        reuse_source_object_instances,
        &source_object_id,
        z_offset,
        footprint_center,
    );
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
                events[layout.build_end.expect("`<build>` has a matching end")..].iter().cloned(),
            );
        }
        _ => unreachable!("build span starts with an element"),
    }
    write_events(&rebuilt)
}

fn build_item_events(
    placements: &[PackedPlacement],
    reuse_source_object_instances: bool,
    source_object_id: &str,
    z_offset: f64,
    footprint_center: (f64, f64),
) -> Vec<Event<'static>> {
    (0..placements.len())
        .map(|index| {
            let object_id = if reuse_source_object_instances {
                source_object_id.to_owned()
            } else {
                (index + 2).to_string()
            };
            build_item_event(
                index,
                &object_id,
                &placements[index],
                z_offset,
                footprint_center,
            )
        })
        .collect()
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

/// Contract 2: duplicated mesh parts are byte copies of the source entry — `paint_color`
/// attributes live there and must never be touched. Returns the per-index component paths.
fn duplicate_colored_mesh_parts(
    entries: &mut Entries,
    core: &[u8],
    count: usize,
) -> Result<Vec<String>, String> {
    let component = first_source_component(core)?;
    let source_path = component.path;
    let source_entry = source_path.trim_start_matches('/').to_owned();
    if entry(entries, &source_entry).is_none() {
        return Err(format!("source mesh part missing from archive: {source_entry}"));
    }
    let mut paths = vec![source_path];
    if count == 1 {
        return Ok(paths);
    }
    let base = source_entry
        .strip_suffix(".model")
        .unwrap_or(&source_entry)
        .to_owned();
    // Relationships are inserted by plain string surgery, mirroring the Python: only when the
    // rels entry exists and is non-empty. `str.replace` would rewrite every closing tag, but
    // well-formed XML has exactly one, so inserting before the first is equivalent.
    let mut relationship_text: Option<String> = entry(entries, RELATIONSHIPS_ENTRY)
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .filter(|text| !text.is_empty());
    for index in 1..count {
        let duplicate_entry = format!("{base}_instance_{}.model", index + 1);
        let source_bytes =
            entry(entries, &source_entry).expect("source entry checked above").to_vec();
        set_entry(entries, &duplicate_entry, source_bytes);
        let duplicate_path = format!("/{duplicate_entry}");
        paths.push(duplicate_path.clone());
        if let Some(text) = relationship_text.as_mut()
            && let Some(position) = text.find("</Relationships>")
        {
            let relationship = format!(
                " <Relationship Target=\"{duplicate_path}\" Id=\"paint-copy-{}\" \
                 Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\n",
                index + 1
            );
            text.insert_str(position, &relationship);
        }
    }
    if let Some(text) = relationship_text {
        set_entry(entries, RELATIONSHIPS_ENTRY, text.into_bytes());
    }
    Ok(paths)
}

/// Builds the inline mesh object for `inline_colored_mesh_objects` mode: the mesh part's
/// `<object>` element with the component transform baked into every vertex.
fn load_inline_mesh_template(entries: &Entries, core: &[u8]) -> Result<Vec<Event<'static>>, String> {
    let component = first_source_component(core)?;
    let source_entry = component.path.trim_start_matches('/');
    let mesh_bytes = entry(entries, source_entry)
        .ok_or_else(|| format!("source mesh part missing from archive: {source_entry}"))?
        .to_vec();
    let events = parse_events(&mesh_bytes, source_entry)?;
    let missing = || "source component has no inline mesh object".to_owned();
    let (root_start, root_end) = root_span(&events)?;
    let resources = child_spans(&events, root_start, root_end)
        .into_iter()
        .find(|&span| element_is(&events[span.0], "resources"))
        .ok_or_else(missing)?;
    let object_span = child_spans(&events, resources.0, resources.1)
        .into_iter()
        .find(|&span| element_is(&events[span.0], "object"))
        .ok_or_else(missing)?;
    if !child_spans(&events, object_span.0, object_span.1)
        .iter()
        .any(|&span| element_is(&events[span.0], "mesh"))
    {
        return Err(missing());
    }
    let mut object = events[object_span.0..object_span.1].to_vec();
    // An inlined mesh replaces the component, so the component's transform — normally applied
    // between mesh and build item — must be baked into the vertex coordinates to keep the
    // object-local geometry identical to the component-based modes (see `item_transform`).
    let matrix = embedded_mesh::parse_transform(component.transform.as_deref().unwrap_or(""))?;
    for event in object.iter_mut().skip(1) {
        if element_is(event, "vertex")
            && let Event::Start(start) | Event::Empty(start) = event
        {
            bake_vertex(start, &matrix)?;
        }
    }
    Ok(object)
}

/// Rewrites one `<vertex>`'s x/y/z with the matrix applied, through the same `_number`
/// formatting the rest of the generated XML uses.
fn bake_vertex(start: &mut BytesStart<'static>, matrix: &[[f64; 3]; 4]) -> Result<(), String> {
    let mut axes = [None::<f64>; 3];
    for attribute in start.attributes() {
        let attribute =
            attribute.map_err(|error| format!("malformed attribute in XML part: {error}"))?;
        let slot = match attribute.key.as_ref() {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            _ => continue,
        };
        let value = attribute.value.trim().parse::<f64>().map_err(|_| {
            format!("vertex coordinate is not a number: {:?}", attribute.value)
        })?;
        axes[slot] = Some(value);
    }
    let [Some(x), Some(y), Some(z)] = axes else {
        return Err("vertex is missing an x/y/z coordinate".to_owned());
    };
    let (x, y, z) = embedded_mesh::apply_matrix(matrix, (x, y, z));
    set_attribute(start, "x", &number(x))?;
    set_attribute(start, "y", &number(y))?;
    set_attribute(start, "z", &number(z))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Metadata parts
// ---------------------------------------------------------------------------

fn rebuild_model_settings(
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
                rebuilt.extend(rebuild_assemble_events(&events, span, placements, z_offset)?);
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

/// Sets the clone's `id` and gives every direct `part` a deterministic uuid — BambuStudio
/// matches model_settings objects to core resources by these ids, so the object id numbering
/// must stay exactly `index + 2`.
/// Duplicates the template's `object_id=<id>|<table>` lines across every packed clone id.
/// Lines without that prefix pass through untouched.
/// Duplicates the template's `object_id=<n>|<table>` lines across every packed clone.
/// The `object_id` here is the 1-based ordinal of the object in the model's object list —
/// OrcaSlicer indexes this file by `object index + 1` (`bbs_3mf.cpp`), not by the 3MF
/// object id used elsewhere in the archive. Lines without that prefix pass through.
fn rebuild_layer_heights_profile(data: &[u8], clone_count: usize) -> Vec<u8> {
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

/// Replaces the plate's `model_instance` children with one per placement (the per-instance
/// metadata BambuStudio reads to map instances back to model_settings objects).
fn rebuild_plate_events(
    events: &[Event<'static>],
    span: (usize, usize),
    placements: &[PackedPlacement],
) -> Result<Vec<Event<'static>>, String> {
    let (lo, hi) = span;
    let mut out: Vec<Event<'static>> = Vec::new();
    let mut appended_end: Option<Event<'static>> = None;
    match events[lo].clone() {
        Event::Empty(start) => {
            out.push(Event::Start(start));
            appended_end = Some(Event::End(BytesEnd::new("plate")));
        }
        Event::Start(_) => out.push(events[lo].clone()),
        _ => unreachable!("plate span starts with an element"),
    }
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
    match appended_end {
        Some(end) => out.push(end),
        None => out.push(events[hi - 1].clone()),
    }
    Ok(out)
}

fn model_instance_events(index: usize) -> Vec<Event<'static>> {
    let mut events = vec![Event::Start(BytesStart::new("model_instance"))];
    for (key, value) in [
        ("object_id", (index + 2).to_string()),
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
    let mut out: Vec<Event<'static>> = Vec::new();
    let mut appended_end: Option<Event<'static>> = None;
    match events[lo].clone() {
        Event::Empty(start) => {
            out.push(Event::Start(start));
            appended_end = Some(Event::End(BytesEnd::new("assemble")));
        }
        Event::Start(_) => out.push(events[lo].clone()),
        _ => unreachable!("assemble span starts with an element"),
    }
    let transform = format!("1 0 0 0 1 0 0 0 1 0 0 {}", number(z_offset));
    for index in 0..placements.len() {
        let object_id = (index + 2).to_string();
        let mut item = BytesStart::new("assemble_item");
        item.push_attribute(("object_id", object_id.as_str()));
        item.push_attribute(("instance_id", "0"));
        item.push_attribute(("transform", transform.as_str()));
        item.push_attribute(("offset", "0 0 0"));
        out.push(Event::Empty(item));
    }
    match appended_end {
        Some(end) => out.push(end),
        None => out.push(events[hi - 1].clone()),
    }
    Ok(out)
}

/// Rewrites cut_information.xml to N objects (`id = index + 1`) — one per packed object, each
/// with the empty cut metadata BambuStudio expects on pre-cut projects.
fn rebuild_cut_information(data: &[u8], count: usize) -> Result<Vec<u8>, String> {
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
    let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for point in rings.iter().flatten() {
        bounds[0] = bounds[0].min(point.x);
        bounds[1] = bounds[1].min(point.y);
        bounds[2] = bounds[2].max(point.x);
        bounds[3] = bounds[3].max(point.y);
    }
    bounds
}

/// Rewrites plate_1.json's `bbox_all`/`bbox_objects`/`first_layer_time` from the world-space
/// footprints, serialized compact like `json.dumps(separators=(",", ":"))`.
fn rebuild_plate_json(data: &[u8], world_polygons: &[Vec<Vec<Point>>]) -> Result<Vec<u8>, String> {
    let mut plate: serde_json::Value = serde_json::from_slice(data)
        .map_err(|error| format!("{PLATE_JSON_ENTRY} is not valid JSON: {error}"))?;
    let bounds: Vec<[f64; 4]> = world_polygons.iter().map(|rings| polygon_bounds(rings)).collect();
    plate["bbox_all"] = serde_json::json!([
        bounds.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min),
        bounds.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min),
        bounds.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max),
        bounds.iter().map(|b| b[3]).fold(f64::NEG_INFINITY, f64::max),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use zip::{ZipArchive, ZipWriter};

    const SYNTHETIC_MESH_PART: &str = r#"<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/><vertex x="10" y="10" z="0"/><vertex x="0" y="10" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/><triangle v1="0" v2="2" v3="3"/></triangles></mesh></object></resources></model>"#;
    const SYNTHETIC_CORE: &str = r#"<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06"><resources><object id="2" type="model"><components><component p:path="/3D/Objects/mesh.model" transform="1 0 0 0 1 0 0 0 1 10 20 -0.5"/></components></object></resources><build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 50 60 0.5"/></build></model>"#;
    const SYNTHETIC_MODEL_SETTINGS: &str = r#"<config><object id="2"/><plate/></config>"#;
    const SYNTHETIC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/Objects/mesh.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;

    /// The synthetic template from `embedded_mesh`'s tests: a component carrying its own
    /// translation (10, 20, -0.5) over a mesh centered on the local origin — the structure
    /// that broke the real Goat 3 P1S pack — plus the metadata entries the assembler touches.
    fn synthetic_template(directory: &Path, include_rels: bool) -> PathBuf {
        let settings = serde_json::json!({
            "printer_model": "Synthetic Printer",
            "printable_area": ["0x0", "200x0", "200x200", "0x200"],
        })
        .to_string();
        let source = directory.join(if include_rels {
            "synthetic_source_rels.3mf"
        } else {
            "synthetic_source.3mf"
        });
        let mut entries: Vec<(&str, String)> = vec![
            ("3D/3dmodel.model", SYNTHETIC_CORE.to_owned()),
            ("3D/Objects/mesh.model", SYNTHETIC_MESH_PART.to_owned()),
            ("Metadata/project_settings.config", settings),
            ("Metadata/model_settings.config", SYNTHETIC_MODEL_SETTINGS.to_owned()),
        ];
        if include_rels {
            entries.push(("3D/_rels/3dmodel.model.rels", SYNTHETIC_RELS.to_owned()));
        }
        write_zip(&source, &entries);
        source
    }

    fn write_zip(path: &Path, entries: &[(&str, String)]) {
        let mut archive = ZipWriter::new(File::create(path).unwrap());
        for (name, content) in entries {
            archive.start_file(*name, SimpleFileOptions::default()).unwrap();
            archive.write_all(content.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
    }

    fn baked_stl(directory: &Path, source: &Path) -> PathBuf {
        let triangles = embedded_mesh::embedded_mesh_triangles(source).unwrap();
        let path = directory.join("baked.stl");
        embedded_mesh::write_binary_stl(&path, &triangles).unwrap();
        path
    }

    fn base_options(stl: &Path) -> AssembleOptions {
        AssembleOptions {
            placements: vec![
                PackedPlacement { x_mm: 60.0, y_mm: 70.0, angle_deg: 0.0 },
                PackedPlacement { x_mm: 150.0, y_mm: 160.0, angle_deg: 90.0 },
            ],
            stl_path: stl.to_path_buf(),
            edge_margin: 0.0,
            purge_lines: Vec::new(),
            keepouts: Vec::new(),
            duplicate_mesh_per_object: false,
            reuse_source_object_instances: false,
            inline_colored_mesh_objects: false,
            footprint_override: None,
        }
    }

    fn read_entry(archive_path: &Path, name: &str) -> Vec<u8> {
        let mut archive = ZipArchive::new(File::open(archive_path).unwrap()).unwrap();
        let mut bytes = Vec::new();
        archive.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
        bytes
    }

    fn has_entry(archive_path: &Path, name: &str) -> bool {
        let mut archive = ZipArchive::new(File::open(archive_path).unwrap()).unwrap();
        (0..archive.len()).any(|index| archive.by_index(index).unwrap().name() == name)
    }

    fn entry_names(archive_path: &Path) -> Vec<String> {
        let mut archive = ZipArchive::new(File::open(archive_path).unwrap()).unwrap();
        (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_owned())
            .collect()
    }

    fn count_elements(data: &[u8], name: &str) -> usize {
        parse_events(data, "test data").unwrap().iter().filter(|event| element_is(event, name)).count()
    }

    /// `(objectid, transform)` of every build item, in document order.
    fn core_items(core: &[u8]) -> Vec<(String, String)> {
        parse_events(core, "test core")
            .unwrap()
            .iter()
            .filter_map(|event| {
                if !element_is(event, "item") {
                    return None;
                }
                let (Event::Start(start) | Event::Empty(start)) = event else { unreachable!() };
                Some((
                    attribute_value(start, "objectid").unwrap().unwrap(),
                    attribute_value(start, "transform").unwrap().unwrap(),
                ))
            })
            .collect()
    }

    fn transform_values(transform: &str) -> Vec<f64> {
        transform.split_whitespace().map(|token| token.parse().unwrap()).collect()
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    fn element_attribute_values(data: &[u8], element: &str, attribute: &str) -> Vec<String> {
        let events = parse_events(data, "test data").unwrap();
        let mut values = Vec::new();
        for event in &events {
            if element_is(event, element)
                && let Event::Start(start) | Event::Empty(start) = event
                && let Some(value) = attribute_value(start, attribute).unwrap()
            {
                values.push(value);
            }
        }
        values
    }

    fn vertex_positions(model: &[u8]) -> Vec<(f64, f64, f64)> {
        parse_events(model, "test mesh part")
            .unwrap()
            .iter()
            .filter_map(|event| {
                if !element_is(event, "vertex") {
                    return None;
                }
                let (Event::Start(start) | Event::Empty(start)) = event else { unreachable!() };
                let axis = |name: &str| {
                    attribute_value(start, name).unwrap().unwrap().parse::<f64>().unwrap()
                };
                Some((axis("x"), axis("y"), axis("z")))
            })
            .collect()
    }

    /// One entry per `model_instance`: its `metadata` (key, value) pairs.
    fn model_instance_metadata(settings: &[u8]) -> Vec<Vec<(String, String)>> {
        let events = parse_events(settings, "test model_settings").unwrap();
        let mut all = Vec::new();
        let mut current: Option<Vec<(String, String)>> = None;
        for event in &events {
            match event {
                Event::Start(start) if start.local_name().into_inner() == "model_instance" => {
                    current = Some(Vec::new());
                }
                Event::End(end) if end.local_name().into_inner() == "model_instance" => {
                    all.push(current.take().unwrap());
                }
                Event::Empty(start)
                    if start.local_name().into_inner() == "metadata" && current.is_some() =>
                {
                    current.as_mut().unwrap().push((
                        attribute_value(start, "key").unwrap().unwrap(),
                        attribute_value(start, "value").unwrap().unwrap(),
                    ));
                }
                _ => {}
            }
        }
        all
    }

    fn assert_png_size(png: &[u8], size: u32) {
        assert_eq!(&png[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), size);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), size);
    }

    /// Regression for the real Goat 3 template (e2e slicing failure): a `<plate>` written as
    /// an open..close pair with existing `<model_instance>` children. The rebuild must
    /// replace those children IN PLACE — new instances inside the plate, exactly one
    /// `</plate>` — or BambuStudio rejects the project ("don't find plate created before").
    /// The synthetic fixture's self-closed `<plate/>` could not catch this shape.
    #[test]
    fn plate_with_existing_instances_rebuilds_instances_in_place() {
        let model_settings = r#"<config><object id="2"/><plate><metadata key="plater_id" value="1"/><model_instance><metadata key="object_id" value="99"/><metadata key="instance_id" value="0"/><metadata key="identify_id" value="200"/></model_instance></plate></config>"#;
        let placements = vec![
            PackedPlacement { x_mm: 10.0, y_mm: 20.0, angle_deg: 0.0 },
            PackedPlacement { x_mm: 30.0, y_mm: 40.0, angle_deg: 90.0 },
        ];
        let rebuilt = rebuild_model_settings(model_settings.as_bytes(), &placements, 0.5).unwrap();
        let text = String::from_utf8(rebuilt.clone()).unwrap();

        assert_eq!(text.matches("<model_instance").count(), 2, "{text}");
        assert_eq!(text.matches("</plate>").count(), 1, "{text}");
        let plate_open = text.find("<plate>").unwrap();
        let plate_close = text.find("</plate>").unwrap();
        for (position, _) in text.match_indices("<model_instance") {
            assert!(
                position > plate_open && position < plate_close,
                "instance at {position} escaped the plate: {text}"
            );
        }
        // The old instance is gone, the plate's own metadata survives, and the new instances
        // carry the deterministic identify ids.
        assert!(!text.contains(r#"value="99""#), "{text}");
        assert!(!text.contains(r#"value="200""#), "{text}");
        assert!(text.contains(r#"key="plater_id" value="1""#), "{text}");
        let instances = model_instance_metadata(&rebuilt);
        assert_eq!(instances.len(), 2);
        for (index, metadata) in instances.iter().enumerate() {
            assert_eq!(
                metadata,
                &vec![
                    ("object_id".to_owned(), (index + 2).to_string()),
                    ("instance_id".to_owned(), "0".to_owned()),
                    ("identify_id".to_owned(), (1000 + index * 10).to_string()),
                ]
            );
        }

        // An open..close pair with NO children must also grow its instances inside the pair.
        let rebuilt = rebuild_model_settings(
            br#"<config><object id="2"/><plate></plate></config>"#,
            &placements,
            0.5,
        )
        .unwrap();
        let text = String::from_utf8(rebuilt).unwrap();
        assert_eq!(text.matches("<model_instance").count(), 2, "{text}");
        assert_eq!(text.matches("</plate>").count(), 1, "{text}");
        assert!(
            text.find("<model_instance").unwrap() > text.find("<plate>").unwrap()
                && text.find("<model_instance").unwrap() < text.find("</plate>").unwrap(),
            "{text}"
        );
    }

    /// Symmetric regression for `<assemble>` with pre-existing assemble_items (the real Goat 3
    /// template has two, one carrying an extra volume_id): the new items replace them inside
    /// the element with exactly one closing tag.
    /// OrcaSlicer aborts a prime-tower slice with "all objects have the same variable layer
    /// height" when the copied profile still points only at the source's ordinal: the clones
    /// slice without their layer-height table and the tables then differ. The rewritten ids
    /// are 1-based ordinals (object index + 1 in `bbs_3mf.cpp`), not the 3MF object ids the
    /// clones carry in the model itself.
    #[test]
    fn layer_heights_profile_is_duplicated_for_every_clone_id() {
        let rebuilt = rebuild_layer_heights_profile(b"object_id=2|0.000000;0.250000;0.460000\n", 3);
        let text = String::from_utf8(rebuilt).unwrap();
        assert_eq!(
            text,
            "object_id=1|0.000000;0.250000;0.460000\n\
             object_id=2|0.000000;0.250000;0.460000\n\
             object_id=3|0.000000;0.250000;0.460000\n"
        );

        // Pass-through for lines without an object_id mapping, and end-to-end: an assembled
        // archive carries one line per placement, and a template without the entry never
        // grows one.
        let passthrough = rebuild_layer_heights_profile(b"not a profile line", 2);
        assert_eq!(
            String::from_utf8(passthrough).unwrap(),
            "not a profile line\n"
        );

        let dir = tempfile::tempdir().unwrap();
        let source = synthetic_template(dir.path(), false);
        let stl = baked_stl(dir.path(), &source);
        let with_profile = dir.path().join("with_profile.3mf");
        let mut entries = vec![
            ("3D/3dmodel.model", SYNTHETIC_CORE.to_owned()),
            ("3D/Objects/mesh.model", SYNTHETIC_MESH_PART.to_owned()),
            (
                "Metadata/project_settings.config",
                serde_json::json!({
                    "printer_model": "Synthetic Printer",
                    "printable_area": ["0x0", "200x0", "200x200", "0x200"],
                })
                .to_string(),
            ),
            (
                "Metadata/model_settings.config",
                SYNTHETIC_MODEL_SETTINGS.to_owned(),
            ),
            (
                "Metadata/layer_heights_profile.txt",
                "object_id=1|0.0;0.2\n".to_owned(),
            ),
        ];
        write_zip(&with_profile, &entries);

        let output = dir.path().join("packed.3mf");
        assemble_project(&with_profile, &output, &base_options(&stl)).unwrap();

        let profile = String::from_utf8(read_entry(
            &output,
            "Metadata/layer_heights_profile.txt",
        ))
        .unwrap();
        assert_eq!(profile, "object_id=1|0.0;0.2\nobject_id=2|0.0;0.2\n");
        let plain = dir.path().join("packed_plain.3mf");
        assemble_project(&source, &plain, &base_options(&stl)).unwrap();
        assert!(!has_entry(&plain, "Metadata/layer_heights_profile.txt"));
        entries.clear();
    }

    #[test]
    fn assemble_with_existing_items_rebuilds_items_in_place() {
        let model_settings = r#"<config><object id="2"/><plate/><assemble><assemble_item object_id="2" instance_id="0" transform="1 0 0 0 1 0 0 0 1 0 0 9" offset="0 0 0"/><assemble_item object_id="2" volume_id="0" transform="1 0 0 0 1 0 0 0 1 5 5 -4.3"/></assemble></config>"#;
        let placements = vec![
            PackedPlacement { x_mm: 10.0, y_mm: 20.0, angle_deg: 0.0 },
            PackedPlacement { x_mm: 30.0, y_mm: 40.0, angle_deg: 90.0 },
        ];
        let rebuilt = rebuild_model_settings(model_settings.as_bytes(), &placements, 0.5).unwrap();
        let text = String::from_utf8(rebuilt).unwrap();

        assert_eq!(text.matches("<assemble_item").count(), 2, "{text}");
        assert_eq!(text.matches("</assemble>").count(), 1, "{text}");
        let assemble_open = text.find("<assemble>").unwrap();
        let assemble_close = text.find("</assemble>").unwrap();
        for (position, _) in text.match_indices("<assemble_item") {
            assert!(
                position > assemble_open && position < assemble_close,
                "item at {position} escaped the assemble element: {text}"
            );
        }
        // The old items (z 9 and z -4.3, incl. the extra volume_id) are replaced by items
        // carrying the source z offset.
        assert!(!text.contains(r#"0 0 9""#), "{text}");
        assert!(!text.contains("-4.3"), "{text}");
        assert!(!text.contains("volume_id"), "{text}");
        assert_eq!(
            text.matches(r#"transform="1 0 0 0 1 0 0 0 1 0 0 0.5""#).count(),
            2,
            "{text}"
        );
        assert!(text.contains(r#"object_id="2""#), "{text}");
        assert!(text.contains(r#"object_id="3""#), "{text}");
    }

    #[test]
    fn stable_uuid_matches_the_python_vectors() {
        assert_eq!(stable_uuid("resource", 0), "9f3bce4a-ccbe-5fb1-94d4-8283e284f863");
        assert_eq!(stable_uuid("resource", 1), "4d85a473-7e39-59be-8e91-25a8029d62d0");
        assert_eq!(stable_uuid("build-item", 0), "25c59a57-7f8d-5ec0-8e2a-9635308923ab");
        assert_eq!(stable_uuid("build-item", 1), "a5cc3fc8-946f-58c7-8783-29d59c72a9fe");
        assert_eq!(stable_uuid("component", 0), "214e7b7d-2a5e-5286-86a3-a67481ba1263");
        assert_eq!(stable_uuid("inline-resource", 0), "b241d998-6fcf-55ec-9fd9-c1d902c24d61");
        assert_eq!(stable_uuid("part-0", 0), "1354723b-e007-51c9-b19a-e01549a67497");
    }

    #[test]
    fn number_matches_the_python_formatting() {
        assert_eq!(number(0.0), "0");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(6.123233995736766e-17), "0");
        assert_eq!(number(1e-11), "0");
        assert_eq!(number(1.0), "1");
        assert_eq!(number(-3.0), "-3");
        assert_eq!(number(200.00000000000003), "200");
        assert_eq!(number(-175.00000000000003), "-175");
        assert_eq!(number(3.70000005), "3.70000005");
        assert_eq!(number(2.5), "2.5");
        assert_eq!(number(-0.5), "-0.5");
        // 10 decimals with trailing zeros stripped.
        assert_eq!(number(0.123456789012), "0.123456789");
        assert_eq!(number(12.345678901234), "12.3456789012");
    }

    #[test]
    fn item_transform_rotates_the_footprint_center_out_of_the_translation() {
        let placement = PackedPlacement { x_mm: 60.0, y_mm: 70.0, angle_deg: 0.0 };
        assert_eq!(
            item_transform(&placement, 0.5, (15.0, 25.0)),
            "1 0 0 0 1 0 0 0 1 45 45 0.5"
        );
        // cos(90°) is ~6e-17 and must print as "0", keeping the attribute clean.
        let rotated = PackedPlacement { x_mm: 150.0, y_mm: 160.0, angle_deg: 90.0 };
        let values = transform_values(&item_transform(&rotated, 0.5, (15.0, 25.0)));
        let expected = [0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 175.0, 145.0, 0.5];
        assert_eq!(values.len(), 12);
        for (actual, reference) in values.iter().zip(expected) {
            assert_close(*actual, reference);
        }
    }

    /// End-to-end port of
    /// `tests/test_build_3mf.py::test_item_transform_compensates_component_translation`.
    #[test]
    fn item_transform_compensates_component_translation() {
        let dir = tempfile::tempdir().unwrap();
        let source = synthetic_template(dir.path(), false);
        let stl = baked_stl(dir.path(), &source);
        let output = dir.path().join("synthetic_packed.3mf");
        let options = base_options(&stl);
        assemble_project(&source, &output, &options).unwrap();

        let core = read_entry(&output, "3D/3dmodel.model");
        assert_eq!(count_elements(&core, "object"), 2, "one resource per placement");
        let items = core_items(&core);
        assert_eq!(items.len(), 2, "one build item per placement");

        // (x, y) refers to the baked mesh's bbox center (15, 25): the item translation must
        // rotate that center out, not use the raw placement.
        let expected = [
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 45.0, 45.0, 0.5],
            [0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 175.0, 145.0, 0.5],
        ];
        for ((_, transform), expected) in items.iter().zip(expected) {
            let values = transform_values(transform);
            assert_eq!(values.len(), 12);
            for (actual, reference) in values.iter().zip(expected) {
                assert_close(*actual, reference);
            }
        }

        // Independent end-to-end check: each clone's world-space vertex mean (embedded mesh
        // through the kept component transform, then the item transform) must equal its
        // packed placement exactly.
        let component_matrix = embedded_mesh::parse_transform(
            &element_attribute_values(&core, "component", "transform")[0],
        )
        .unwrap();
        let vertices = vertex_positions(&read_entry(&output, "3D/Objects/mesh.model"));
        for ((_, transform), expected) in items.iter().zip([(60.0, 70.0), (150.0, 160.0)]) {
            let item_matrix = embedded_mesh::parse_transform(transform).unwrap();
            let world: Vec<(f64, f64)> = vertices
                .iter()
                .map(|&point| {
                    let local = embedded_mesh::apply_matrix(&component_matrix, point);
                    let world = embedded_mesh::apply_matrix(&item_matrix, local);
                    (world.0, world.1)
                })
                .collect();
            let mean_x = world.iter().map(|point| point.0).sum::<f64>() / world.len() as f64;
            let mean_y = world.iter().map(|point| point.1).sum::<f64>() / world.len() as f64;
            assert_close(mean_x, expected.0);
            assert_close(mean_y, expected.1);
        }

        // Contract 1: project_settings.config is carried through byte-for-byte.
        assert_eq!(
            read_entry(&output, "Metadata/project_settings.config"),
            read_entry(&source, "Metadata/project_settings.config"),
        );
        // Contract 6: the synthetic template has no Bambu-only metadata, so none is invented.
        assert!(!has_entry(&output, "Metadata/cut_information.xml"));
        assert!(!has_entry(&output, "Metadata/plate_1.json"));

        // model_settings cardinality: N object clones + N plate instances.
        let settings = read_entry(&output, "Metadata/model_settings.config");
        assert_eq!(
            element_attribute_values(&settings, "object", "id"),
            vec!["2", "3"]
        );
        let instances = model_instance_metadata(&settings);
        assert_eq!(instances.len(), 2);
        for (index, metadata) in instances.iter().enumerate() {
            assert_eq!(
                metadata,
                &vec![
                    ("object_id".to_owned(), (index + 2).to_string()),
                    ("instance_id".to_owned(), "0".to_owned()),
                    ("identify_id".to_owned(), (1000 + index * 10).to_string()),
                ]
            );
        }

        // Previews: five PNG entries at the sizes BambuStudio expects; the three full plates
        // share one render.
        let plate_png = read_entry(&output, "Metadata/plate_1.png");
        assert_eq!(plate_png, read_entry(&output, "Metadata/plate_no_light_1.png"));
        assert_eq!(plate_png, read_entry(&output, "Metadata/top_1.png"));
        assert_png_size(&plate_png, 512);
        assert_png_size(&read_entry(&output, "Metadata/pick_1.png"), 512);
        assert_png_size(&read_entry(&output, "Metadata/plate_1_small.png"), 128);

        // Template entry order is preserved; new entries (the previews here) come last.
        let names = entry_names(&output);
        assert_eq!(names[0], "3D/3dmodel.model");
        assert_eq!(names[1], "3D/Objects/mesh.model");
        assert_eq!(names[2], "Metadata/project_settings.config");
        assert_eq!(names[3], "Metadata/model_settings.config");

        // Determinism: assembling the same template twice must produce identical bytes.
        let second = dir.path().join("synthetic_packed_again.3mf");
        assemble_project(&source, &second, &options).unwrap();
        assert_eq!(fs::read(&output).unwrap(), fs::read(&second).unwrap());
    }

    #[test]
    fn duplicate_mode_clones_the_mesh_part_and_appends_relationships() {
        let dir = tempfile::tempdir().unwrap();
        let source = synthetic_template(dir.path(), true);
        let stl = baked_stl(dir.path(), &source);
        let output = dir.path().join("duplicate.3mf");
        let mut options = base_options(&stl);
        options.duplicate_mesh_per_object = true;
        assemble_project(&source, &output, &options).unwrap();

        let core = read_entry(&output, "3D/3dmodel.model");
        let paths = element_attribute_values(&core, "component", "p:path");
        assert_eq!(
            paths,
            vec!["/3D/Objects/mesh.model", "/3D/Objects/mesh_instance_2.model"]
        );
        // Contract 2: the duplicate is a byte copy of the source mesh part.
        let original = read_entry(&source, "3D/Objects/mesh.model");
        assert_eq!(original, read_entry(&output, "3D/Objects/mesh.model"));
        assert_eq!(original, read_entry(&output, "3D/Objects/mesh_instance_2.model"));
        let rels = String::from_utf8(read_entry(&output, "3D/_rels/3dmodel.model.rels")).unwrap();
        assert!(rels.contains("Target=\"/3D/Objects/mesh.model\""), "{rels}");
        assert!(
            rels.contains("Target=\"/3D/Objects/mesh_instance_2.model\" Id=\"paint-copy-2\""),
            "{rels}"
        );
    }

    #[test]
    fn reuse_mode_keeps_one_object_and_the_source_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let source = synthetic_template(dir.path(), false);
        let stl = baked_stl(dir.path(), &source);
        let output = dir.path().join("reuse.3mf");
        let mut options = base_options(&stl);
        options.reuse_source_object_instances = true;
        assemble_project(&source, &output, &options).unwrap();

        let core = read_entry(&output, "3D/3dmodel.model");
        assert_eq!(count_elements(&core, "object"), 1, "one shared resource");
        let items = core_items(&core);
        assert_eq!(items.len(), 2);
        assert!(
            items.iter().all(|(objectid, _)| objectid == "2"),
            "both items reference the source object"
        );
        // Reuse mode skips the model_settings rewrite entirely: bytes identical.
        assert_eq!(
            read_entry(&output, "Metadata/model_settings.config"),
            read_entry(&source, "Metadata/model_settings.config"),
        );
        assert!(!has_entry(&output, "Metadata/cut_information.xml"));
        assert!(!has_entry(&output, "Metadata/plate_1.json"));
    }

    #[test]
    fn inline_mode_bakes_the_component_transform_into_inlined_vertices() {
        let dir = tempfile::tempdir().unwrap();
        let source = synthetic_template(dir.path(), false);
        let stl = baked_stl(dir.path(), &source);
        let output = dir.path().join("inline.3mf");
        let mut options = base_options(&stl);
        options.inline_colored_mesh_objects = true;
        assemble_project(&source, &output, &options).unwrap();

        let core = read_entry(&output, "3D/3dmodel.model");
        assert_eq!(count_elements(&core, "object"), 2);
        assert_eq!(count_elements(&core, "mesh"), 2, "resources carry inline meshes");
        assert_eq!(count_elements(&core, "component"), 0, "no component remains");
        // The component translation (10, 20, -0.5) is baked into every vertex.
        let vertices = vertex_positions(&core);
        let baked = [
            (10.0, 20.0, -0.5),
            (20.0, 20.0, -0.5),
            (20.0, 30.0, -0.5),
            (10.0, 30.0, -0.5),
        ];
        assert_eq!(vertices.len(), 8, "two objects × four vertices");
        for (actual, expected) in vertices.iter().take(4).zip(baked) {
            assert_close(actual.0, expected.0);
            assert_close(actual.1, expected.1);
            assert_close(actual.2, expected.2);
        }
    }

    /// Contract 3: the source build item's z translation (transform[11] = 0.5 here) becomes
    /// every clone's item z offset, in every mesh strategy.
    #[test]
    fn every_mode_reuses_the_source_item_z_offset() {
        fn set_duplicate(options: &mut AssembleOptions) {
            options.duplicate_mesh_per_object = true;
        }
        fn set_reuse(options: &mut AssembleOptions) {
            options.reuse_source_object_instances = true;
        }
        fn set_inline(options: &mut AssembleOptions) {
            options.inline_colored_mesh_objects = true;
        }
        let modes: Vec<(&str, bool, fn(&mut AssembleOptions))> = vec![
            ("default", false, |_| {}),
            ("duplicate", true, set_duplicate),
            ("reuse", false, set_reuse),
            ("inline", false, set_inline),
        ];
        let dir = tempfile::tempdir().unwrap();
        for (label, include_rels, mutate) in modes {
            let source = synthetic_template(dir.path(), include_rels);
            let stl = baked_stl(dir.path(), &source);
            let output = dir.path().join(format!("{label}.3mf"));
            let mut options = base_options(&stl);
            mutate(&mut options);
            assemble_project(&source, &output, &options)
                .unwrap_or_else(|error| panic!("{label} mode failed: {error}"));
            let items = core_items(&read_entry(&output, "3D/3dmodel.model"));
            assert_eq!(items.len(), 2, "{label} mode");
            for (_, transform) in &items {
                assert_close(transform_values(transform)[11], 0.5);
            }
        }
    }

    #[test]
    fn cut_information_present_is_rewritten_and_stays_well_formed() {
        let dir = tempfile::tempdir().unwrap();
        let settings = serde_json::json!({
            "printer_model": "Synthetic Printer",
            "printable_area": ["0x0", "200x0", "200x200", "0x200"],
        })
        .to_string();
        let source = dir.path().join("cut_info_source.3mf");
        write_zip(
            &source,
            &[
                ("3D/3dmodel.model", SYNTHETIC_CORE.to_owned()),
                ("3D/Objects/mesh.model", SYNTHETIC_MESH_PART.to_owned()),
                ("Metadata/project_settings.config", settings),
                (
                    "Metadata/model_settings.config",
                    SYNTHETIC_MODEL_SETTINGS.to_owned(),
                ),
                (
                    "Metadata/cut_information.xml",
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><objects><object id=\"7\"><cut_id \
                     id=\"0\" check_sum=\"1\" connectors_cnt=\"0\"/></object></objects>"
                        .to_owned(),
                ),
            ],
        );
        let stl = baked_stl(dir.path(), &source);
        let output = dir.path().join("cut_info_out.3mf");
        assemble_project(&source, &output, &base_options(&stl)).unwrap();
        let text = String::from_utf8(read_entry(&output, "Metadata/cut_information.xml")).unwrap();
        // Regression: the root's End event must survive the rewrite — a truncated document
        // makes BambuStudio fail the whole archive load with "(2): unexpected end of data".
        assert!(
            text.trim_end().ends_with("</objects>"),
            "cut_information root must close: {text}"
        );
        assert_eq!(text.matches("<object ").count(), 2);
        assert!(!text.contains("id=\"7\""), "old cut object must be replaced");
    }
}

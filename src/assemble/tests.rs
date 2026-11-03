use super::core_model::item_transform;
use super::format::{number, stable_uuid};
use super::metadata::{rebuild_layer_heights_profile, rebuild_model_settings};
use super::xml::{attribute_value, element_is, parse_events};
use super::*;
use quick_xml::events::Event;
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::PathBuf,
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

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
        (
            "Metadata/model_settings.config",
            SYNTHETIC_MODEL_SETTINGS.to_owned(),
        ),
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
        archive
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
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
            PackedPlacement {
                x_mm: 60.0,
                y_mm: 70.0,
                angle_deg: 0.0,
            },
            PackedPlacement {
                x_mm: 150.0,
                y_mm: 160.0,
                angle_deg: 90.0,
            },
        ],
        stl_path: stl.to_path_buf(),
        edge_margin: 0.0,
        purge_lines: Vec::new(),
        keepouts: Vec::new(),
        duplicate_mesh_per_object: false,
        reuse_source_object_instances: false,
        inline_colored_mesh_objects: false,
        footprint_override: None,
        frame: None,
        world_polygons: None,
    }
}

fn read_entry(archive_path: &Path, name: &str) -> Vec<u8> {
    let mut archive = ZipArchive::new(File::open(archive_path).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
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
    parse_events(data, "test data")
        .unwrap()
        .iter()
        .filter(|event| element_is(event, name))
        .count()
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
            let (Event::Start(start) | Event::Empty(start)) = event else {
                unreachable!()
            };
            Some((
                attribute_value(start, "objectid").unwrap().unwrap(),
                attribute_value(start, "transform").unwrap().unwrap(),
            ))
        })
        .collect()
}

fn transform_values(transform: &str) -> Vec<f64> {
    transform
        .split_whitespace()
        .map(|token| token.parse().unwrap())
        .collect()
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
            let (Event::Start(start) | Event::Empty(start)) = event else {
                unreachable!()
            };
            let axis = |name: &str| {
                attribute_value(start, name)
                    .unwrap()
                    .unwrap()
                    .parse::<f64>()
                    .unwrap()
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
        PackedPlacement {
            x_mm: 10.0,
            y_mm: 20.0,
            angle_deg: 0.0,
        },
        PackedPlacement {
            x_mm: 30.0,
            y_mm: 40.0,
            angle_deg: 90.0,
        },
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

    let profile =
        String::from_utf8(read_entry(&output, "Metadata/layer_heights_profile.txt")).unwrap();
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
        PackedPlacement {
            x_mm: 10.0,
            y_mm: 20.0,
            angle_deg: 0.0,
        },
        PackedPlacement {
            x_mm: 30.0,
            y_mm: 40.0,
            angle_deg: 90.0,
        },
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
        text.matches(r#"transform="1 0 0 0 1 0 0 0 1 0 0 0.5""#)
            .count(),
        2,
        "{text}"
    );
    assert!(text.contains(r#"object_id="2""#), "{text}");
    assert!(text.contains(r#"object_id="3""#), "{text}");
}

#[test]
fn stable_uuid_matches_the_python_vectors() {
    assert_eq!(
        stable_uuid("resource", 0),
        "9f3bce4a-ccbe-5fb1-94d4-8283e284f863"
    );
    assert_eq!(
        stable_uuid("resource", 1),
        "4d85a473-7e39-59be-8e91-25a8029d62d0"
    );
    assert_eq!(
        stable_uuid("build-item", 0),
        "25c59a57-7f8d-5ec0-8e2a-9635308923ab"
    );
    assert_eq!(
        stable_uuid("build-item", 1),
        "a5cc3fc8-946f-58c7-8783-29d59c72a9fe"
    );
    assert_eq!(
        stable_uuid("component", 0),
        "214e7b7d-2a5e-5286-86a3-a67481ba1263"
    );
    assert_eq!(
        stable_uuid("inline-resource", 0),
        "b241d998-6fcf-55ec-9fd9-c1d902c24d61"
    );
    assert_eq!(
        stable_uuid("part-0", 0),
        "1354723b-e007-51c9-b19a-e01549a67497"
    );
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
    let placement = PackedPlacement {
        x_mm: 60.0,
        y_mm: 70.0,
        angle_deg: 0.0,
    };
    assert_eq!(
        item_transform(&placement, 0.5, (15.0, 25.0), None),
        "1 0 0 0 1 0 0 0 1 45 45 0.5"
    );
    // cos(90°) is ~6e-17 and must print as "0", keeping the attribute clean.
    let rotated = PackedPlacement {
        x_mm: 150.0,
        y_mm: 160.0,
        angle_deg: 90.0,
    };
    let values = transform_values(&item_transform(&rotated, 0.5, (15.0, 25.0), None));
    let expected = [
        0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 175.0, 145.0, 0.5,
    ];
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
    assert_eq!(
        count_elements(&core, "object"),
        2,
        "one resource per placement"
    );
    let items = core_items(&core);
    assert_eq!(items.len(), 2, "one build item per placement");

    // (x, y) refers to the baked mesh's bbox center (15, 25): the item translation must
    // rotate that center out, not use the raw placement.
    let expected = [
        [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 45.0, 45.0, 0.5],
        [
            0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 175.0, 145.0, 0.5,
        ],
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
    assert_eq!(
        plate_png,
        read_entry(&output, "Metadata/plate_no_light_1.png")
    );
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
        vec![
            "/3D/Objects/mesh.model",
            "/3D/Objects/mesh_instance_2.model"
        ]
    );
    // Contract 2: the duplicate is a byte copy of the source mesh part.
    let original = read_entry(&source, "3D/Objects/mesh.model");
    assert_eq!(original, read_entry(&output, "3D/Objects/mesh.model"));
    assert_eq!(
        original,
        read_entry(&output, "3D/Objects/mesh_instance_2.model")
    );
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
    assert_eq!(
        count_elements(&core, "mesh"),
        2,
        "resources carry inline meshes"
    );
    assert_eq!(
        count_elements(&core, "component"),
        0,
        "no component remains"
    );
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
    type Mode = (&'static str, bool, fn(&mut AssembleOptions));
    let modes: Vec<Mode> = vec![
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
    assert!(
        !text.contains("id=\"7\""),
        "old cut object must be replaced"
    );
}

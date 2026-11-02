//! The S1 object reader on generated 3MF projects: every part with its kind and paint, the
//! item rotation, mirroring, and determinism.

use std::{fs::File, io::Write, path::Path};

use goat_packer::embedded_mesh::{PartKind, load_object_mesh};
use goat_packer::mesh_section::{SectionOptions, object_section, raw_section_loops};
use goat_packer::{Point, polygon_area};
use zip::{ZipWriter, write::SimpleFileOptions};

fn area(loops: &[Vec<Point>]) -> f64 {
    loops.iter().map(|ring| polygon_area(ring)).sum()
}

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

const OBJECT_PART: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources>
<object id="1" type="model"><mesh><vertices>
<vertex x="0" y="0" z="0"/><vertex x="20" y="0" z="0"/><vertex x="0" y="10" z="0"/><vertex x="20" y="10" z="0"/>
<vertex x="0" y="0" z="5"/><vertex x="20" y="0" z="5"/><vertex x="0" y="10" z="5"/><vertex x="20" y="10" z="5"/>
</vertices><triangles>
<triangle v1="0" v2="2" v3="3" paint_supports="4"/><triangle v1="0" v2="3" v3="1" paint_supports="8" paint_color="1C"/>
<triangle v1="4" v2="5" v3="7"/><triangle v1="4" v2="7" v3="6"/>
<triangle v1="0" v2="1" v3="5"/><triangle v1="0" v2="5" v3="4"/>
<triangle v1="2" v2="6" v3="7"/><triangle v1="2" v2="7" v3="3"/>
<triangle v1="0" v2="4" v3="6"/><triangle v1="0" v2="6" v3="2"/>
<triangle v1="1" v2="3" v3="7"/><triangle v1="1" v2="7" v3="5"/>
</triangles></mesh></object>
<object id="2" type="model"><mesh><vertices>
<vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="1" y="1" z="0"/>
<vertex x="0" y="0" z="1"/><vertex x="1" y="0" z="1"/><vertex x="0" y="1" z="1"/><vertex x="1" y="1" z="1"/>
</vertices><triangles>
<triangle v1="0" v2="2" v3="3"/><triangle v1="0" v2="3" v3="1"/><triangle v1="4" v2="5" v3="7"/><triangle v1="4" v2="7" v3="6"/>
<triangle v1="0" v2="1" v3="5"/><triangle v1="0" v2="5" v3="4"/><triangle v1="2" v2="6" v3="7"/><triangle v1="2" v2="7" v3="3"/>
<triangle v1="0" v2="4" v3="6"/><triangle v1="0" v2="6" v3="2"/><triangle v1="1" v2="3" v3="7"/><triangle v1="1" v2="7" v3="5"/>
</triangles></mesh></object>
</resources></model>"#;

fn core_model(item_transform: &str, body_transform: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06" requiredextensions="p"><resources>
<object id="3" type="model"><components>
<component p:path="/3D/Objects/object_1.model" objectid="1" transform="{body_transform}"/>
<component p:path="/3D/Objects/object_1.model" objectid="2" transform="1 0 0 0 1 0 0 0 1 5 2 1"/>
</components></object></resources>
<build><item objectid="3" transform="{item_transform}" printable="1"/></build></model>"#
    )
}

const MODEL_SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<config><object id="3"><metadata key="name" value="box"/>
<part id="1" subtype="normal_part"><metadata key="name" value="body"/></part>
<part id="2" subtype="negative_part"><metadata key="name" value="cut"/></part>
</object><plate><metadata key="plater_id" value="1"/></plate></config>"#;

fn write_3mf(path: &Path, item_transform: &str, body_transform: &str) {
    let mut archive = ZipWriter::new(File::create(path).unwrap());
    let core = core_model(item_transform, body_transform);
    for (name, content) in [
        ("3D/3dmodel.model", core.as_str()),
        ("3D/Objects/object_1.model", OBJECT_PART),
        ("Metadata/model_settings.config", MODEL_SETTINGS),
    ] {
        archive
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        archive.write_all(content.as_bytes()).unwrap();
    }
    archive.finish().unwrap();
}

#[test]
fn reads_every_part_with_kinds_paint_and_the_item_rotation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rotated.3mf");
    write_3mf(
        &path,
        "0 1 0 -1 0 0 0 0 1 100 50 3",
        "1 0 0 0 1 0 0 0 1 -10 -5 -3",
    );
    let mesh = load_object_mesh(&path).unwrap();
    assert_eq!(mesh.parts.len(), 2);
    assert_eq!(mesh.parts[0].kind, PartKind::Normal);
    assert_eq!(mesh.parts[1].kind, PartKind::Negative);
    assert_eq!(
        mesh.parts[0].paint_supports.get(&0).map(String::as_str),
        Some("4")
    );
    assert_eq!(
        mesh.parts[0].paint_supports.get(&1).map(String::as_str),
        Some("8")
    );
    assert_eq!(
        mesh.parts[0].paint_color.get(&1).map(String::as_str),
        Some("1C")
    );
    // The 20 x 10 body centred on the item turns a quarter: 10 wide, 20 deep, on the bed.
    let (min, max) = mesh.normal_bounds();
    assert_eq!((min, max), ([-5.0, -10.0, 0.0], [5.0, 10.0, 5.0]));
    assert_eq!(mesh.frame_origin, [100.0, 50.0, 0.0]);
    // The negative 1 mm cube at component offset (5, 2, 1) turns to x in [-3, -2], y in [5, 6].
    let cut = mesh.parts[1].bounds().unwrap();
    assert_eq!(cut, ([-3.0, 5.0, 4.0], [-2.0, 6.0, 5.0]));
    let section = object_section(&mesh, 4.5, &SectionOptions::EXACT);
    assert!(near(area(&section), 200.0 - 1.0, 1e-6));
}

#[test]
fn mirrored_component_keeps_sections_counter_clockwise() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mirrored.3mf");
    write_3mf(
        &path,
        "1 0 0 0 1 0 0 0 1 50 50 0",
        "-1 0 0 0 1 0 0 0 1 0 0 0",
    );
    let mesh = load_object_mesh(&path).unwrap();
    assert!(mesh.parts[0].mirrored);
    assert!(!mesh.parts[1].mirrored);
    let loops = raw_section_loops(&mesh.parts[0], 2.5);
    assert_eq!(loops.len(), 1);
    assert!(
        near(polygon_area(&loops[0]), 200.0, 1e-6),
        "solid stays on the left"
    );
}

#[test]
fn reading_is_deterministic() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("box.3mf");
    write_3mf(
        &path,
        "0.6 0.8 0 -0.8 0.6 0 0 0 1 10 20 0",
        "1 0 0 0 1 0 0 0 1 0 0 0",
    );
    let first = load_object_mesh(&path).unwrap();
    assert_eq!(load_object_mesh(&path).unwrap(), first);
}

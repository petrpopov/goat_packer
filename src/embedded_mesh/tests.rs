//! Unit tests of the first-component reader and the STL writer.

use super::*;
use std::path::PathBuf;
use zip::{ZipWriter, write::SimpleFileOptions};

/// A self-contained single-object 3MF whose component carries its own translation — the
/// structure that broke the real Goat 3 P1S pack: mesh centered on the local origin,
/// component transform translating it by (10, 20, -0.5). Without compensation every clone
/// lands offset from its packed placement and can fall off the bed.
fn synthetic_source_3mf(directory: &Path) -> PathBuf {
    const MESH_PART: &str = r#"<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/><vertex x="10" y="10" z="0"/><vertex x="0" y="10" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/><triangle v1="0" v2="2" v3="3"/></triangles></mesh></object></resources></model>"#;
    const CORE: &str = r#"<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06"><resources><object id="2" type="model"><components><component p:path="/3D/Objects/mesh.model" transform="1 0 0 0 1 0 0 0 1 10 20 -0.5"/></components></object></resources><build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 50 60 0.5"/></build></model>"#;
    let settings = serde_json::json!({
        "printer_model": "Synthetic Printer",
        "printable_area": ["0x0", "200x0", "200x200", "0x200"],
    })
    .to_string();
    let source = directory.join("synthetic_source.3mf");
    write_zip(
        &source,
        &[
            ("3D/3dmodel.model", CORE),
            ("3D/Objects/mesh.model", MESH_PART),
            ("Metadata/project_settings.config", settings.as_str()),
            (
                "Metadata/model_settings.config",
                r#"<config><object id="2"/><plate/></config>"#,
            ),
        ],
    );
    source
}

/// The same square mesh declared inline in the core model with no component at all:
/// exercises the identity-component-transform fallback.
fn inline_mesh_source_3mf(directory: &Path) -> PathBuf {
    const CORE: &str = r#"<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="2" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/><vertex x="10" y="10" z="0"/><vertex x="0" y="10" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/><triangle v1="0" v2="2" v3="3"/></triangles></mesh></object></resources><build><item objectid="2"/></build></model>"#;
    let source = directory.join("inline_source.3mf");
    write_zip(&source, &[("3D/3dmodel.model", CORE)]);
    source
}

fn write_zip(path: &Path, entries: &[(&str, &str)]) {
    let mut archive = ZipWriter::new(File::create(path).unwrap());
    for (name, content) in entries {
        archive
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        archive.write_all(content.as_bytes()).unwrap();
    }
    archive.finish().unwrap();
}

fn near(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 && (a.2 - b.2).abs() < 1e-9
}

/// Both directions must hold: every baked vertex sits on an expected corner and every
/// expected corner is covered (the two triangles share their corners).
fn assert_corner_set(actual: &[[(f64, f64, f64); 3]], expected: &[(f64, f64, f64)]) {
    for corner in actual.iter().flatten() {
        assert!(
            expected.iter().any(|point| near(*corner, *point)),
            "no expected corner near {corner:?}"
        );
    }
    for point in expected {
        assert!(
            actual.iter().flatten().any(|corner| near(*corner, *point)),
            "no baked vertex near {point:?}"
        );
    }
}

fn baked_square_stl(directory: &Path) -> PathBuf {
    let triangles = embedded_mesh_triangles(&synthetic_source_3mf(directory)).unwrap();
    let path = directory.join("baked.stl");
    write_binary_stl(&path, &triangles).unwrap();
    path
}

#[test]
fn embedded_mesh_triangles_bakes_the_component_transform() {
    let dir = tempfile::tempdir().unwrap();
    let triangles = embedded_mesh_triangles(&synthetic_source_3mf(dir.path())).unwrap();
    assert_eq!(triangles.len(), 2);
    assert_corner_set(
        &triangles,
        &[
            (10.0, 20.0, -0.5),
            (20.0, 20.0, -0.5),
            (20.0, 30.0, -0.5),
            (10.0, 30.0, -0.5),
        ],
    );
}

#[test]
fn embedded_mesh_triangles_falls_back_to_the_inline_core_mesh() {
    let dir = tempfile::tempdir().unwrap();
    let triangles = embedded_mesh_triangles(&inline_mesh_source_3mf(dir.path())).unwrap();
    assert_corner_set(
        &triangles,
        &[
            (0.0, 0.0, 0.0),
            (10.0, 0.0, 0.0),
            (10.0, 10.0, 0.0),
            (0.0, 10.0, 0.0),
        ],
    );
}

#[test]
fn write_binary_stl_matches_the_python_byte_layout() {
    let dir = tempfile::tempdir().unwrap();
    let triangles = embedded_mesh_triangles(&synthetic_source_3mf(dir.path())).unwrap();
    let path = dir.path().join("baked.stl");
    write_binary_stl(&path, &triangles).unwrap();
    let data = fs::read(&path).unwrap();
    assert_eq!(data.len(), 84 + 50 * triangles.len());
    assert!(
        data[..80].iter().all(|&byte| byte == 0),
        "header is 80 zero bytes"
    );
    assert_eq!(
        data[80..84],
        u32::try_from(triangles.len()).unwrap().to_le_bytes()
    );
    // The first triangle's payload pins the f64 → f32 conversion order the Python
    // struct.pack performs; the STL bytes must stay f32-identical to its output.
    let vertices: Vec<f32> = data[96..96 + 36]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();
    assert_eq!(
        vertices,
        vec![10.0f32, 20.0, -0.5, 20.0, 20.0, -0.5, 20.0, 30.0, -0.5]
    );
    for index in 0..triangles.len() {
        let base = 84 + index * 50;
        assert!(data[base..base + 12].iter().all(|&byte| byte == 0));
        assert_eq!(&data[base + 48..base + 50], &0u16.to_le_bytes());
    }
}

#[test]
fn extract_footprint_round_trips_the_written_stl() {
    let dir = tempfile::tempdir().unwrap();
    let path = baked_square_stl(dir.path());
    let footprint = crate::extract_footprint(&path).unwrap();
    assert_eq!(
        footprint.loops.len(),
        1,
        "flat square has one outer boundary"
    );
    // extract_footprint closes each loop by repeating its start point at the end.
    let mut corners = footprint.loops[0].clone();
    assert_eq!(corners.first(), corners.last(), "loop is closed");
    corners.pop();
    corners.dedup();
    assert_eq!(corners.len(), 4);
    assert!((footprint.width - 10.0).abs() < 1e-6);
    assert!((footprint.height - 10.0).abs() < 1e-6);
}

#[test]
fn stl_xy_center_finds_the_baked_square_center() {
    let dir = tempfile::tempdir().unwrap();
    let path = baked_square_stl(dir.path());
    let (x, y) = stl_xy_center(&path).unwrap();
    assert!((x - 15.0).abs() < 1e-9);
    assert!((y - 25.0).abs() < 1e-9);
}

#[test]
fn parse_transform_identity_on_empty() {
    let identity = parse_transform("").unwrap();
    assert_eq!(
        identity,
        [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0]
        ]
    );
    assert_eq!(
        apply_matrix(&identity, (3.5, -2.0, 7.25)),
        (3.5, -2.0, 7.25)
    );
}

#[test]
fn parse_transform_reads_the_translation_into_the_last_row() {
    let matrix = parse_transform("1 0 0 0 1 0 0 0 1 10 20 -0.5").unwrap();
    assert_eq!(matrix[0], [1.0, 0.0, 0.0]);
    assert_eq!(matrix[1], [0.0, 1.0, 0.0]);
    assert_eq!(matrix[2], [0.0, 0.0, 1.0]);
    assert_eq!(matrix[3], [10.0, 20.0, -0.5]);
    assert!(near(
        apply_matrix(&matrix, (0.0, 0.0, 0.0)),
        (10.0, 20.0, -0.5)
    ));
}

#[test]
fn apply_matrix_follows_the_row_vector_convention() {
    // 90° CCW rotation about Z. With row vectors (1,0,0)·M is the first ROW of M, so the
    // basis vector must land on (0, 1, 0); a column-vector reading would give (0, -1, 0).
    let matrix = parse_transform("0 1 0 -1 0 0 0 0 1 0 0 0").unwrap();
    assert!(near(
        apply_matrix(&matrix, (1.0, 0.0, 0.0)),
        (0.0, 1.0, 0.0)
    ));
}

#[test]
fn parse_transform_rejects_other_than_twelve_numbers() {
    assert!(parse_transform("1 0 0").is_err());
    assert!(parse_transform("1 0 0 0 1 0 0 0 1 0 0 0 5").is_err());
    assert!(parse_transform("1 0 0 0 1 0 0 0 1 0 0 x").is_err());
}

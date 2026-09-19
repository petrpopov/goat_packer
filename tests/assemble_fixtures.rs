//! Fixture-dependent assembly tests, ported from the former `tests/test_build_3mf.py`.
//!
//! Like `tests/quality.rs`, these read external assets under `../Goat 3/` and `../Goat 4/`
//! (relative to the repo root). That directory is not part of this repo; when it is absent the
//! tests fail at runtime — that is the documented convention, they must merely compile.

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use goat_packer::{PackedPlacement, Rect, assemble::assemble_project, assemble::AssembleOptions};
use quick_xml::{Reader, events::Event};
use serde_json::json;
use zip::ZipArchive;

/// The Python suite's shared two-placement layout (its `placements_file` helper).
fn fixture_placements() -> Vec<PackedPlacement> {
    vec![
        PackedPlacement { x_mm: 60.0, y_mm: 70.0, angle_deg: 0.0 },
        PackedPlacement { x_mm: 150.0, y_mm: 160.0, angle_deg: 90.0 },
    ]
}

/// The placements JSON the verifier binary reads (same content the Python tests wrote).
fn write_placements_file(directory: &Path) -> PathBuf {
    let path = directory.join("placements.json");
    std::fs::write(
        &path,
        json!({
            "placements": [
                {"x_mm": 60.0, "y_mm": 70.0, "angle_deg": 0.0},
                {"x_mm": 150.0, "y_mm": 160.0, "angle_deg": 90.0},
            ],
            "resolution_mm": 0.25,
            "requested_clearance_mm": 0.7,
            "angle_step_deg": 2.0,
            "attempts": 10,
            "seed": 1,
        })
        .to_string(),
    )
    .unwrap();
    path
}

/// Mirrors `build_project`'s own fallback when no `--stl` is passed: `goat4_v2.stl` next to the
/// template, else `../source/goat4_v2.stl` relative to it.
fn resolved_goat4_stl(source: &Path) -> PathBuf {
    let candidates = [
        source.parent().unwrap().join("goat4_v2.stl"),
        source
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("source")
            .join("goat4_v2.stl"),
    ];
    candidates
        .iter()
        .find(|candidate| candidate.exists())
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

fn base_options(stl: &Path) -> AssembleOptions {
    AssembleOptions {
        placements: fixture_placements(),
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

// ---------------------------------------------------------------------------
// zip / xml / png helpers
// ---------------------------------------------------------------------------

fn open_archive(path: &Path) -> ZipArchive<File> {
    ZipArchive::new(File::open(path).unwrap()).unwrap()
}

fn read_entry(archive: &mut ZipArchive<File>, name: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    archive
        .by_name(name)
        .unwrap_or_else(|error| panic!("no entry {name:?}: {error}"))
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

fn has_entry(archive: &mut ZipArchive<File>, name: &str) -> bool {
    archive.by_name(name).is_ok()
}

fn entry_names(archive: &mut ZipArchive<File>) -> Vec<String> {
    (0..archive.len())
        .map(|index| archive.by_index(index).unwrap().name().to_owned())
        .collect()
}

/// The `testzip()` equivalent: the zip crate verifies each entry's CRC while it is read, so
/// reading every entry to end surfaces any corruption.
fn assert_zip_crc_ok(archive: &mut ZipArchive<File>) {
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
    }
}

/// A parsed XML part with the handful of lookups the Python did via ElementTree.
struct Doc {
    events: Vec<Event<'static>>,
}

impl Doc {
    fn parse(bytes: &[u8]) -> Self {
        let mut reader = Reader::from_reader(bytes);
        let mut events = Vec::new();
        loop {
            match reader.read_event().unwrap() {
                Event::Eof => break,
                event => events.push(event.into_owned()),
            }
        }
        Self { events }
    }

    /// Direct children of the element chain `ancestors` (e.g. `["resources"]` for
    /// `resources/object`, `["plate"]` for `plate/model_instance`).
    fn count_under(&self, ancestors: &[&str], name: &str) -> usize {
        let mut stack: Vec<String> = Vec::new();
        let mut matches = 0usize;
        for event in &self.events {
            match event {
                Event::Start(start) => {
                    let name_owned = start.local_name().into_inner().to_owned();
                    if name_owned == name && stack_ends_with(&stack, ancestors) {
                        matches += 1;
                    }
                    stack.push(name_owned);
                }
                Event::Empty(start) => {
                    if start.local_name().into_inner() == name && stack_ends_with(&stack, ancestors)
                    {
                        matches += 1;
                    }
                }
                Event::End(_) => {
                    stack.pop();
                }
                _ => {}
            }
        }
        matches
    }

    /// Top-level children of the document root with the given local name (ElementTree's
    /// `findall("object")` on the parsed config root).
    fn count_top_level(&self, name: &str) -> usize {
        self.count_under(&[], name)
    }

    /// Values of `attribute` on every `element` anywhere in the document, in document order.
    fn attribute_values(&self, element: &str, attribute: &str) -> Vec<String> {
        let mut values = Vec::new();
        for event in &self.events {
            let (Event::Start(start) | Event::Empty(start)) = event else { continue };
            if start.local_name().into_inner() != element {
                continue;
            }
            for candidate in start.attributes() {
                let candidate = candidate.unwrap();
                if candidate.key.as_ref() == attribute {
                    values.push(candidate.value.into_owned());
                }
            }
        }
        values
    }
}

fn stack_ends_with(stack: &[String], suffix: &[&str]) -> bool {
    stack.len() == suffix.len()
        && stack
            .iter()
            .zip(suffix)
            .all(|(entry, expected)| entry.as_str() == *expected)
}

/// Reads a PNG's IHDR width/height by hand (the suite deliberately has no image crate).
fn assert_png_size(png: &[u8], expected: (u32, u32)) {
    assert!(
        png.len() > 24 && png[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        "not a PNG"
    );
    assert_eq!(&png[12..16], b"IHDR");
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!((width, height), expected);
}

// ---------------------------------------------------------------------------
// shared assertions (the Python `assert_common_integrity`)
// ---------------------------------------------------------------------------

/// The compensated build-item transform formula: rotation about the local origin with the STL's
/// XY bbox center rotated out of the translation (see `assemble::item_transform`).
fn expected_item_transform(
    placement: &PackedPlacement,
    center: (f64, f64),
    z_offset: f64,
) -> [f64; 12] {
    let angle = placement.angle_deg.to_radians();
    let (s, c) = angle.sin_cos();
    [
        c,
        s,
        0.0,
        -s,
        c,
        0.0,
        0.0,
        0.0,
        1.0,
        placement.x_mm - (center.0 * c - center.1 * s),
        placement.y_mm - (center.0 * s + center.1 * c),
        z_offset,
    ]
}

fn assert_transform_matches(transform: &str, expected: [f64; 12]) {
    let actual: Vec<f64> = transform
        .split_whitespace()
        .map(|value| value.parse().unwrap())
        .collect();
    assert_eq!(actual.len(), 12);
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "transform value {actual} != {expected}"
        );
    }
}

/// Everything every build asserts: CRC, byte-identical project settings, resource/item
/// cardinality, exact compensated item transforms, model_settings cardinality, relationship
/// targets, and the five preview PNGs at their expected sizes.
fn assert_common_integrity(source: &Path, output: &Path, stl: &Path) {
    let center = goat_packer::embedded_mesh::stl_xy_center(stl).unwrap();
    // The Python hardcoded the Goat 4 source item's z translation here.
    let z_offset = 3.70000005;

    let mut original = open_archive(source);
    let mut built = open_archive(output);
    assert_zip_crc_ok(&mut built);
    assert_eq!(
        read_entry(&mut original, "Metadata/project_settings.config"),
        read_entry(&mut built, "Metadata/project_settings.config"),
        "project_settings.config must be carried through byte-for-byte"
    );

    let core = Doc::parse(&read_entry(&mut built, "3D/3dmodel.model"));
    assert_eq!(core.count_under(&["resources"], "object"), 2);
    assert_eq!(core.count_under(&["build"], "item"), 2);
    let transforms = core.attribute_values("item", "transform");
    assert_transform_matches(&transforms[0], expected_item_transform(&fixture_placements()[0], center, z_offset));
    assert_transform_matches(&transforms[1], expected_item_transform(&fixture_placements()[1], center, z_offset));

    let settings = Doc::parse(&read_entry(&mut built, "Metadata/model_settings.config"));
    assert_eq!(settings.count_top_level("object"), 2);
    assert_eq!(settings.count_under(&["plate"], "model_instance"), 2);

    let names = entry_names(&mut built);
    let relationships = Doc::parse(&read_entry(&mut built, "3D/_rels/3dmodel.model.rels"));
    for target in relationships.attribute_values("Relationship", "Target") {
        let target = target.trim_start_matches('/');
        assert!(
            names.iter().any(|name| name == target),
            "relationship target {target:?} missing from the archive"
        );
    }

    for (name, size) in [
        ("Metadata/plate_1.png", (512, 512)),
        ("Metadata/plate_1_small.png", (128, 128)),
        ("Metadata/plate_no_light_1.png", (512, 512)),
        ("Metadata/top_1.png", (512, 512)),
        ("Metadata/pick_1.png", (512, 512)),
    ] {
        assert_png_size(&read_entry(&mut built, name), size);
    }
}

/// The production-namespace attribute name on `<component>`.
const PRODUCTION_PATH: &str = "p:path";

fn component_paths(core: &Doc) -> Vec<String> {
    core.attribute_values("component", PRODUCTION_PATH)
        .into_iter()
        .map(|path| path.trim_start_matches('/').to_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// tests (one per ported Python test)
// ---------------------------------------------------------------------------

/// Port of `test_bambu_build_updates_optional_bambu_metadata`.
#[test]
fn bambu_build_updates_optional_bambu_metadata() {
    let source = Path::new("../Goat 4/p1s/goat4_p1s_contour_packed_purge_safe_faster.3mf");
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("bambu.3mf");
    let stl = resolved_goat4_stl(source);
    assemble_project(source, &output, &base_options(&stl)).unwrap();
    assert_common_integrity(source, &output, &stl);

    let mut built = open_archive(&output);
    let cut = Doc::parse(&read_entry(&mut built, "Metadata/cut_information.xml"));
    assert_eq!(cut.count_top_level("object"), 2);
    let plate: serde_json::Value =
        serde_json::from_slice(&read_entry(&mut built, "Metadata/plate_1.json")).unwrap();
    assert_eq!(
        plate["bbox_objects"].as_array().map(Vec::len),
        Some(2),
        "{plate}"
    );
}

/// Port of `test_snap_build_preserves_optional_metadata_presence`.
#[test]
fn snap_build_preserves_optional_metadata_presence() {
    let source = Path::new("../Goat 4/snap/goat4_snapmaker_u1_contour_packed_purge_safe_faster.3mf");
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("snap.3mf");
    let stl = resolved_goat4_stl(source);
    assemble_project(source, &output, &base_options(&stl)).unwrap();
    assert_common_integrity(source, &output, &stl);

    // Contract 6: rewrite-only-if-present — the Snapmaker template's metadata presence must be
    // preserved exactly (never invent Bambu-only metadata).
    let mut original = open_archive(source);
    let mut built = open_archive(&output);
    for name in ["Metadata/cut_information.xml", "Metadata/plate_1.json"] {
        assert_eq!(
            has_entry(&mut original, name),
            has_entry(&mut built, name),
            "{name} presence must match the source"
        );
    }
}

/// Port of `test_colored_source_mesh_is_reused_without_touching_paint_attributes`.
#[test]
fn colored_source_mesh_is_reused_without_touching_paint_attributes() {
    let source = Path::new("../Goat 3/p1s/goat3_fast_max.3mf");
    let stl = Path::new("../Goat 3/source/goat3.stl");
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("colored.3mf");
    assemble_project(source, &output, &base_options(stl)).unwrap();

    let mut original = open_archive(source);
    let mut built = open_archive(&output);
    let source_core = Doc::parse(&read_entry(&mut original, "3D/3dmodel.model"));
    let built_core = Doc::parse(&read_entry(&mut built, "3D/3dmodel.model"));

    let source_paths = component_paths(&source_core);
    assert_eq!(source_paths.len(), 1, "the source has one shared mesh part");
    let source_path = &source_paths[0];
    let built_paths = component_paths(&built_core);
    assert_eq!(built_paths.len(), 2, "one component per packed object");
    assert!(
        built_paths.iter().all(|path| path == source_path),
        "every clone must point at the source mesh part: {built_paths:?}"
    );
    // Contract 2: the shared mesh part is byte-identical, `paint_color` included.
    let source_mesh = read_entry(&mut original, source_path);
    assert_eq!(source_mesh, read_entry(&mut built, source_path));
    assert!(
        source_mesh.windows(b"paint_color".len()).any(|window| window == b"paint_color"),
        "the source mesh part carries paint attributes"
    );
}

/// Port of `test_preview_accepts_custom_keepout_with_y_axis_flip`.
#[test]
fn preview_accepts_custom_keepout_with_y_axis_flip() {
    let source = Path::new("../Goat 3/p1s/goat3_fast_max.3mf");
    let stl = Path::new("../Goat 3/source/goat3.stl");
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("keepout_preview.3mf");
    let mut options = base_options(stl);
    options.keepouts = vec![Rect::new(226.85, 212.35, 245.15, 248.65)];
    assemble_project(source, &output, &options).unwrap();

    let mut built = open_archive(&output);
    assert_png_size(&read_entry(&mut built, "Metadata/plate_1.png"), (512, 512));
}

/// Port of `test_colored_clone_can_duplicate_mesh_part_per_object`.
#[test]
fn colored_clone_can_duplicate_mesh_part_per_object() {
    let source = Path::new("../Goat 3/snap/goat3_snap.3mf");
    let stl = Path::new("../Goat 3/source/goat3.stl");
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("duplicate_mesh.3mf");
    let mut options = base_options(stl);
    options.duplicate_mesh_per_object = true;
    assemble_project(source, &output, &options).unwrap();

    let mut built = open_archive(&output);
    let core = Doc::parse(&read_entry(&mut built, "3D/3dmodel.model"));
    let paths = component_paths(&core);
    assert_eq!(paths.len(), 2);
    assert_ne!(paths[0], paths[1], "each object must reference its own mesh part");
    assert!(
        read_entry(&mut built, &paths[0])
            .windows(b"paint_color".len())
            .any(|window| window == b"paint_color"),
        "the duplicated mesh part keeps the paint attributes"
    );
    // Contract 2: the duplicate is a byte copy of the source mesh part.
    assert_eq!(read_entry(&mut built, &paths[0]), read_entry(&mut built, &paths[1]));
}

/// Port of `test_colored_clone_can_use_standard_3mf_instances`.
#[test]
fn colored_clone_can_use_standard_3mf_instances() {
    let source = Path::new("../Goat 3/snap/goat3_snap.3mf");
    let stl = Path::new("../Goat 3/source/goat3.stl");
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("instances.3mf");
    let mut options = base_options(stl);
    options.reuse_source_object_instances = true;
    assemble_project(source, &output, &options).unwrap();

    {
        let mut built = open_archive(&output);
        let core = Doc::parse(&read_entry(&mut built, "3D/3dmodel.model"));
        assert_eq!(core.count_under(&["resources"], "object"), 1, "one shared resource");
        let object_ids = core.attribute_values("item", "objectid");
        assert_eq!(object_ids, vec!["2", "2"]);
        let settings = Doc::parse(&read_entry(&mut built, "Metadata/model_settings.config"));
        assert_eq!(settings.attribute_values("object", "id"), vec!["2"]);
        assert_eq!(
            settings.count_under(&["plate"], "model_instance"),
            1,
            "preserve Orca-compatible source metadata"
        );
    }

    // Independent verifier check: the built project holds exactly the two placements.
    let placements = write_placements_file(dir.path());
    let report = Command::new(env!("CARGO_BIN_EXE_verify_outputs"))
        .arg(stl)
        .arg(&placements)
        .arg(&output)
        .args(["--minimum-gap", "0.0", "--edge-margin", "0.0"])
        .output()
        .expect("verify_outputs runs");
    assert!(
        report.status.success(),
        "verify_outputs failed: {}",
        String::from_utf8_lossy(&report.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(report["count"], 2, "{report}");
}

/// Port of `test_colored_clone_preserves_source_z_offset`.
#[test]
fn colored_clone_preserves_source_z_offset() {
    let source = Path::new("../Goat 3/snap/goat3_snap.3mf");
    let stl = Path::new("../Goat 3/source/goat3.stl");
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("source_z.3mf");
    let mut options = base_options(stl);
    options.reuse_source_object_instances = true;
    assemble_project(source, &output, &options).unwrap();

    let mut built = open_archive(&output);
    let core = Doc::parse(&read_entry(&mut built, "3D/3dmodel.model"));
    let z_values: Vec<f64> = core
        .attribute_values("item", "transform")
        .iter()
        .map(|transform| {
            transform
                .split_whitespace()
                .nth(11)
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    assert_eq!(z_values.len(), 2);
    for z in z_values {
        assert!((z - 4.30000019).abs() < 1e-6, "item z offset {z} != 4.30000019");
    }
}

//! Self-contained regression tests for the plate-grid offsets `build_schedule_plates_3mf` gives
//! every plate beyond the first. BambuStudio assigns objects to plates purely by grid geometry at
//! slice time, so a wrong column count silently throws a whole plate's objects off the sheet —
//! these tests lock the grid to what real BambuStudio-saved projects use (see the comment on
//! `cols` in `src/scheduler.rs`).

use goat_packer::scheduler::{PlateSpec, build_schedule_plates_3mf};
use std::io::Read;
use xmltree::Element;
use zip::ZipArchive;

const BED_WIDTH_MM: f64 = 256.0;
const BED_DEPTH_MM: f64 = 256.0;

/// Two plates is the most common schedule shape (one day batch + the overnight batch) and the
/// case `round(sqrt(n))` got wrong: it stacked plate 2 below plate 1 while BambuStudio's grid
/// puts it to the right (`ceil(sqrt(2)) = 2` columns). Plate 2's items must move by exactly
/// (+bed*1.2, 0).
#[test]
fn two_plates_lay_out_horizontally() {
    let output = build_two_plate_file();

    let items = build_items(output.path());
    assert_eq!(items.len(), 3, "2 + 1 objects across two plates");

    let stride = BED_WIDTH_MM * 1.2;
    assert_translation(
        &items[0],
        10.0,
        20.0,
        "plate 1 item 1 stays at the origin slot",
    );
    assert_translation(
        &items[1],
        30.0,
        40.0,
        "plate 1 item 2 stays at the origin slot",
    );
    assert_translation(
        &items[2],
        10.0 + stride,
        20.0,
        "plate 2 shifts right by one bed*1.2 slot, not down",
    );

    // Every plate needs its own wipe-tower entry (same bed coordinates as the source — the
    // tower entry is used verbatim per plate, not shifted with the plate's grid slot), or
    // BambuStudio auto-places plate 2's tower on top of packed objects. See
    // patch_wipe_tower_per_plate for both measured failure modes.
    let settings = read_project_settings(output.path());
    assert_eq!(
        settings["wipe_tower_x"],
        serde_json::json!(["230.962", "230.962"]),
        "one identical tower entry per plate"
    );
    assert_eq!(settings["wipe_tower_y"], serde_json::json!(["160", "160"]));
    assert_eq!(
        settings["machine_start_gcode"], "G28 ; keep",
        "blank_start_gcode=false leaves the start gcode untouched"
    );
}

/// Five plates exercise the wrap: `ceil(sqrt(5)) = 3` columns, row-major, rows downward —
/// plate 4 wraps to (row 1, col 0) at dy = -bed*1.2.
#[test]
fn five_plates_wrap_into_a_three_column_grid() {
    let source = synthetic_source_3mf(6);
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("five_plates.3mf");
    let plates: Vec<PlateSpec> = [6usize, 5, 4, 3, 6]
        .iter()
        .enumerate()
        .map(|(index, &count)| PlateSpec {
            label: format!("plate {}", index + 1),
            object_count: count,
        })
        .collect();
    build_schedule_plates_3mf(
        source.path(),
        &output,
        &plates,
        BED_WIDTH_MM,
        BED_DEPTH_MM,
        false,
    )
    .unwrap();

    let items = build_items(&output);
    let stride_x = BED_WIDTH_MM * 1.2;
    let stride_y = BED_DEPTH_MM * 1.2;
    // Source items sit at x = 10, 30, 50, ... (item i at (10 + 20i, 20 + 20i)); plate p keeps
    // the first N of them, so its first item is always the source's item 0 at (10, 20).
    let expected = [
        (0.0, 0.0),            // plate 1: (row 0, col 0)
        (stride_x, 0.0),       // plate 2: (row 0, col 1)
        (2.0 * stride_x, 0.0), // plate 3: (row 0, col 2)
        (0.0, -stride_y),      // plate 4: (row 1, col 0)
        (stride_x, -stride_y), // plate 5: (row 1, col 1)
    ];
    let mut cursor = 0usize;
    for (plate_index, plate) in plates.iter().enumerate() {
        let item = &items[cursor];
        let (dx, dy) = expected[plate_index];
        assert_translation(
            item,
            10.0 + dx,
            20.0 + dy,
            &format!("plate {} first item", plate_index + 1),
        );
        cursor += plate.object_count;
    }
}

fn build_two_plate_file() -> tempfile::NamedTempFile {
    let source = synthetic_source_3mf(2);
    let dir = tempfile::tempdir().unwrap();
    // Keep the tempdir alive by returning a file inside it: copy out instead, so the returned
    // path outlives this function.
    let temp_output = dir.path().join("two_plates.3mf");
    let plates = [
        PlateSpec {
            label: "2 pcs".to_owned(),
            object_count: 2,
        },
        PlateSpec {
            label: "1 pc".to_owned(),
            object_count: 1,
        },
    ];
    build_schedule_plates_3mf(
        source.path(),
        &temp_output,
        &plates,
        BED_WIDTH_MM,
        BED_DEPTH_MM,
        false,
    )
    .unwrap();
    let output = tempfile::NamedTempFile::new().unwrap();
    std::fs::copy(&temp_output, output.path()).unwrap();
    output
}

/// Minimal single-plate 3MF with `object_count` packed build items: exactly the entries
/// `build_schedule_plates_3mf` reads (core model objects/build items, model_settings objects,
/// one plate, one assemble with an assemble_item per object). Item i sits at
/// (10 + 20i, 20 + 20i) with identity rotation.
fn synthetic_source_3mf(object_count: usize) -> tempfile::NamedTempFile {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic_packed.3mf");

    let mut objects = String::new();
    let mut items = String::new();
    let mut config_objects = String::new();
    let mut assemble_items = String::new();
    let mut instances = String::new();
    for index in 1..=object_count {
        let id = index;
        let x = 10.0 + 20.0 * (index - 1) as f64;
        let y = 20.0 + 20.0 * (index - 1) as f64;
        objects.push_str(&format!(
            "<object id=\"{id}\" type=\"model\" p:UUID=\"1000000{id:04}\">\
             <metadata name=\"Title\">synthetic {id}</metadata>\
             <components><component objectid=\"900{id:04}\" p:path=\"3D/Objects/mesh{id}.stl\" \
             transform=\"1 0 0 0 1 0 0 0 1 0 0 0\"/></components></object>"
        ));
        items.push_str(&format!(
            "<item objectid=\"{id}\" p:UUID=\"2000000{id:04}\" \
             transform=\"1 0 0 0 1 0 0 0 1 {x} {y} 1\"/>"
        ));
        config_objects.push_str(&format!(
            "<object id=\"{id}\"><metadata key=\"name\" value=\"obj {id}\"/></object>"
        ));
        assemble_items.push_str(&format!(
            "<assemble_item object_id=\"{id}\" partserialnum=\"1\"/>"
        ));
        instances.push_str(&format!(
            "<model_instance><metadata key=\"object_id\" value=\"{id}\"/>\
             <metadata key=\"instance_id\" value=\"0\"/>\
             <metadata key=\"identify_id\" value=\"{id}0\"/></model_instance>"
        ));
    }

    let core = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <model unit=\"millimeter\" xml:lang=\"en-US\" \
         xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\" \
         xmlns:p=\"http://schemas.microsoft.com/3dmanufacturing/production/2015/06\">\
         <resources>{objects}</resources>\
         <build>{items}</build></model>"
    );
    let model_settings = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <config><plate><metadata key=\"plater_id\" value=\"1\"/>\
         <metadata key=\"plater_name\" value=\"Synthetic\"/>{instances}</plate>\
         {config_objects}<assemble>{assemble_items}</assemble></config>"
    );

    let file = std::fs::File::create(&path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer.start_file("3D/3dmodel.model", options).unwrap();
    std::io::Write::write_all(&mut writer, core.as_bytes()).unwrap();
    writer
        .start_file("Metadata/model_settings.config", options)
        .unwrap();
    std::io::Write::write_all(&mut writer, model_settings.as_bytes()).unwrap();
    let project_settings = serde_json::json!({
        "wipe_tower_x": ["230.962"],
        "wipe_tower_y": ["160"],
        "machine_start_gcode": "G28 ; keep",
    });
    writer
        .start_file("Metadata/project_settings.config", options)
        .unwrap();
    std::io::Write::write_all(&mut writer, project_settings.to_string().as_bytes()).unwrap();
    writer.finish().unwrap();

    let keep = tempfile::NamedTempFile::new().unwrap();
    std::fs::copy(&path, keep.path()).unwrap();
    keep
}

fn build_items(output: &std::path::Path) -> Vec<Element> {
    let mut zip = ZipArchive::new(std::fs::File::open(output).unwrap()).unwrap();
    let mut data = Vec::new();
    zip.by_name("3D/3dmodel.model")
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    let core = Element::parse(data.as_slice()).unwrap();
    core.get_child("build")
        .unwrap()
        .children
        .iter()
        .filter_map(xmltree::XMLNode::as_element)
        .filter(|element| element.name == "item")
        .cloned()
        .collect()
}

fn assert_translation(item: &Element, expected_x: f64, expected_y: f64, context: &str) {
    let values: Vec<f64> = item.attributes["transform"]
        .split_whitespace()
        .map(|value| value.parse().unwrap())
        .collect();
    assert!(
        (values[9] - expected_x).abs() < 1e-6,
        "{context}: x {} != expected {expected_x}",
        values[9]
    );
    assert!(
        (values[10] - expected_y).abs() < 1e-6,
        "{context}: y {} != expected {expected_y}",
        values[10]
    );
}

fn read_project_settings(output: &std::path::Path) -> serde_json::Value {
    let mut zip = ZipArchive::new(std::fs::File::open(output).unwrap()).unwrap();
    let mut data = Vec::new();
    zip.by_name("Metadata/project_settings.config")
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    serde_json::from_slice(&data).unwrap()
}

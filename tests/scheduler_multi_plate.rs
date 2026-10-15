use goat_packer::scheduler::{PlateSpec, build_schedule_plates_3mf};
use std::collections::HashSet;
use std::io::Read;
use xmltree::Element;
use zip::ZipArchive;

const BED_WIDTH_MM: f64 = 256.0;
const BED_DEPTH_MM: f64 = 256.0;

#[test]
fn plates_are_disjoint_and_land_in_bambustudios_own_plate_grid() {
    let source = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/packed.3mf"
    ));
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("schedule_plates.3mf");

    let plates = [
        PlateSpec {
            label: "18 pcs".to_owned(),
            object_count: 18,
        },
        PlateSpec {
            label: "26 pcs".to_owned(),
            object_count: 26,
        },
        PlateSpec {
            label: "29 pcs".to_owned(),
            object_count: 29,
        },
    ];
    build_schedule_plates_3mf(source, &output, &plates, BED_WIDTH_MM, BED_DEPTH_MM, false).unwrap();

    let source_core = read_xml_from_path(source, "3D/3dmodel.model");
    let source_build = source_core.get_child("build").unwrap();
    let source_transforms: Vec<String> = named_children(source_build, "item")
        .iter()
        .map(|item| item.attributes["transform"].clone())
        .collect();

    let mut zip = ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
    let core = read_xml(&mut zip, "3D/3dmodel.model");
    let build = core.get_child("build").unwrap();
    let items = named_children(build, "item");
    assert_eq!(items.len(), 18 + 26 + 29);

    // Every build item id is unique across all plates: no two plates share an object.
    let ids: Vec<&str> = items
        .iter()
        .map(|item| item.attributes["objectid"].as_str())
        .collect();
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(
        unique.len(),
        ids.len(),
        "object ids must not repeat across plates"
    );

    // BambuStudio places plate i's objects in a `ceil(sqrt(plate_count))`-column row-major grid,
    // each slot offset by bed_size * 1.2 — it assigns an object to a plate purely by testing its
    // bounding box against that grid at slice time, never by reading <plate>/<model_instance>.
    // cols = ceil(sqrt(3)) = 2, so plate 1 sits at the grid origin, plate 2 shifts by (bed*1.2, 0),
    // and plate 3 wraps to (0, -bed*1.2).
    let plate_x = |item: &&Element| -> f64 {
        item.attributes["transform"]
            .split_whitespace()
            .nth(9)
            .unwrap()
            .parse()
            .unwrap()
    };
    let plate_y = |item: &&Element| -> f64 {
        item.attributes["transform"]
            .split_whitespace()
            .nth(10)
            .unwrap()
            .parse()
            .unwrap()
    };
    let stride = BED_WIDTH_MM * 1.2;
    // cols = ceil(sqrt(3)) = 2: plate 0 -> (row 0, col 0), plate 1 -> (row 0, col 1),
    // plate 2 -> (row 1, col 0).
    let grid = [(0usize, 0usize), (0, 1), (1, 0)];
    let mut cursor = 0;
    for (plate_index, plate) in plates.iter().enumerate() {
        let (row, col) = grid[plate_index];
        let dx = col as f64 * stride;
        let dy = -(row as f64) * stride;
        for offset in 0..plate.object_count {
            let item = &items[cursor + offset];
            let source_values: Vec<f64> = source_transforms[offset]
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            let expected_x = source_values[9] + dx;
            let expected_y = source_values[10] + dy;
            assert!(
                (plate_x(&item) - expected_x).abs() < 1e-6,
                "plate {plate_index} object {offset}: x {} != expected {expected_x}",
                plate_x(&item)
            );
            assert!(
                (plate_y(&item) - expected_y).abs() < 1e-6,
                "plate {plate_index} object {offset}: y {} != expected {expected_y}",
                plate_y(&item)
            );
        }
        cursor += plate.object_count;
    }

    let resources = core.get_child("resources").unwrap();
    let resource_ids: HashSet<&str> = named_children(resources, "object")
        .iter()
        .map(|object| object.attributes["id"].as_str())
        .collect();
    assert_eq!(
        resource_ids, unique,
        "every build item must resolve to a resource object"
    );

    let model = read_xml(&mut zip, "Metadata/model_settings.config");
    let plate_elements = named_children(&model, "plate");
    assert_eq!(plate_elements.len(), 3);
    let mut plate_counts = Vec::new();
    for (index, plate) in plate_elements.iter().enumerate() {
        let plater_id = metadata_value(plate, "plater_id").unwrap();
        assert_eq!(plater_id, (index + 1).to_string());
        let plater_name = metadata_value(plate, "plater_name").unwrap();
        assert_eq!(plater_name, plates[index].label);
        plate_counts.push(named_children(plate, "model_instance").len());
    }
    assert_eq!(plate_counts, vec![18, 26, 29]);

    let assemble = model.get_child("assemble").unwrap();
    let assemble_ids: HashSet<&str> = named_children(assemble, "assemble_item")
        .iter()
        .map(|item| item.attributes["object_id"].as_str())
        .collect();
    assert_eq!(assemble_ids, unique);

    let config_ids: HashSet<&str> = named_children(&model, "object")
        .iter()
        .map(|object| object.attributes["id"].as_str())
        .collect();
    assert_eq!(config_ids, unique);
}

#[test]
fn rejects_a_plate_larger_than_the_source() {
    let source = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/packed.3mf"
    ));
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("too_big.3mf");
    let plates = [PlateSpec {
        label: "999 pcs".to_owned(),
        object_count: 999,
    }];
    let error =
        build_schedule_plates_3mf(source, &output, &plates, BED_WIDTH_MM, BED_DEPTH_MM, false)
            .unwrap_err();
    assert!(error.to_string().contains("999"));
}

fn read_xml<R: std::io::Read + std::io::Seek>(zip: &mut ZipArchive<R>, name: &str) -> Element {
    let mut data = Vec::new();
    zip.by_name(name).unwrap().read_to_end(&mut data).unwrap();
    Element::parse(data.as_slice()).unwrap()
}

fn read_xml_from_path(path: &std::path::Path, name: &str) -> Element {
    let mut zip = ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    read_xml(&mut zip, name)
}

fn named_children<'a>(element: &'a Element, name: &str) -> Vec<&'a Element> {
    element
        .children
        .iter()
        .filter_map(|node| node.as_element())
        .filter(|node| node.name == name)
        .collect()
}

fn metadata_value<'a>(element: &'a Element, key: &str) -> Option<&'a str> {
    named_children(element, "metadata")
        .into_iter()
        .find(|metadata| metadata.attributes.get("key").map(String::as_str) == Some(key))
        .and_then(|metadata| metadata.attributes.get("value"))
        .map(String::as_str)
}

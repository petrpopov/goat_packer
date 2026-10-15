use goat_packer::scheduler::subset_3mf;
use std::io::Read;
use xmltree::Element;
use zip::ZipArchive;

#[test]
fn subset_keeps_core_and_model_settings_referentially_consistent() {
    let source = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/packed.3mf"
    ));
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("two_objects.3mf");

    let kept = subset_3mf(source, &output, 2, false).unwrap();
    assert_eq!(kept, vec!["2", "3"]);

    let mut source_zip = ZipArchive::new(std::fs::File::open(source).unwrap()).unwrap();
    let mut output_zip = ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
    let mut source_settings = Vec::new();
    source_zip
        .by_name("Metadata/project_settings.config")
        .unwrap()
        .read_to_end(&mut source_settings)
        .unwrap();
    let mut output_settings = Vec::new();
    output_zip
        .by_name("Metadata/project_settings.config")
        .unwrap()
        .read_to_end(&mut output_settings)
        .unwrap();
    assert_eq!(source_settings, output_settings);

    let core = read_xml(&mut output_zip, "3D/3dmodel.model");
    let raw_core = String::from_utf8(read_zip_entry(&output, "3D/3dmodel.model")).unwrap();
    assert!(raw_core.contains("p:path="));
    assert!(raw_core.contains("p:UUID="));
    assert!(raw_core.contains("xml:lang="));
    let resources = core.get_child("resources").unwrap();
    assert_eq!(named_children(resources, "object").len(), 2);
    let build = core.get_child("build").unwrap();
    let build_ids: Vec<_> = named_children(build, "item")
        .iter()
        .map(|node| node.attributes["objectid"].as_str())
        .collect();
    assert_eq!(build_ids, ["2", "3"]);

    let model = read_xml(&mut output_zip, "Metadata/model_settings.config");
    let config_ids: Vec<_> = named_children(&model, "object")
        .iter()
        .map(|node| node.attributes["id"].as_str())
        .collect();
    assert_eq!(config_ids, ["2", "3"]);
    let plate = model.get_child("plate").unwrap();
    assert_eq!(named_children(plate, "model_instance").len(), 2);
    let assemble = model.get_child("assemble").unwrap();
    assert!(
        named_children(assemble, "assemble_item")
            .iter()
            .all(|node| ["2", "3"].contains(&node.attributes["object_id"].as_str()))
    );
}

#[test]
fn u1_benchmark_copy_only_blanks_start_gcode() {
    let source = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/packed_snap.3mf"
    ));
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("u1_cli.3mf");
    subset_3mf(source, &output, 19, true).unwrap();

    let source_config = read_zip_entry(source, "Metadata/project_settings.config");
    let output_config = read_zip_entry(&output, "Metadata/project_settings.config");
    let mut expected: serde_json::Value = serde_json::from_slice(&source_config).unwrap();
    expected["machine_start_gcode"] = serde_json::Value::String(String::new());
    let actual: serde_json::Value = serde_json::from_slice(&output_config).unwrap();
    assert_eq!(actual, expected);
}

fn read_xml<R: std::io::Read + std::io::Seek>(zip: &mut ZipArchive<R>, name: &str) -> Element {
    let mut data = Vec::new();
    zip.by_name(name).unwrap().read_to_end(&mut data).unwrap();
    Element::parse(data.as_slice()).unwrap()
}

fn named_children<'a>(element: &'a Element, name: &str) -> Vec<&'a Element> {
    element
        .children
        .iter()
        .filter_map(|node| node.as_element())
        .filter(|node| node.name == name)
        .collect()
}

fn read_zip_entry(path: &std::path::Path, name: &str) -> Vec<u8> {
    let mut zip = ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut data = Vec::new();
    zip.by_name(name).unwrap().read_to_end(&mut data).unwrap();
    data
}

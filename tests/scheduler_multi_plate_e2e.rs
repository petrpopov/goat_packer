use goat_packer::scheduler::{PlateSpec, build_schedule_plates_3mf};

/// End-to-end multi-plate archive creation, with local fixtures and no external slicer.
#[test]
fn build_multi_plate_file_and_count_plates() {
    let source = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/packed.3mf"
    ));
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("three_plates.3mf");
    let plates = [
        PlateSpec {
            label: "20 pcs".to_owned(),
            object_count: 20,
        },
        PlateSpec {
            label: "15 pcs".to_owned(),
            object_count: 15,
        },
        PlateSpec {
            label: "8 pcs".to_owned(),
            object_count: 8,
        },
    ];
    build_schedule_plates_3mf(source, &output, &plates, 256.0, 256.0, false).unwrap();
    assert_eq!(goat_packer::scheduler::plate_count_3mf(&output).unwrap(), 3);
}

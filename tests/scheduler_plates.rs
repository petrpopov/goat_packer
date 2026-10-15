use goat_packer::scheduler::plate_count_3mf;
use std::path::Path;

#[test]
fn single_plate_source_reports_one_plate() {
    let count = plate_count_3mf(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/bambu.3mf"
    )))
    .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn multi_plate_source_reports_every_plate() {
    // Synthetic source has nine instances split across three plates.
    let count = plate_count_3mf(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/three_plates.3mf"
    )))
    .unwrap();
    assert_eq!(count, 3);
}

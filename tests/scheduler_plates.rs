use goat_packer::scheduler::plate_count_3mf;
use std::path::Path;

#[test]
fn single_plate_source_reports_one_plate() {
    let count = plate_count_3mf(Path::new("../Goat 4/p1s/goat4_single_pla.3mf")).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn multi_plate_source_reports_every_plate() {
    // goat4_multi_pla.3mf packs 41 objects across three plates (18 + 7 + 16), not one dense
    // plate; daily_scheduler must reject it rather than silently undercount batches whose later
    // objects belong to plate 2 or 3.
    let count = plate_count_3mf(Path::new("../Goat 4/p1s/goat4_multi_pla.3mf")).unwrap();
    assert_eq!(count, 3);
}

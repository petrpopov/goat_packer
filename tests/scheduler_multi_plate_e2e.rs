use goat_packer::scheduler::{PlateSpec, build_schedule_plates_3mf};

/// Opt-in e2e: builds a multi-plate 3MF from a real packed source, then the operator slices
/// every plate with the real BambuStudio CLI (`--slice N`); verified 2026-09-02 (Goat 3 P1S,
/// plates 20/15/8 — all Success, exact object counts in the G-code headers). Runs only when MULTIPLATE_E2E_SOURCE points at a packed single-plate 3MF;
/// otherwise it is a no-op so the suite stays green without external assets.
#[test]
fn build_multi_plate_file_for_manual_slicing() {
    let Ok(source) = std::env::var("MULTIPLATE_E2E_SOURCE") else {
        return;
    };
    let output = std::env::var("MULTIPLATE_E2E_OUTPUT").unwrap_or_else(|_| {
        "/tmp/multiplate_e2e/goat3_plates.3mf".to_owned()
    });
    let plates = [
        PlateSpec { label: "20 pcs".to_owned(), object_count: 20 },
        PlateSpec { label: "15 pcs".to_owned(), object_count: 15 },
        PlateSpec { label: "8 pcs".to_owned(), object_count: 8 },
    ];
    build_schedule_plates_3mf(
        std::path::Path::new(&source),
        std::path::Path::new(&output),
        &plates,
        256.0,
        256.0,
        false,
    )
    .unwrap();
    println!("wrote {output}");
}

use goat_packer::{Rect, colored_packer::{PrimeTowerBounds, tower_repack_required}};

#[test]
fn requests_repack_when_final_tower_outgrows_reserved_area() {
    let used = Rect::new(10.0, 10.0, 30.0, 30.0);
    let stable = PrimeTowerBounds {
        centerline: Rect::new(12.0, 12.0, 28.0, 28.0),
        keepout: Rect::new(11.0, 11.0, 29.0, 29.0),
        extrusion_width_mm: 0.5,
    };
    let grown = PrimeTowerBounds {
        centerline: Rect::new(12.0, 12.0, 32.0, 28.0),
        keepout: Rect::new(11.0, 11.0, 33.0, 29.0),
        extrusion_width_mm: 0.5,
    };
    assert!(!tower_repack_required(used, stable));
    assert!(tower_repack_required(used, grown));
}

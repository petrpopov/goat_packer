use goat_packer::{Rect, axis_aligned_line_keepout};

#[test]
fn insets_template_printable_area_by_required_edge_margin() {
    let printable = Rect::new(0.5, 1.0, 270.5, 271.0);
    let inset = printable.inset(3.5).unwrap();
    assert_eq!(inset.min_x, 4.0);
    assert_eq!(inset.min_y, 4.5);
    assert_eq!(inset.max_x, 267.0);
    assert_eq!(inset.max_y, 267.5);
    assert!(printable.inset(136.0).is_err());
}

#[test]
fn converts_axis_aligned_purge_segments_to_nozzle_width_keepouts() {
    let horizontal = axis_aligned_line_keepout(85.0, 1.0, 185.0, 1.0, 0.5).unwrap();
    assert_eq!(horizontal.min_x, 84.75);
    assert_eq!(horizontal.min_y, 0.75);
    assert_eq!(horizontal.max_x, 185.25);
    assert_eq!(horizontal.max_y, 1.25);

    let vertical = axis_aligned_line_keepout(240.0, 1.0, 240.0, 11.0, 0.5).unwrap();
    assert_eq!(vertical.min_x, 239.75);
    assert_eq!(vertical.min_y, 0.75);
    assert_eq!(vertical.max_x, 240.25);
    assert_eq!(vertical.max_y, 11.25);

    assert!(axis_aligned_line_keepout(0.0, 0.0, 1.0, 1.0, 0.5).is_err());
}

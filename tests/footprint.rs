use goat_packer::{Point, extract_footprint, polygon_area};
use std::{fs::File, io::Write};

fn write_binary_stl(path: &std::path::Path, triangles: &[[[f32; 3]; 3]]) {
    let mut f = File::create(path).unwrap();
    f.write_all(&[0u8; 80]).unwrap();
    f.write_all(&(triangles.len() as u32).to_le_bytes())
        .unwrap();
    for tri in triangles {
        f.write_all(&[0u8; 12]).unwrap();
        for vertex in tri {
            for value in vertex {
                f.write_all(&value.to_le_bytes()).unwrap();
            }
        }
        f.write_all(&[0u8; 2]).unwrap();
    }
}

#[test]
fn extracts_and_centers_flat_bottom_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("box.stl");
    let tris = [
        [[10., 20., 0.], [14., 20., 0.], [14., 22., 0.]],
        [[10., 20., 0.], [14., 22., 0.], [10., 22., 0.]],
        [[10., 20., 2.], [14., 20., 2.], [14., 22., 2.]],
    ];
    write_binary_stl(&path, &tris);

    let fp = extract_footprint(&path).unwrap();

    assert_eq!(fp.triangle_count, 3);
    assert_eq!(fp.loops.len(), 1);
    assert!((fp.width - 4.0).abs() < 1e-6);
    assert!((fp.height - 2.0).abs() < 1e-6);
    assert!(fp.loops[0].contains(&Point { x: -2.0, y: -1.0 }));
    assert!(fp.loops[0].contains(&Point { x: 2.0, y: 1.0 }));
}

#[test]
fn goat4_real_footprint_has_expected_dimensions_and_closed_loops() {
    let path = std::path::Path::new("../Goat 4/source/goat4_v2.stl");
    let fp = extract_footprint(path).unwrap();
    assert_eq!(fp.triangle_count, 15_624);
    assert!((fp.width - 57.950_335).abs() < 0.001);
    assert!((fp.height - 81.379_105).abs() < 0.001);
    assert_eq!(fp.loops.len(), 2, "outer contour plus keyring hole");
    assert!(fp.loops.iter().all(|loop_| loop_.len() >= 4));
    let even_odd_area = polygon_area(&fp.loops[0]).abs()
        - fp.loops[1..]
            .iter()
            .map(|points| polygon_area(points).abs())
            .sum::<f64>();
    assert!((even_odd_area - 1_683.274_375).abs() < 0.01);
}

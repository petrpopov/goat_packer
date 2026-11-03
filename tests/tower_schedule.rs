//! The tool schedule of the prime-tower model (work package I3, spec 04 §4.1-§4.2): change
//! counts per build ordering, the one-layer dilation and the tower top, the mesh pass over
//! paint, part filaments and layer ranges, and the filament sources of a 3MF.

use std::collections::BTreeSet;
use std::io::Write;

use goat_packer::embedded_mesh::{ObjectMesh, Part};
use goat_packer::first_layer::{ProjectSettings, SlicerBuild};
use goat_packer::tower_model::{
    FilamentSources, LayerGrid, LayerOrdering, LayerRange, ToolSchedule, changes_per_layer,
    read_filament_sources, tool_schedule,
};
use serde_json::json;

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

fn sets(layers: &[&[u32]]) -> Vec<BTreeSet<u32>> {
    layers
        .iter()
        .map(|set| set.iter().copied().collect())
        .collect()
}

/// The layer grid of the owner's U1 project (0.25 mm first layer, 0.28 mm layers).
fn u1_layers() -> ProjectSettings {
    ProjectSettings::from_json(&json!({
        "initial_layer_print_height": "0.25",
        "layer_height": "0.28",
    }))
}

// ---------------------------------------------------------------------------------------
// Tool changes.

#[test]
fn changes_follow_the_build_ordering() {
    let raw = sets(&[&[1], &[1, 3], &[1, 3], &[3], &[3]]);
    // Orca family: the loaded filament first whenever the layer below's filaments are all on it.
    assert_eq!(
        changes_per_layer(&raw, LayerOrdering::LoadedFirst),
        vec![0, 1, 1, 1, 0]
    );
    // BambuStudio may pay the change into the next single filament one layer early (the
    // corpus A2L t-bar printed two changes at z 15.04).
    assert_eq!(
        changes_per_layer(&raw, LayerOrdering::WholeObject),
        vec![0, 1, 2, 1, 0]
    );
    // The first layer primes its first filament, so it may end on any.
    assert_eq!(
        changes_per_layer(&sets(&[&[1, 2], &[2]]), LayerOrdering::WholeObject),
        vec![1, 1]
    );
    assert_eq!(
        changes_per_layer(&sets(&[&[1], &[], &[1]]), LayerOrdering::LoadedFirst),
        vec![0, 0, 0]
    );
}

#[test]
fn schedule_dilates_by_one_layer_and_ends_at_the_last_change() {
    let grid = LayerGrid::for_object(0.2, 0.28, 2.0);
    assert_eq!(grid.count, 8);
    assert!(
        near(grid.plane(0), 0.1, 1e-12) && near(grid.top(7), 2.16, 1e-9) && grid.bottom(7) < 2.0
    );
    let raw = sets(&[&[1], &[1], &[1], &[1, 3], &[1], &[1], &[1], &[1]]);
    let schedule = ToolSchedule::from_sets(&grid, &raw, LayerOrdering::LoadedFirst);
    let filaments: Vec<Vec<u32>> = schedule
        .layers
        .iter()
        .map(|layer| layer.filaments.clone())
        .collect();
    assert_eq!(filaments[1], vec![1]);
    assert_eq!(filaments[2..=4], [vec![1, 3], vec![1, 3], vec![1, 3]]);
    assert_eq!(schedule.max_changes(), 1);
    // Layer 5 changes back to 1: the tower runs to its top.
    assert_eq!(schedule.tower_layers().len(), 6);
    assert!(near(schedule.tower_top().unwrap(), grid.top(5), 1e-12));
    assert_eq!(schedule.used, vec![1, 3]);
    let single = ToolSchedule::from_sets(
        &grid,
        &vec![BTreeSet::from([2]); 8],
        LayerOrdering::WholeObject,
    );
    assert_eq!(single.tower_top(), None);
}

// ---------------------------------------------------------------------------------------
// Filament sources and the mesh pass.

/// A 10 mm cube on filament 1 with one side triangle painted with filament 2 over its whole
/// height, and a layer range switching unpainted material to filament 3 between 5 and 7 mm.
#[test]
fn paint_parts_and_layer_ranges_make_the_layer_sets() {
    let mut cube = Part {
        vertices: vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 10.0],
            [10.0, 0.0, 10.0],
            [10.0, 10.0, 10.0],
            [0.0, 10.0, 10.0],
        ],
        triangles: vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ],
        ..Part::default()
    };
    // "8": an unsplit leaf with state 2, i.e. filament 2.
    cube.paint_color.insert(4, "8".to_owned());
    let mesh = ObjectMesh::from_plate_parts(vec![cube]).unwrap();
    let sources = FilamentSources {
        object_filament: 1,
        layer_ranges: vec![LayerRange {
            min_z: 5.0,
            max_z: 7.0,
            filament: Some(3),
            layer_height: Some(0.28),
        }],
        ..FilamentSources::default()
    };
    let settings = u1_layers();
    let schedule = tool_schedule(&mesh, &sources, &settings, SlicerBuild::SnapmakerOrca).unwrap();
    let grid = LayerGrid::for_object(0.25, 0.28, 10.0);
    let set_at = |z: f64| {
        let layer = (0..grid.count)
            .find(|&l| grid.bottom(l) < z && z <= grid.top(l))
            .unwrap();
        schedule.layers[layer].filaments.clone()
    };
    assert_eq!(set_at(2.0), vec![1, 2]);
    assert_eq!(set_at(6.0), vec![2, 3]);
    assert_eq!(set_at(4.9), vec![1, 2, 3], "dilated across the range edge");
    assert_eq!(schedule.used, vec![1, 2, 3]);
    // A range with its own layer height makes the grid variable: refused.
    let variable = FilamentSources {
        layer_ranges: vec![LayerRange {
            min_z: 5.0,
            max_z: 7.0,
            filament: None,
            layer_height: Some(0.12),
        }],
        ..sources.clone()
    };
    assert!(tool_schedule(&mesh, &variable, &settings, SlicerBuild::SnapmakerOrca).is_err());
    let profile = FilamentSources {
        variable_layer_height: true,
        ..sources
    };
    assert_eq!(
        tool_schedule(&mesh, &profile, &settings, SlicerBuild::SnapmakerOrca)
            .unwrap_err()
            .setting,
        "layer_heights_profile"
    );
}

#[test]
fn filament_sources_are_read_from_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sources.3mf");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    let entries = [
        (
            "3D/3dmodel.model",
            r#"<?xml version="1.0"?><model><resources/><build><item objectid="9" transform="1 0 0 0 1 0 0 0 1 0 0 0"/></build></model>"#,
        ),
        (
            "Metadata/model_settings.config",
            r#"<?xml version="1.0"?><config>
<object id="4"><metadata key="extruder" value="4"/></object>
<object id="9"><metadata key="extruder" value="2"/>
<part id="1" subtype="normal_part"><metadata key="extruder" value="0"/></part>
<part id="2" subtype="normal_part"><metadata key="extruder" value="3"/></part>
</object></config>"#,
        ),
        (
            "Metadata/layer_config_ranges.xml",
            r#"<?xml version="1.0"?><objects>
<object id="1"><range min_z="1" max_z="2"><option opt_key="extruder">4</option></range></object>
<object id="2"><range min_z="14.9" max_z="18.24"><option opt_key="extruder">3</option><option opt_key="layer_height">0.28</option></range></object>
</objects>"#,
        ),
    ];
    for (name, text) in entries {
        zip.start_file(name, options).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let sources = read_filament_sources(&path).unwrap();
    assert_eq!(sources.object_filament, 2);
    assert_eq!(sources.part_filaments.get(&2), Some(&3));
    assert_eq!(sources.part_filaments.get(&1), None, "0 means the object's");
    assert_eq!(
        sources.layer_ranges,
        vec![LayerRange {
            min_z: 14.9,
            max_z: 18.24,
            filament: Some(3),
            layer_height: Some(0.28)
        }]
    );
    assert_eq!(sources.range_filament_at(15.0), Some(3));
    assert_eq!(sources.range_filament_at(18.24), None);
    assert!(!sources.variable_layer_height);
}

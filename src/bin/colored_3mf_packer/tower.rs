//! The prime tower the packer reserves: the union of what the probes' slices printed and what
//! the target GUI's tower model predicts (design §6). Where the slicer that ran differs from
//! the GUI the plate is opened in (U1: vanilla Orca prints 9.5 mm of depth, the Snapmaker GUI
//! 16.1), the larger one counts.

use goat_packer::{
    Point, Rect,
    gcode_collisions::{TowerLayer, TowerLines},
    object_print::layer_bands,
    pack::TowerSpec,
    plate_centre::gui_tower_anchor_range,
    project_settings_edit::edit_project_settings_in_3mf,
    tower_model::{
        TowerPlate, TowerPrint, ToolSchedule, force_tower_shape, printed_tower,
        read_filament_sources, tool_schedule,
    },
};

use crate::AppResult;
use crate::job::Job;
use crate::probe::ClassProbe;

/// A chamfer ring narrower than this is no ring: the column's own line-width noise, mm.
const CHAMFER_MIN_MM: f64 = 0.05;
/// z that identifies one tower layer when two sources are merged, as an integer of this step.
const LAYER_KEY_STEP_MM: f64 = 1e-3;

/// A tower relative to its anchor: the column, the first-layer brim and the chamfer rings
/// (layers above the first that are wider than the column).
#[derive(Clone, Debug, PartialEq)]
struct Shape {
    body: Rect,
    brim_mm: f64,
    rings: Vec<Ring>,
    top_z: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Ring {
    z_bottom: f64,
    z_top: f64,
    width_mm: f64,
}

/// Writes the forced tower shape (`--tower-width`) into a copy of the project: the output 3MF
/// and every probe copy, so each slice prints the tower the plan reserved. The second
/// documented exception to "project_settings.config is copied byte-for-byte", after the
/// tower position.
pub fn apply_tower_shape(job: &Job, project: &std::path::Path) -> AppResult<()> {
    let Some(width) = job.args.tower_width else {
        return Ok(());
    };
    edit_project_settings_in_3mf(project, |settings| {
        force_tower_shape(settings, job.build, width);
        Ok(())
    })?;
    Ok(())
}

/// The tower to reserve, `None` when the slicer printed none.
pub fn tower_spec(job: &Job, probes: &[ClassProbe]) -> AppResult<Option<TowerSpec>> {
    let layers = union_layers(probes);
    let model = model_tower(job);
    if layers.is_empty() {
        // Without probes (`--support-source model`) the tower model alone decides; with
        // probes, a slice that printed no tower means there is none.
        let Some(print) = model.as_ref().filter(|_| probes.is_empty()) else {
            println!("no prime/wipe tower in the probe slices; packing without a tower");
            return Ok(None);
        };
        return model_only_spec(job, print);
    }
    let anchor = job
        .tower_anchor
        .unwrap_or((layers[0].extent.min_x, layers[0].extent.min_y));
    let measured = measured_shape(&layers, anchor);
    let shape = match &model {
        Some(print) => merged(&measured, &model_shape(print)),
        None => measured.clone(),
    };
    let anchor_range = anchor_range(job, model.as_ref())?;
    println!(
        "prime tower: column {:.1}x{:.1} mm, brim {:.1} mm, {} chamfer rings, top z {:.1} (probes: \
         {:.1}x{:.1}); anchor range x [{:.1}, {:.1}], y [{:.1}, {:.1}]",
        shape.body.max_x - shape.body.min_x,
        shape.body.max_y - shape.body.min_y,
        shape.brim_mm,
        shape.rings.len(),
        shape.top_z,
        measured.body.max_x - measured.body.min_x,
        measured.body.max_y - measured.body.min_y,
        anchor_range.min_x,
        anchor_range.max_x,
        anchor_range.min_y,
        anchor_range.max_y,
    );
    Ok(Some(TowerSpec {
        body: shape.body,
        brim_l0_mm: shape.brim_mm,
        chamfer: chamfer_entries(&shape.rings),
        // The GUI the plate is opened in decides whether a support over the brim warns.
        brim_conflicts: job.flavour.flavour.conflict_tower_lines() != TowerLines::None,
        top_z_mm: shape.top_z,
        anchor_range,
        source_anchor: Point {
            x: anchor.0,
            y: anchor.1,
        },
    }))
}

/// The tower the target GUI's model predicts, no probe involved.
fn model_only_spec(job: &Job, print: &TowerPrint) -> AppResult<Option<TowerSpec>> {
    let shape = model_shape(print);
    let anchor_range = anchor_range(job, Some(print))?;
    let anchor = job
        .tower_anchor
        .unwrap_or((anchor_range.min_x, anchor_range.min_y));
    println!(
        "prime tower (model): column {:.1}x{:.1} mm, brim {:.1} mm, {} chamfer rings, top z {:.1}; \
         anchor range x [{:.1}, {:.1}], y [{:.1}, {:.1}]",
        shape.body.max_x - shape.body.min_x,
        shape.body.max_y - shape.body.min_y,
        shape.brim_mm,
        shape.rings.len(),
        shape.top_z,
        anchor_range.min_x,
        anchor_range.max_x,
        anchor_range.min_y,
        anchor_range.max_y,
    );
    Ok(Some(TowerSpec {
        body: shape.body,
        brim_l0_mm: shape.brim_mm,
        chamfer: chamfer_entries(&shape.rings),
        brim_conflicts: job.flavour.flavour.conflict_tower_lines() != TowerLines::None,
        top_z_mm: shape.top_z,
        anchor_range,
        source_anchor: Point {
            x: anchor.0,
            y: anchor.1,
        },
    }))
}

/// `spec` widened to cover a tower another slice printed at `anchor`: the larger column,
/// brim and rings, the same merge as the probes and the model.
pub fn widened(spec: &TowerSpec, layers: &[TowerLayer], anchor: Point) -> TowerSpec {
    let printed = measured_shape(layers, (anchor.x, anchor.y));
    let shape = merged(&spec_shape(spec), &printed);
    TowerSpec {
        body: shape.body,
        brim_l0_mm: shape.brim_mm,
        chamfer: chamfer_entries(&shape.rings),
        top_z_mm: shape.top_z,
        ..spec.clone()
    }
}

/// The shape a spec describes (a ring entry is a point in z, which `chamfer_entries` keeps).
fn spec_shape(spec: &TowerSpec) -> Shape {
    Shape {
        body: spec.body,
        brim_mm: spec.brim_l0_mm,
        rings: spec
            .chamfer
            .iter()
            .map(|&[z, width_mm]| Ring {
                z_bottom: z,
                z_top: z,
                width_mm,
            })
            .collect(),
        top_z: spec.top_z_mm,
    }
}

/// Per layer index, the union of every probe's layer.
fn union_layers(probes: &[ClassProbe]) -> Vec<TowerLayer> {
    let mut layers: Vec<TowerLayer> = Vec::new();
    for layer in probes.iter().flat_map(|probe| &probe.tower) {
        match layers.iter_mut().find(|known| known.layer == layer.layer) {
            Some(known) => known.extent = known.extent.union(layer.extent),
            None => layers.push(*layer),
        }
    }
    layers.sort_by_key(|layer| layer.layer);
    layers
}

/// The column is the union of the upper half of the layers (as `gui_checks` reads it: the
/// brim and the chamfer only ever make the lower layers wider); the first layer's excess is
/// the brim, a later layer's the ring on it.
fn measured_shape(layers: &[TowerLayer], anchor: (f64, f64)) -> Shape {
    let relative = |layer: &TowerLayer| layer.extent.translated(-anchor.0, -anchor.1);
    let half = layers.len() / 2;
    let body = layers[half..]
        .iter()
        .map(relative)
        .reduce(Rect::union)
        .expect("a tower has layers");
    let rings = layers[1..half.max(1)]
        .iter()
        .filter_map(|layer| {
            let width_mm = excess(relative(layer), body);
            (width_mm > CHAMFER_MIN_MM).then_some(Ring {
                z_bottom: layer.z_bottom,
                z_top: layer.z,
                width_mm,
            })
        })
        .collect();
    Shape {
        body,
        brim_mm: excess(relative(&layers[0]), body),
        rings,
        top_z: layers[layers.len() - 1].z,
    }
}

/// How far `outer` reaches past `inner` on its worst side, never negative.
fn excess(outer: Rect, inner: Rect) -> f64 {
    [
        inner.min_x - outer.min_x,
        inner.min_y - outer.min_y,
        outer.max_x - inner.max_x,
        outer.max_y - inner.max_y,
    ]
    .into_iter()
    .fold(0.0, f64::max)
}

/// The GUI's tower for the project, `None` when the model refuses it (a note says why).
fn model_tower(job: &Job) -> Option<TowerPrint> {
    let build = job.build;
    let sources = read_filament_sources(&job.args.source_3mf)
        .map_err(|error| error.to_string())
        .and_then(|sources| {
            tool_schedule(&job.mesh, &sources, &job.settings, build).map_err(|e| e.to_string())
        })
        .and_then(|schedule: ToolSchedule| {
            let plate = TowerPlate {
                printable: job.profile.printable_bed(),
                object_height: job.mesh.height(),
            };
            printed_tower(&job.settings, &schedule, build, &plate).map_err(|e| e.to_string())
        });
    match sources {
        Ok(print) => print,
        Err(reason) => {
            eprintln!("note: tower model skipped, the probes' tower stands alone: {reason}");
            None
        }
    }
}

fn model_shape(print: &TowerPrint) -> Shape {
    let zones = print.body_zones((0.0, 0.0));
    let body = Rect::of_points(zones[0].loops.iter().flatten());
    let rings = print
        .chamfer
        .iter()
        .map(|ring| Ring {
            z_bottom: ring.z_bottom,
            z_top: ring.z_top,
            width_mm: ring.reach_mm,
        })
        .collect();
    Shape {
        body,
        brim_mm: print.l0_brim_mm,
        rings,
        top_z: print.height,
    }
}

/// Everything either shape covers: the union of the columns, the larger brim, each layer's
/// wider ring and the higher top.
fn merged(a: &Shape, b: &Shape) -> Shape {
    let mut rings = a.rings.clone();
    for ring in &b.rings {
        let key = |ring: &Ring| (ring.z_top / LAYER_KEY_STEP_MM).round() as i64;
        match rings.iter_mut().find(|known| key(known) == key(ring)) {
            Some(known) => known.width_mm = known.width_mm.max(ring.width_mm),
            None => rings.push(*ring),
        }
    }
    rings.sort_by(|x, y| x.z_top.total_cmp(&y.z_top));
    Shape {
        body: a.body.union(b.body),
        brim_mm: a.brim_mm.max(b.brim_mm),
        rings,
        top_z: a.top_z.max(b.top_z),
    }
}

/// `TowerSpec` has one entry per `[z, width]` and tests a ring at that z only, so a layer
/// that straddles a 2 mm band edge (1.88-2.16) becomes one entry in each band it meets.
fn chamfer_entries(rings: &[Ring]) -> Vec<[f64; 2]> {
    let mut entries = Vec::new();
    for ring in rings {
        for band in layer_bands(ring.z_bottom, ring.z_top) {
            let (from, to) = goat_packer::object_print::band_z_range(band);
            let overlap = (ring.z_bottom.max(from), ring.z_top.min(to));
            entries.push([(overlap.0 + overlap.1) * 0.5, ring.width_mm]);
        }
    }
    entries
}

/// Where the target GUI keeps a stored tower: the model's range, or (a model that refused
/// the project) the GUI estimate alone.
fn anchor_range(job: &Job, model: Option<&TowerPrint>) -> AppResult<Rect> {
    let range = match model {
        Some(print) => print.anchor_range,
        None => {
            let settings = serde_json::from_slice(&job.settings_bytes)?;
            gui_tower_anchor_range(
                &settings,
                &job.profile.slicer_kind,
                job.profile.printable_bed(),
                job.mesh.height(),
            )
        }
    };
    range.ok_or_else(|| {
        format!(
            "no tower position is inside the range the {} GUI keeps; the plate is too small \
             for the prime tower",
            job.flavour.flavour.name()
        )
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(index: usize, z_bottom: f64, z: f64, extent: Rect) -> TowerLayer {
        TowerLayer {
            layer: index,
            z_bottom,
            z,
            extent,
        }
    }

    /// A 10x8 column at (100, 50) with a 3 mm brim on layer 1 and a 1 mm ring on layer 2.
    fn goat_like() -> Vec<TowerLayer> {
        let column = Rect::new(100.0, 50.0, 110.0, 58.0);
        let mut layers = vec![
            layer(1, 0.0, 0.2, column.grown(3.0)),
            layer(2, 0.2, 0.48, column.grown(1.0)),
        ];
        layers.extend((3..=10).map(|i| layer(i, 0.2 + 0.28 * (i - 2) as f64, 0.2 + 0.28 * (i - 1) as f64, column)));
        layers
    }

    #[test]
    fn the_column_is_the_upper_half_and_the_lower_layers_are_brim_and_rings() {
        let shape = measured_shape(&goat_like(), (100.0, 50.0));
        assert_eq!(shape.body, Rect::new(0.0, 0.0, 10.0, 8.0));
        assert!((shape.brim_mm - 3.0).abs() < 1e-9);
        assert_eq!(shape.rings.len(), 1);
        assert!((shape.rings[0].width_mm - 1.0).abs() < 1e-9);
        assert!((shape.top_z - 2.72).abs() < 1e-9);
    }

    #[test]
    fn the_merge_keeps_the_wider_column_ring_and_brim() {
        let measured = measured_shape(&goat_like(), (100.0, 50.0));
        let model = Shape {
            body: Rect::new(0.0, 0.0, 10.0, 16.0),
            brim_mm: 2.0,
            rings: vec![
                Ring { z_bottom: 0.2, z_top: 0.48, width_mm: 1.5 },
                Ring { z_bottom: 0.48, z_top: 0.76, width_mm: 0.5 },
            ],
            top_z: 2.0,
        };
        let both = merged(&measured, &model);
        assert_eq!(both.body, Rect::new(0.0, 0.0, 10.0, 16.0));
        assert!((both.brim_mm - 3.0).abs() < 1e-9);
        let widths: Vec<f64> = both.rings.iter().map(|ring| ring.width_mm).collect();
        assert_eq!(widths, vec![1.5, 0.5]);
        assert!((both.top_z - 2.72).abs() < 1e-9);
    }

    #[test]
    fn a_ring_over_a_band_edge_is_an_entry_in_each_band_it_meets() {
        let ring = Ring { z_bottom: 1.88, z_top: 2.16, width_mm: 1.2 };
        let entries = chamfer_entries(&[ring]);
        assert_eq!(entries.len(), 2);
        assert!(entries[0][0] > 1.88 && entries[0][0] < 2.0);
        assert!(entries[1][0] > 2.0 && entries[1][0] < 2.16);
        assert!(entries.iter().all(|entry| entry[1] == 1.2));
    }
}

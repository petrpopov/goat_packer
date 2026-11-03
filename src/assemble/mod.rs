//! 3MF plate assembly: clones contour-packed placements into a slicer 3MF template. Ports
//! `build_3mf.py`; contracts in
//! `docs/superpowers/specs/2026-09-01-eliminate-python-rust-port-design.md` (Task 4).
//!
//! The XML parts are rewritten with a quick-xml event round-trip: parse into owned events,
//! touch only the spans that must change, and write everything else back verbatim. quick-xml
//! keeps raw bytes, so untouched regions round-trip with their attribute order, quoting and
//! namespace prefixes intact — the reason it was chosen over xmltree, which structurally
//! cannot represent prefixed attributes (`p:UUID`, `p:path`).
//!
//! Modules: `archive` (zip entries), `xml` (event helpers), `core_model` / `mesh_parts` (the
//! model and its meshes), `metadata` (settings, cut information, plate JSON), `bed` and
//! `preview` (thumbnails), `format` (numbers and ids).

mod archive;
mod bed;
mod core_model;
mod format;
mod frame;
mod mesh_parts;
mod metadata;
mod preview;
#[cfg(test)]
mod tests;
mod xml;

use std::path::{Path, PathBuf};

use crate::{Footprint, PackedPlacement, Rect, embedded_mesh, extract_footprint};

use archive::{
    CORE_MODEL_ENTRY, CUT_INFORMATION_ENTRY, Entries, LAYER_HEIGHTS_ENTRY, MODEL_SETTINGS_ENTRY,
    PLATE_JSON_ENTRY, PROJECT_SETTINGS_ENTRY, entry, entry_bytes, read_template_entries, set_entry,
    write_archive,
};
use bed::parse_bed_geometry;
use core_model::{CoreRebuild, source_z_offset};
use mesh_parts::{duplicate_colored_mesh_parts, load_inline_mesh_template};
use metadata::{
    rebuild_cut_information, rebuild_layer_heights_profile, rebuild_model_settings,
    rebuild_plate_json,
};
use preview::{placed_polygons, write_previews};

pub use core_model::{source_item_transform_of, source_model_center};
pub use frame::ItemFrame;

/// Everything the assembler needs besides the template itself.
#[derive(Clone, Debug)]
pub struct AssembleOptions {
    pub placements: Vec<PackedPlacement>,
    /// The collision STL: the source 3MF's embedded mesh with the component transform baked
    /// in (see `embedded_mesh`). The footprint and the compensation center are both derived
    /// from this exact file — a differently-exported STL would shift every object.
    pub stl_path: PathBuf,
    pub edge_margin: f64,
    /// `(x0, y0, x1, y1, width)` purge-line segments, drawn on the previews.
    pub purge_lines: Vec<[f64; 5]>,
    /// Keepout rectangles (prime tower, purge-line keepouts), drawn on the previews.
    pub keepouts: Vec<Rect>,
    /// Mesh strategies (mutually exclusive, spec contract 5). The packer binary uses the
    /// default mode only; the others exist for tests and manual recovery paths.
    pub duplicate_mesh_per_object: bool,
    pub reuse_source_object_instances: bool,
    pub inline_colored_mesh_objects: bool,
    /// The footprint the placements were packed against, when it differs from the mesh
    /// projection (the probe slice's first layer — body plus supports plus brim). The preview
    /// polygons must show what was packed, or the plate thumbnails lie about collisions.
    /// `None` derives the projection from `stl_path` as before.
    pub footprint_override: Option<Footprint>,
    /// Builds every clone from the source item's own transform (`ItemFrame::clone_transform`;
    /// each placement's `angle_deg` is its rotation class). `None` = the legacy transform of
    /// rotation and translation compensated by the STL's bbox centre.
    pub frame: Option<ItemFrame>,
    /// What the previews and the plate JSON show per placement, in bed coordinates; `None`
    /// = `footprint_override` (or the STL's projection) rotated and moved per placement.
    pub world_polygons: Option<Vec<Vec<Vec<crate::Point>>>>,
}

/// Clones `options.placements` into `template` and writes the packed project to `output`.
pub fn assemble_project(
    template: &Path,
    output: &Path,
    options: &AssembleOptions,
) -> Result<(), String> {
    if options.placements.is_empty() {
        return Err("placement list is empty".to_owned());
    }
    let mut entries = read_template_entries(template)?;

    // Contract 1: project_settings.config is read for the bed geometry only and copied
    // through byte-for-byte — it is never re-serialized by the assembler.
    let settings = entry_bytes(&entries, PROJECT_SETTINGS_ENTRY)?;
    let (printable, exclusions) = parse_bed_geometry(settings)?;

    let core = entry_bytes(&entries, CORE_MODEL_ENTRY)?.to_vec();
    let (world_polygons, footprint_center) = match &options.world_polygons {
        Some(polygons) => (polygons.clone(), (0.0, 0.0)),
        None => {
            let footprint = footprint_of(options)?;
            let center = embedded_mesh::stl_xy_center(&options.stl_path)
                .map_err(|error| format!("cannot read {:?}: {error}", options.stl_path))?;
            (placed_polygons(&footprint.loops, &options.placements), center)
        }
    };
    // Contract 3: the source item's z translation becomes the clones' item z offset, keeping
    // the packed objects at the same on-bed height the source project sliced at.
    let z_offset = source_z_offset(&core)?;

    rewrite_core_model(&mut entries, &core, options, z_offset, footprint_center)?;
    rewrite_metadata(&mut entries, options, z_offset, &world_polygons)?;
    write_previews(
        &mut entries,
        options,
        &printable,
        &exclusions,
        &world_polygons,
    );
    write_archive(output, &entries)
}

/// Contract 3: the collision STL is the embedded mesh with the component's transform baked
/// in, so its XY bbox center is exactly the point every build-item translation must
/// compensate (`placement − R(angle)·c_stl`). When the caller packed against the probe
/// slice's first layer (supports and brim included), that patch replaces the projection —
/// and its loops are already centered on the same bbox center, so the placement math is
/// unchanged.
fn footprint_of(options: &AssembleOptions) -> Result<Footprint, String> {
    match &options.footprint_override {
        Some(footprint) => Ok(footprint.clone()),
        None => extract_footprint(&options.stl_path).map_err(|error| {
            format!(
                "cannot extract the footprint from {:?}: {error}",
                options.stl_path
            )
        }),
    }
}

/// The core model with one object (and mesh part, per the mesh strategy) per placement.
fn rewrite_core_model(
    entries: &mut Entries,
    core: &[u8],
    options: &AssembleOptions,
    z_offset: f64,
    footprint_center: (f64, f64),
) -> Result<(), String> {
    let component_paths = if options.duplicate_mesh_per_object {
        Some(duplicate_colored_mesh_parts(
            entries,
            core,
            options.placements.len(),
        )?)
    } else {
        None
    };
    let inline_mesh_template = if options.inline_colored_mesh_objects {
        Some(load_inline_mesh_template(entries, core)?)
    } else {
        None
    };
    let rebuilt = CoreRebuild {
        placements: &options.placements,
        component_paths: component_paths.as_deref(),
        inline_mesh_template: inline_mesh_template.as_deref(),
        reuse_source_object_instances: options.reuse_source_object_instances,
        z_offset,
        footprint_center,
        frame: options.frame,
    }
    .apply(core)?;
    set_entry(entries, CORE_MODEL_ENTRY, rebuilt);
    Ok(())
}

/// Settings, layer heights, cut information and plate JSON for the clones (skipped in reuse
/// mode, where the source metadata stays valid as it is).
fn rewrite_metadata(
    entries: &mut Entries,
    options: &AssembleOptions,
    z_offset: f64,
    world_polygons: &[Vec<Vec<crate::Point>>],
) -> Result<(), String> {
    if options.reuse_source_object_instances {
        return Ok(());
    }
    let count = options.placements.len();
    let model_settings = entry_bytes(entries, MODEL_SETTINGS_ENTRY)?;
    let rebuilt = rebuild_model_settings(model_settings, &options.placements, z_offset)?;
    set_entry(entries, MODEL_SETTINGS_ENTRY, rebuilt);
    // OrcaSlicer refuses to slice a prime tower "unless all objects have the same variable
    // layer height": the copied entry still points only at the source's ordinal, so every
    // clone slices without the source's layer-height table. Re-point the table at every
    // clone's ordinal so all objects carry identical variable heights again.
    if let Some(profile) = entry(entries, LAYER_HEIGHTS_ENTRY) {
        let rebuilt = rebuild_layer_heights_profile(profile, count);
        set_entry(entries, LAYER_HEIGHTS_ENTRY, rebuilt);
    }
    // Contract 6: cut_information.xml and plate_1.json are rewritten only when the template
    // already carries them — Snapmaker projects lack them and Bambu-only metadata must never
    // be invented.
    if let Some(cut) = entry(entries, CUT_INFORMATION_ENTRY) {
        let rebuilt = rebuild_cut_information(cut, count)?;
        set_entry(entries, CUT_INFORMATION_ENTRY, rebuilt);
    }
    if let Some(plate) = entry(entries, PLATE_JSON_ENTRY) {
        let rebuilt = rebuild_plate_json(plate, world_polygons)?;
        set_entry(entries, PLATE_JSON_ENTRY, rebuilt);
    }
    Ok(())
}

//! The probes of one rotation class kept between runs. The key hashes everything a probe
//! depends on, so a hit is the very data the slicer would give again (up to its own run-to-run
//! variation, which is why the probe is a union of several runs in the first place).

use std::{fs, path::{Path, PathBuf}, time::UNIX_EPOCH};

use goat_packer::{Point, embedded_mesh::ObjectMesh};
use sha2::{Digest, Sha256};

use crate::AppResult;
use crate::job::Job;
use crate::probe::ClassProbe;

/// Bumped whenever what a probe stores, or how it is read from G-code, changes: old entries
/// then miss instead of feeding stale geometry to the packer.
pub const MODEL_VERSION: u32 = 1;

/// SHA-256 hex of everything the probe of `rotation_deg` at `centre` depends on: the mesh of
/// every part with its paint, the item transform, the whole project settings, the slicer
/// binary, the rotation and where the probe object stands.
pub fn cache_key(job: &Job, rotation_deg: f64, centre: Point) -> String {
    let mut hash = Sha256::new();
    hash.update(MODEL_VERSION.to_le_bytes());
    hash_mesh(&mut hash, &job.mesh);
    for value in job.frame.source.iter().chain(&[job.frame.origin.0, job.frame.origin.1]) {
        hash.update(value.to_le_bytes());
    }
    hash_bytes(&mut hash, &job.settings_bytes);
    // The probe copy is edited when the gap is forced; hashed only then, so a plate that
    // slices with its project's own value keeps the keys (and cache entries) it always had.
    if job.own_gap.edits_project() {
        hash_bytes(&mut hash, b"own_gap_first_layer");
        hash.update(job.own_gap.first_layer_mm.to_le_bytes());
    }
    hash_bytes(&mut hash, job.profile.slicer_path.as_bytes());
    hash_bytes(&mut hash, slicer_fingerprint(&job.profile.slicer_path).as_bytes());
    hash.update([u8::from(job.profile.blank_start_gcode_for_cli)]);
    for value in [rotation_deg, centre.x, centre.y] {
        hash.update(value.to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}

/// Length-prefixed, so two byte strings never run into each other.
fn hash_bytes(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn hash_mesh(hash: &mut Sha256, mesh: &ObjectMesh) {
    for part in &mesh.parts {
        hash_bytes(hash, format!("{:?}", part.kind).as_bytes());
        hash.update(part.source_id.to_le_bytes());
        hash.update([u8::from(part.mirrored)]);
        for value in part.vertices.iter().flatten() {
            hash.update(value.to_le_bytes());
        }
        for index in part.triangles.iter().flatten() {
            hash.update(index.to_le_bytes());
        }
        for (triangle, code) in part.paint_color.iter().chain(&part.paint_supports) {
            hash.update(triangle.to_le_bytes());
            hash_bytes(hash, code.as_bytes());
        }
    }
}

/// Size and modification time of the slicer binary: its version without running it (a hit
/// must make no slicer call). A missing file hashes as empty and fails at slicing time.
fn slicer_fingerprint(path: &str) -> String {
    let Ok(meta) = fs::metadata(path) else {
        return String::new();
    };
    let seconds = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |age| age.as_secs());
    format!("{}:{seconds}", meta.len())
}

fn entry_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("probe-{key}.json"))
}

/// The cached probe under `key`; a missing or unreadable entry is a miss.
pub fn load(dir: &Path, key: &str) -> Option<ClassProbe> {
    serde_json::from_slice(&fs::read(entry_path(dir, key)).ok()?).ok()
}

/// Stores `probe` under `key` (written beside the entry first, so a reader never sees half).
pub fn store(dir: &Path, key: &str, probe: &ClassProbe) -> AppResult<()> {
    fs::create_dir_all(dir)?;
    let path = entry_path(dir, key);
    let partial = path.with_extension("json.part");
    fs::write(&partial, serde_json::to_vec(probe)?)?;
    fs::rename(partial, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_probe_comes_back_and_another_key_misses() {
        let dir = tempfile::tempdir().unwrap();
        let probe = ClassProbe { runs: 3, ..ClassProbe::default() };
        store(dir.path(), "abc", &probe).unwrap();
        assert_eq!(load(dir.path(), "abc").unwrap().runs, 3);
        assert!(load(dir.path(), "abd").is_none());
        assert!(load(&dir.path().join("missing"), "abc").is_none());
    }
}

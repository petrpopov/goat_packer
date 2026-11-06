//! The model's results kept between runs (design §5.6). The key hashes everything a run
//! depends on: the mesh of every part with its paint, every support-relevant setting, the
//! flavour, the rotation and `MODEL_VERSION`. What is cached is the raw ensemble (the distinct
//! realizations), not the grown envelope, so a changed margin needs no recomputation.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ensemble::MemberSummary;
use super::flavour::Flavour;
use super::merge::{MergeOrder, Overlap};
use super::settings::SupportSettings;
use crate::embedded_mesh::ObjectMesh;
use crate::object_print::Realization;

/// Bumped on any change of what the generator computes: old entries then miss.
pub const MODEL_VERSION: u32 = 9;

/// The members of one (object, settings, flavour, rotation) and the union of their
/// realizations (`ensemble::run_union`: one realization since `MODEL_VERSION` 9).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Ensemble {
    pub members: Vec<MemberSummary>,
    pub realizations: Vec<Realization>,
}

impl Ensemble {
    /// How many distinct graphs the members drew.
    pub fn distinct_graphs(&self) -> usize {
        let hashes: std::collections::BTreeSet<&str> =
            self.members.iter().map(|member| member.hash.as_str()).collect();
        hashes.len()
    }
}

/// SHA-256 hex of everything the ensemble depends on.
pub fn cache_key(
    mesh: &ObjectMesh,
    settings: &SupportSettings,
    flavour: Flavour,
    rotation_deg: f64,
    orders: &[MergeOrder],
    overlaps: &[Overlap],
) -> String {
    let mut hash = Sha256::new();
    hash.update(MODEL_VERSION.to_le_bytes());
    hash_bytes(&mut hash, flavour.name().as_bytes());
    hash.update(rotation_deg.to_le_bytes());
    for order in orders {
        hash_bytes(&mut hash, format!("{order:?}").as_bytes());
    }
    for overlap in overlaps {
        hash_bytes(&mut hash, format!("{overlap:?}").as_bytes());
    }
    hash_bytes(&mut hash, &serde_json::to_vec(settings).unwrap_or_default());
    for part in &mesh.parts {
        hash_bytes(&mut hash, format!("{:?}", part.kind).as_bytes());
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
            hash_bytes(&mut hash, code.as_bytes());
        }
    }
    format!("{:x}", hash.finalize())
}

/// Length-prefixed, so two byte strings never run into each other.
fn hash_bytes(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn entry_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("support-{key}.json"))
}

/// The cached ensemble under `key`; a missing or unreadable entry is a miss.
pub fn load(dir: &Path, key: &str) -> Option<Ensemble> {
    serde_json::from_slice(&fs::read(entry_path(dir, key)).ok()?).ok()
}

/// Stores `ensemble` under `key` (written beside the entry first, so a reader never sees
/// half of it).
pub fn store(dir: &Path, key: &str, ensemble: &Ensemble) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = entry_path(dir, key);
    let partial = path.with_extension("json.part");
    fs::write(&partial, serde_json::to_vec(ensemble)?)?;
    fs::rename(partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_ensemble_comes_back_and_another_key_misses() {
        let dir = tempfile::tempdir().unwrap();
        let ensemble = Ensemble {
            members: Vec::new(),
            realizations: vec![Realization {
                member: 7,
                ..Realization::default()
            }],
        };
        store(dir.path(), "abc", &ensemble).unwrap();
        assert_eq!(load(dir.path(), "abc").unwrap(), ensemble);
        assert!(load(dir.path(), "abd").is_none());
        assert!(load(&dir.path().join("missing"), "abc").is_none());
    }
}

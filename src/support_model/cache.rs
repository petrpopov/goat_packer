//! The model's results kept between runs (design §5.6). The key hashes everything a run
//! depends on: the mesh of every part with its paint, every support-relevant setting, the
//! flavour, the rotation and `MODEL_VERSION`. The raw ensemble (its drawn union) is cached
//! apart from the grown envelope, so a changed margin needs no recomputation; the envelope has
//! its own entry (`envelope_key`: the ensembles' keys and the margins, `ENVELOPE_VERSION`),
//! since growing it was most of a job on an ensemble hit.

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
/// 9: the distinct graphs are drawn as one union (`bands::union_realization`).
pub const MODEL_VERSION: u32 = 9;

/// The ensemble of one (object, settings, flavour, rotation).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Ensemble {
    pub members: Vec<MemberSummary>,
    /// How many distinct branch graphs the members produced.
    #[serde(default)]
    pub graphs: usize,
    /// The realizations the envelope unites: the distinct graphs drawn together, one.
    pub realizations: Vec<Realization>,
}

/// Bumped on any change of how `envelope` grows an ensemble: old envelope entries then miss.
pub const ENVELOPE_VERSION: u32 = 1;

/// One ensemble of a class's envelope with the margins it is grown by.
pub struct EnvelopePart<'a> {
    /// The ensemble's `cache_key`.
    pub ensemble_key: &'a str,
    pub first_layer_mm: f64,
    /// The margin of every band the ensemble has, from band 0.
    pub band_mm: Vec<f64>,
}

/// SHA-256 hex of a class's grown envelope: the ensembles it unites (their keys hash
/// `MODEL_VERSION` and every input) and the margins each is grown by.
pub fn envelope_key(parts: &[EnvelopePart]) -> String {
    let mut hash = Sha256::new();
    hash.update(ENVELOPE_VERSION.to_le_bytes());
    for part in parts {
        hash_bytes(&mut hash, part.ensemble_key.as_bytes());
        hash.update(part.first_layer_mm.to_le_bytes());
        hash.update((part.band_mm.len() as u64).to_le_bytes());
        for margin in &part.band_mm {
            hash.update(margin.to_le_bytes());
        }
    }
    format!("{:x}", hash.finalize())
}

/// A grown envelope as stored: its own key inside, so a renamed file is a miss.
#[derive(Deserialize, Serialize)]
struct EnvelopeEntry {
    key: String,
    envelope: Realization,
}

fn envelope_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("envelope-{key}.json"))
}

/// The cached envelope under `key`; a missing, unreadable or mislabelled entry is a miss.
pub fn load_envelope(dir: &Path, key: &str) -> Option<Realization> {
    let entry: EnvelopeEntry = serde_json::from_slice(&fs::read(envelope_path(dir, key)).ok()?).ok()?;
    (entry.key == key).then_some(entry.envelope)
}

/// Stores the envelope grown under `key` (written beside the entry first, like `store`).
pub fn store_envelope(dir: &Path, key: &str, envelope: &Realization) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = envelope_path(dir, key);
    let partial = path.with_extension("json.part");
    let entry = EnvelopeEntry {
        key: key.to_owned(),
        envelope: envelope.clone(),
    };
    fs::write(&partial, serde_json::to_vec(&entry)?)?;
    fs::rename(partial, path)
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
            graphs: 1,
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

    #[test]
    fn an_envelope_comes_back_under_its_key_and_any_margin_change_misses() {
        let dir = tempfile::tempdir().unwrap();
        let part = |first_layer_mm: f64, band_mm: Vec<f64>| EnvelopePart {
            ensemble_key: "ensemble",
            first_layer_mm,
            band_mm,
        };
        let key = envelope_key(&[part(0.5, vec![1.0, 2.0])]);
        let envelope = Realization {
            member: 3,
            ..Realization::default()
        };
        store_envelope(dir.path(), &key, &envelope).unwrap();
        assert_eq!(load_envelope(dir.path(), &key).unwrap(), envelope);
        for other in [
            envelope_key(&[part(0.6, vec![1.0, 2.0])]),
            envelope_key(&[part(0.5, vec![1.0, 2.5])]),
            envelope_key(&[part(0.5, vec![1.0])]),
            envelope_key(&[part(0.5, vec![1.0, 2.0]), part(0.5, vec![1.0, 2.0])]),
        ] {
            assert_ne!(other, key);
            assert!(load_envelope(dir.path(), &other).is_none());
        }
        // A file renamed to another key is not taken for it.
        let renamed = envelope_key(&[part(0.7, vec![])]);
        fs::copy(envelope_path(dir.path(), &key), envelope_path(dir.path(), &renamed)).unwrap();
        assert!(load_envelope(dir.path(), &renamed).is_none());
    }
}

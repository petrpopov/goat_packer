//! Packing results kept between runs (`pack-<key>.json` in `--cache-dir`). Packing is
//! deterministic, so the outcome of the same prints, rules, tower and options is the one a
//! new run would find; on a warm cache it was nearly the whole job (U1 23.6 s of 24.8 s at two
//! threads). A cached outcome still passes `PackPlan::validate` before any plate is written.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use goat_packer::{
    PackOptions,
    pack::{DEFAULT_MEMBERS, LevelReport, PACK_VERSION, PackOutcome},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::plan::PackPlan;

/// Set once any pack of this process ran instead of coming from the cache.
static PACKED: AtomicBool = AtomicBool::new(false);

/// True when every pack of this run so far came from the cache (and at least the report asks
/// after one pack).
pub fn all_cached() -> bool {
    !PACKED.load(Ordering::Relaxed)
}

/// An outcome as stored: its own key inside, so a renamed file is a miss. The ladder report is
/// kept beside the outcome because the outcome never serializes it (it carries wall times).
#[derive(Deserialize, Serialize)]
struct PackEntry {
    key: String,
    outcome: PackOutcome,
    levels: Vec<LevelReport>,
}

/// The outcome of `pack` for `plan` under `options` (`resolutions` = the levels it runs):
/// from `dir` when an entry is there, else packed and stored. A failed pack is not stored.
pub fn cached(
    dir: Option<&Path>,
    plan: &PackPlan,
    options: &PackOptions,
    pack: impl FnOnce() -> Result<PackOutcome, String>,
) -> Result<PackOutcome, String> {
    let Some(dir) = dir else {
        PACKED.store(true, Ordering::Relaxed);
        return pack();
    };
    let key = cache_key(plan, options);
    if let Some(outcome) = load(dir, &key) {
        return Ok(outcome);
    }
    PACKED.store(true, Ordering::Relaxed);
    let outcome = pack()?;
    if let Err(error) = store(dir, &key, &outcome) {
        // The cache only saves time: a full disk must not fail the job.
        eprintln!("warning: packing result not cached: {error}");
    }
    Ok(outcome)
}

/// SHA-256 hex of everything a pack depends on: the algorithm's version, the members, the
/// plan (prints, plate rules, tower, foreign gaps) and every packing option.
pub fn cache_key(plan: &PackPlan, options: &PackOptions) -> String {
    let mut hash = Sha256::new();
    hash.update(PACK_VERSION.to_le_bytes());
    hash_bytes(&mut hash, format!("{DEFAULT_MEMBERS:?}").as_bytes());
    hash_bytes(&mut hash, &serde_json::to_vec(plan).unwrap_or_default());
    hash_bytes(&mut hash, &serde_json::to_vec(options).unwrap_or_default());
    format!("{:x}", hash.finalize())
}

/// Length-prefixed, so two byte strings never run into each other.
fn hash_bytes(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn entry_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("pack-{key}.json"))
}

/// The cached outcome under `key`; a missing, unreadable or mislabelled entry is a miss.
fn load(dir: &Path, key: &str) -> Option<PackOutcome> {
    let entry: PackEntry = serde_json::from_slice(&fs::read(entry_path(dir, key)).ok()?).ok()?;
    (entry.key == key).then(|| PackOutcome {
        levels: entry.levels,
        ..entry.outcome
    })
}

/// Stores `outcome` under `key` (written beside the entry first, so a reader never sees half).
fn store(dir: &Path, key: &str, outcome: &PackOutcome) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = entry_path(dir, key);
    let partial = path.with_extension("json.part");
    let entry = PackEntry {
        key: key.to_owned(),
        outcome: outcome.clone(),
        levels: outcome.levels.clone(),
    };
    fs::write(&partial, serde_json::to_vec(&entry)?)?;
    fs::rename(partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack_steps::ladder;
    use crate::test_support::{test_job, test_plan};

    fn refuse() -> Result<PackOutcome, String> {
        Err("packed instead of read from the cache".to_owned())
    }

    #[test]
    fn a_second_run_reads_the_same_layout_and_packs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut job = test_job(dir.path());
        job.args.cache_dir = Some(dir.path().join("cache"));
        let plan = test_plan(&job, 2);
        let first = plan.pack(&job.args).unwrap();
        let options = PackOptions {
            resolutions: ladder(&job.args),
            ..plan.options(&job.args)
        };
        let second = cached(job.args.cache_dir.as_deref(), &plan, &options, refuse).unwrap();
        assert!(!first.placements.is_empty());
        assert_eq!(serde_json::to_value(&first).unwrap(), serde_json::to_value(&second).unwrap());
        assert_eq!(first.levels, second.levels);
    }

    #[test]
    fn another_plan_or_option_misses() {
        let dir = tempfile::tempdir().unwrap();
        let mut job = test_job(dir.path());
        job.args.cache_dir = Some(dir.path().join("cache"));
        let plan = test_plan(&job, 2);
        plan.pack(&job.args).unwrap();
        let options = PackOptions {
            resolutions: ladder(&job.args),
            ..plan.options(&job.args)
        };
        let cache = job.args.cache_dir.as_deref();
        assert!(cached(cache, &plan, &options, refuse).is_ok());
        let wider = PackOptions {
            clearance: options.clearance + 0.1,
            ..options.clone()
        };
        assert!(cached(cache, &plan, &wider, refuse).is_err());
        assert!(cached(cache, &test_plan(&job, 1), &options, refuse).is_err());
    }

    #[test]
    fn a_renamed_entry_is_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        let job = test_job(dir.path());
        let outcome = test_plan(&job, 1).pack(&job.args).unwrap();
        store(dir.path(), "aaaa", &outcome).unwrap();
        fs::rename(entry_path(dir.path(), "aaaa"), entry_path(dir.path(), "bbbb")).unwrap();
        assert!(load(dir.path(), "bbbb").is_none());
    }
}

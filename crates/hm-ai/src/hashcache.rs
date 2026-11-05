//! Remembers SHA-256 results between scans, keyed by path, size and modification time, so a file
//! that hasn't changed is never hashed twice. This is the one thing a scan writes, under
//! `~/Library/Caches/HealthyMac`, which rule 2 allows. Losing or corrupting it only costs time, and
//! nothing dangerous can follow from a wrong entry: the cleaner hashes both files itself before it
//! replaces anything.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::files::{is_sha256, read_capped, FileRec};

const CACHE_CAP: u64 = 64 * 1024 * 1024;
/// Scans can run side by side in one process, as with the UI's inventory and a cleaning scan.
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    size: u64,
    mtime_ns: i128,
    sha256: String,
}

#[derive(Debug, Default)]
pub(crate) struct HashCache {
    path: PathBuf,
    entries: HashMap<String, Entry>,
    touched: HashSet<String>,
    dirty: bool,
}

pub(crate) fn cache_path(home: &Path) -> PathBuf {
    home.join("Library/Caches/HealthyMac/model-hashes.json")
}

impl HashCache {
    pub fn load(home: &Path) -> Self {
        let path = cache_path(home);
        let entries = read_capped(&path, CACHE_CAP)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            entries,
            ..Self::default()
        }
    }

    pub fn get(&mut self, record: &FileRec) -> Option<String> {
        let key = record.path.to_string_lossy().into_owned();
        let entry = self.entries.get(&key)?;
        let valid = entry.size == record.len
            && entry.mtime_ns == record.mtime_ns
            && is_sha256(&entry.sha256);
        if valid {
            let sha256 = entry.sha256.clone();
            self.touched.insert(key);
            Some(sha256)
        } else {
            None
        }
    }

    pub fn insert(&mut self, record: &FileRec, sha256: String) {
        let key = record.path.to_string_lossy().into_owned();
        self.touched.insert(key.clone());
        self.dirty = true;
        self.entries.insert(
            key,
            Entry {
                size: record.len,
                mtime_ns: record.mtime_ns,
                sha256,
            },
        );
    }

    /// Writes atomically if anything was added. Entries not used by a complete scan are dropped, so
    /// it doesn't grow forever. A cancelled scan didn't get to use them all, so it keeps them.
    pub fn save(&mut self, complete: bool) {
        if !self.dirty {
            return;
        }
        if complete {
            let touched = std::mem::take(&mut self.touched);
            self.entries.retain(|key, _| touched.contains(key));
            self.touched = touched;
        }
        let Some(parent) = self.path.parent() else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec(&self.entries) else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let unique = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".model-hashes-{}-{unique}.tmp", std::process::id()));
        // Caching is best effort. A failure here only means hashing again next time.
        if fs::write(&temporary, bytes).is_err() || fs::rename(&temporary, &self.path).is_err() {
            let _ = fs::remove_file(&temporary);
            return;
        }
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;

    const DIGEST: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn round_trips_and_invalidates_on_any_change() {
        let fx = Fixture::new();
        let path = fx.write("m.gguf", b"pesos");
        let record = FileRec::stat(&path, "m.gguf").unwrap();
        let mut cache = HashCache::load(&fx.home);
        assert_eq!(cache.get(&record), None);
        cache.insert(&record, DIGEST.to_string());
        cache.save(true);

        let mut again = HashCache::load(&fx.home);
        assert_eq!(again.get(&record).as_deref(), Some(DIGEST));

        fx.set_age(&path, 3);
        let touched = FileRec::stat(&path, "m.gguf").unwrap();
        assert_eq!(again.get(&touched), None, "a different mtime");

        fs::write(&path, b"otros pesos mas largos").unwrap();
        fx.set_age(&path, 3);
        let resized = FileRec::stat(&path, "m.gguf").unwrap();
        assert_eq!(again.get(&resized), None, "a different size");
    }

    #[test]
    fn garbage_on_disk_is_an_empty_cache() {
        let fx = Fixture::new();
        let path = fx.write("m.gguf", b"pesos");
        let record = FileRec::stat(&path, "m.gguf").unwrap();
        fx.write("Library/Caches/HealthyMac/model-hashes.json", b"{nope");
        assert_eq!(HashCache::load(&fx.home).get(&record), None);
    }

    #[test]
    fn a_non_digest_in_the_cache_is_not_believed() {
        let fx = Fixture::new();
        let path = fx.write("m.gguf", b"pesos");
        let record = FileRec::stat(&path, "m.gguf").unwrap();
        let mut cache = HashCache::load(&fx.home);
        cache.insert(&record, "../../evil".to_string());
        cache.save(true);
        assert_eq!(HashCache::load(&fx.home).get(&record), None);
    }

    #[test]
    fn nothing_new_writes_nothing_and_complete_scans_prune() {
        let fx = Fixture::new();
        let keep = FileRec::stat(&fx.write("a", b"a"), "a").unwrap();
        let gone = FileRec::stat(&fx.write("b", b"b"), "b").unwrap();

        let mut cache = HashCache::load(&fx.home);
        cache.save(true);
        assert!(!cache_path(&fx.home).exists());

        cache.insert(&keep, DIGEST.to_string());
        cache.insert(&gone, DIGEST.to_string());
        cache.save(true);

        let mut second = HashCache::load(&fx.home);
        assert!(second.get(&keep).is_some());
        let extra = FileRec::stat(&fx.write("c", b"c"), "c").unwrap();
        second.insert(&extra, DIGEST.to_string());
        second.save(true);

        let mut third = HashCache::load(&fx.home);
        assert!(third.get(&keep).is_some());
        assert!(third.get(&gone).is_none());
    }
}

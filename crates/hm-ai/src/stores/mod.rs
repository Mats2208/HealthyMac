//! One module per on-disk layout. Each turns a folder into models and leftovers. None of them
//! changes anything.

pub(crate) mod folders;
pub(crate) mod huggingface;
pub(crate) mod ollama;

use std::path::PathBuf;
use std::time::SystemTime;

use crate::files::{unique_bytes, FileRec};
use crate::metadata::Detected;
use crate::types::RuntimeKind;

/// Nothing younger than this is called abandoned or orphaned. Ollama writes a model's manifest only
/// after all its blobs, and its own pruning leaves young blobs alone for the same reason: until the
/// manifest appears, a finished blob of a pull in progress looks exactly like an orphan.
pub(crate) const ABANDONED_MIN_DAYS: u64 = 7;

/// A model as found on disk, before what is known across all of them gets filled in.
#[derive(Clone, Debug)]
pub(crate) struct RawModel {
    pub name: String,
    pub runtime: RuntimeKind,
    pub detected: Detected,
    pub root: PathBuf,
    pub files: Vec<FileRec>,
    /// When the runtime last recorded using it. See the `usage` module.
    pub used: Option<SystemTime>,
    pub incomplete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeftoverKind {
    /// A download that stopped and wasn't resumed.
    Partial,
    /// An Ollama blob no manifest uses.
    OllamaOrphan,
    /// A Hugging Face blob no snapshot uses.
    HfUnreferenced,
    /// An old revision of a Hugging Face repository that no ref points at.
    HfOldRevision,
    /// A payload in Hugging Face's shared store no repository links to.
    HfStoreOrphan,
}

/// Something on disk that isn't a usable model.
#[derive(Clone, Debug)]
pub(crate) struct Leftover {
    pub kind: LeftoverKind,
    pub runtime: RuntimeKind,
    /// Stable part of the finding id.
    pub key: String,
    pub label: String,
    /// What to remove.
    pub paths: Vec<PathBuf>,
    /// The real files that removing `paths` frees.
    pub records: Vec<FileRec>,
    pub newest: Option<SystemTime>,
}

impl Leftover {
    pub fn bytes(&self) -> u64 {
        unique_bytes(&self.records)
    }
}

#[derive(Debug, Default)]
pub(crate) struct StoreScan {
    pub models: Vec<RawModel>,
    pub leftovers: Vec<Leftover>,
    /// Lock folders old enough that no download can be holding them.
    pub stale_locks: Vec<PathBuf>,
    /// `datasets--*` repositories in the Hugging Face cache.
    pub dataset_repos: Vec<PathBuf>,
}

impl StoreScan {
    pub fn absorb(&mut self, other: StoreScan) {
        self.models.extend(other.models);
        self.leftovers.extend(other.leftovers);
        self.stale_locks.extend(other.stale_locks);
        self.dataset_repos.extend(other.dataset_repos);
    }
}

/// The newest of some times.
pub(crate) fn newest<I: IntoIterator<Item = Option<SystemTime>>>(times: I) -> Option<SystemTime> {
    times.into_iter().flatten().max()
}

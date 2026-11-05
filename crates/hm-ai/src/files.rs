//! Filesystem primitives shared by the stores: what we know about a file, and walking without ever
//! following a link by accident.

use std::fs::{self, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use hm_core::ScanContext;

/// Stops a pathological tree from turning a scan into an endless walk.
const MAX_WALK_ENTRIES: usize = 500_000;

/// A regular file as found on disk.
#[derive(Clone, Debug)]
pub(crate) struct FileRec {
    /// The real file: the last component is never a symlink.
    pub path: PathBuf,
    /// The name the runtime knows it by. For a Hugging Face blob this is the name in the snapshot,
    /// as the blob's own name is a hash and says nothing about what it is.
    pub logical: String,
    pub dev: u64,
    pub ino: u64,
    /// Apparent size.
    pub len: u64,
    /// Allocated on disk.
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    pub mtime_ns: i128,
    /// SHA-256 of the content when the store's own naming already guarantees it.
    pub sha256: Option<String>,
}

impl FileRec {
    fn from_meta(path: &Path, logical: String, meta: &Metadata) -> Self {
        Self {
            path: path.to_path_buf(),
            logical,
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            bytes: meta.blocks() * 512,
            modified: meta.modified().ok(),
            mtime_ns: i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec()),
            sha256: None,
        }
    }

    /// `None` unless `path` itself is a regular file. Symlinks do not count.
    pub fn stat(path: &Path, logical: impl Into<String>) -> Option<Self> {
        let meta = fs::symlink_metadata(path).ok()?;
        meta.is_file()
            .then(|| Self::from_meta(path, logical.into(), &meta))
    }

    pub fn with_sha256(mut self, sha256: Option<String>) -> Self {
        self.sha256 = sha256;
        self
    }

    pub fn key(&self) -> (u64, u64) {
        (self.dev, self.ino)
    }
}

/// Removes `.` and `..` without touching the disk. Hugging Face's relative links only need this,
/// and it never stats anything, so it can't raise a permission prompt.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Where a symlink points, one hop, as a normalized absolute path.
pub(crate) fn link_target(link: &Path) -> Option<PathBuf> {
    let target = fs::read_link(link).ok()?;
    if target.is_absolute() {
        Some(normalize(&target))
    } else {
        Some(normalize(&link.parent()?.join(target)))
    }
}

/// `true` for exactly 64 lowercase hex digits, the form every store here writes a SHA-256 in.
pub(crate) fn is_sha256(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Walks `dir` without following symlinks. `visit` sees every entry except `dir` itself and returns
/// whether to descend into it, if it's a directory. Returns `false` if part of the tree couldn't be
/// read or the walk was cut short, in which case the caller knows less than it might think.
pub(crate) fn walk(
    dir: &Path,
    max_depth: usize,
    ctx: &ScanContext,
    visit: &mut dyn FnMut(&Path, &Metadata) -> bool,
) -> bool {
    let mut complete = true;
    let mut seen = 0usize;
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((current, depth)) = stack.pop() {
        if ctx.is_cancelled() {
            return false;
        }
        let Ok(entries) = fs::read_dir(&current) else {
            complete = false;
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_WALK_ENTRIES {
                return false;
            }
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                complete = false;
                continue;
            };
            let descend = visit(&path, &meta);
            if descend && meta.is_dir() {
                if depth + 1 < max_depth {
                    stack.push((path, depth + 1));
                } else {
                    complete = false;
                }
            }
        }
    }
    complete
}

/// The file name as text, for anything that only makes sense as text.
pub(crate) fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `path` relative to `base` as text with `/`, or its name if it isn't under `base`.
pub(crate) fn relative(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .map(|rel| rel.to_string_lossy().into_owned())
        .unwrap_or_else(|_| name_of(path))
}

/// Reads a small file, refusing anything bigger than `cap`.
pub(crate) fn read_capped(path: &Path, cap: u64) -> Option<Vec<u8>> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > cap {
        return None;
    }
    fs::read(path).ok()
}

/// Whether a time is at least `days` old. A time in the future isn't old.
pub(crate) fn older_than(time: Option<SystemTime>, now: SystemTime, days: u64) -> bool {
    time.and_then(|time| hm_core::fmt::age_days(time, now))
        .is_some_and(|age| age >= days)
}

/// Unique on-disk bytes across `records`, counting hard links once.
pub(crate) fn unique_bytes<'a>(records: impl IntoIterator<Item = &'a FileRec>) -> u64 {
    let mut seen = std::collections::HashSet::new();
    records
        .into_iter()
        .filter(|record| seen.insert(record.key()))
        .map(|record| record.bytes)
        .sum()
}

/// Runs a read-only query with the SQLite that ships in macOS, so there is nothing to link. Opened
/// with `immutable=1`: nothing is written, not even a `-shm` file next to someone else's database,
/// at the cost of not seeing changes still in a WAL. For what's read here that means, at worst,
/// data a few minutes old.
pub(crate) fn sqlite_query(database: &Path, sql: &str) -> Option<String> {
    let mut encoded = String::from("file:");
    for byte in database.to_string_lossy().bytes() {
        match byte {
            b'%' | b'?' | b'#' | b' ' => encoded.push_str(&format!("%{byte:02X}")),
            other => encoded.push(char::from(other)),
        }
    }
    encoded.push_str("?mode=ro&immutable=1");
    let output = hm_core::exec::run(
        Path::new("/usr/bin/sqlite3"),
        &["-readonly", &encoded, sql],
        std::time::Duration::from_secs(3),
    )
    .ok()?;
    output.success().then_some(output.stdout)
}

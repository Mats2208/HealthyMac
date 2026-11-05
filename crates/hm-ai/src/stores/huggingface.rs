//! The Hugging Face hub cache. Two generations of layout coexist, often in one repository:
//!
//! ```text
//! hub/models--org--name/
//!     refs/main                       text file with a commit hash
//!     snapshots/<commit>/config.json  symlink to ../../blobs/<etag>
//!     blobs/<etag>                    a regular file, or...
//!     blobs/<etag> -> ../../blobs/ab/<xet hash>    ...a symlink into the shared store
//! hub/blobs/.huggingface-shared-blobs
//! hub/blobs/ab/<xet hash>             the real file, read-only
//! hub/blobs/ab/<xet hash>.refs        hint listing the symlinks
//! ```
//!
//! What the names mean, checked by hashing real files on this project's reference machine:
//!
//! - An `etag` of 64 hex digits is the SHA-256 of the content. Of 40 it's a git SHA-1 of a small
//!   file, and useless for comparing with anything else.
//! - A shared store name is a **Xet hash, not a SHA-256**, even though it looks like one. Only the
//!   etag of the symlink pointing at it says what the content's SHA-256 is.
//!
//! The shared store changes what removal means too. Trashing `models--*` alone frees nothing for
//! these blobs: the bytes stay behind in `hub/blobs`. And it's shared with `datasets--*` and
//! `spaces--*`, so all repositories must be read before saying that any payload is unused.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use hm_core::fsutil::{measure, CancelToken};
use hm_core::ScanContext;

use super::{newest, Leftover, LeftoverKind, RawModel, StoreScan, ABANDONED_MIN_DAYS};
use crate::files::{
    is_sha256, link_target, name_of, older_than, read_capped, relative, walk, FileRec,
};
use crate::locate::Roots;
use crate::metadata;
use crate::types::RuntimeKind;
use crate::usage::UsageRecords;

const STORE_MARKER: &str = ".huggingface-shared-blobs";
const STORE_LAYOUT: &[u8] = b"1\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepoKind {
    Model,
    Dataset,
    Space,
}

/// `models--org--name` is `org/name`. The organization can't contain `--`, the name can.
fn parse_repo_dir(name: &str) -> Option<(RepoKind, String)> {
    let (kind, encoded) = if let Some(rest) = name.strip_prefix("models--") {
        (RepoKind::Model, rest)
    } else if let Some(rest) = name.strip_prefix("datasets--") {
        (RepoKind::Dataset, rest)
    } else {
        (RepoKind::Space, name.strip_prefix("spaces--")?)
    };
    if encoded.is_empty() {
        return None;
    }
    Some((kind, encoded.replacen("--", "/", 1)))
}

fn store_is_valid(hub: &Path) -> bool {
    read_capped(&hub.join("blobs").join(STORE_MARKER), 16)
        .is_some_and(|bytes| bytes == STORE_LAYOUT)
}

/// `hub/blobs/<first two of hash>/<hash>`, and nothing else, is a store payload.
fn store_payload(hub: &Path, target: &Path) -> Option<PathBuf> {
    let relative = target.strip_prefix(hub.join("blobs")).ok()?;
    let mut parts = relative.iter();
    let prefix = parts.next()?.to_str()?;
    let hash = parts.next()?.to_str()?;
    (parts.next().is_none() && is_sha256(hash) && hash.starts_with(prefix) && prefix.len() == 2)
        .then(|| target.to_path_buf())
}

fn manifest_of(payload: &Path) -> PathBuf {
    payload.with_file_name(format!("{}.refs", name_of(payload)))
}

#[derive(Debug, Default)]
struct BlobEntry {
    /// The store payload, when this entry is a symlink into the store.
    store: Option<PathBuf>,
    /// The real file with the etag's digest, if it exists.
    record: Option<FileRec>,
}

#[derive(Debug)]
enum Link {
    Blob(String),
    /// A regular file in the snapshot, or a link leading out of the repository's blobs.
    File(Box<FileRec>),
    Dangling,
}

#[derive(Debug)]
struct Revision {
    commit: String,
    dir: PathBuf,
    files: Vec<(String, Link)>,
}

#[derive(Debug)]
struct Repo {
    dir: PathBuf,
    kind: RepoKind,
    id: String,
    blobs: BTreeMap<String, BlobEntry>,
    revisions: Vec<Revision>,
    active: HashSet<String>,
    /// Everything was read. If not, nothing is concluded from what wasn't seen.
    complete: bool,
    /// The last time anything happened here.
    activity: Option<SystemTime>,
    incomplete_files: Vec<FileRec>,
}

impl Repo {
    fn blobs_dir(&self) -> PathBuf {
        self.dir.join("blobs")
    }

    fn active_revisions(&self) -> impl Iterator<Item = &Revision> {
        let everything = self.active.is_empty();
        self.revisions
            .iter()
            .filter(move |revision| everything || self.active.contains(&revision.commit))
    }

    fn detached_revisions(&self) -> impl Iterator<Item = &Revision> {
        // With no ref resolving, nothing shows that any revision is out of date: someone may be
        // using it by commit hash, or the refs may be missing. Then all count as in use.
        let any_active = !self.active.is_empty();
        self.revisions
            .iter()
            .filter(move |revision| any_active && !self.active.contains(&revision.commit))
    }

    fn blobs_used_by<'a>(&self, revisions: impl Iterator<Item = &'a Revision>) -> HashSet<String> {
        revisions
            .flat_map(|revision| revision.files.iter())
            .filter_map(|(_, link)| match link {
                Link::Blob(name) => Some(name.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Reads a repository's blobs folder. Symlinks are followed only into the shared store, or checked
/// against the gate first, so a link planted in a cache can't lead the scan somewhere protected.
fn read_blobs(repo: &mut Repo, hub: &Path, roots: &Roots) {
    let blobs_dir = repo.blobs_dir();
    let entries = match fs::read_dir(&blobs_dir) {
        Ok(entries) => entries,
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                repo.complete = false;
            }
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = name_of(&path);
        if name.ends_with(".incomplete") {
            repo.incomplete_files
                .extend(FileRec::stat(&path, name.clone()));
            continue;
        }
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            repo.complete = false;
            continue;
        };
        let digest = is_sha256(&name).then(|| name.clone());
        let mut blob = BlobEntry::default();
        if meta.is_file() {
            blob.record =
                FileRec::stat(&path, name.clone()).map(|record| record.with_sha256(digest));
        } else if meta.file_type().is_symlink() {
            if let Some(target) = link_target(&path) {
                blob.store = store_payload(hub, &target);
                if roots.allowed(&target) {
                    blob.record = FileRec::stat(&target, name.clone())
                        .map(|record| record.with_sha256(digest));
                }
            }
        }
        repo.blobs.insert(name, blob);
    }
}

fn read_revision(
    repo: &Repo,
    commit: &str,
    dir: &Path,
    roots: &Roots,
    ctx: &ScanContext,
) -> Option<Revision> {
    let blobs_dir = repo.blobs_dir();
    let mut files = Vec::new();
    let complete = walk(dir, 32, ctx, &mut |path, meta| {
        if meta.is_dir() {
            return true;
        }
        let logical = relative(path, dir);
        let link = if meta.file_type().is_symlink() {
            match link_target(path) {
                Some(target) if target.parent() == Some(blobs_dir.as_path()) => {
                    Link::Blob(name_of(&target))
                }
                Some(target) if roots.allowed(&target) => {
                    match FileRec::stat(&target, logical.clone()) {
                        Some(record) => Link::File(Box::new(record)),
                        None => Link::Dangling,
                    }
                }
                _ => Link::Dangling,
            }
        } else {
            match FileRec::stat(path, logical.clone()) {
                Some(record) => Link::File(Box::new(record)),
                None => Link::Dangling,
            }
        };
        files.push((logical, link));
        false
    });
    complete.then(|| Revision {
        commit: commit.to_string(),
        dir: dir.to_path_buf(),
        files,
    })
}

fn read_repo(
    dir: PathBuf,
    kind: RepoKind,
    id: String,
    hub: &Path,
    roots: &Roots,
    ctx: &ScanContext,
) -> Repo {
    let mut repo = Repo {
        dir,
        kind,
        id,
        blobs: BTreeMap::new(),
        revisions: Vec::new(),
        active: HashSet::new(),
        complete: true,
        activity: None,
        incomplete_files: Vec::new(),
    };
    read_blobs(&mut repo, hub, roots);

    let snapshots = repo.dir.join("snapshots");
    match fs::read_dir(&snapshots) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                let commit = name_of(&path);
                if commit.starts_with('.')
                    || !fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir())
                {
                    continue;
                }
                match read_revision(&repo, &commit, &path, roots, ctx) {
                    Some(revision) => repo.revisions.push(revision),
                    None => repo.complete = false,
                }
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => repo.complete = false,
    }

    let mut times = vec![
        fs::metadata(repo.blobs_dir())
            .ok()
            .and_then(|meta| meta.modified().ok()),
        fs::metadata(&snapshots)
            .ok()
            .and_then(|meta| meta.modified().ok()),
    ];
    times.extend(repo.incomplete_files.iter().map(|file| file.modified));

    let refs = repo.dir.join("refs");
    let mut pointed_at = Vec::new();
    if refs.is_dir() {
        let refs_read = walk(&refs, 8, ctx, &mut |path, meta| {
            if meta.is_file() {
                times.push(meta.modified().ok());
                if let Some(bytes) = read_capped(path, 256) {
                    pointed_at.push(String::from_utf8_lossy(&bytes).trim().to_string());
                }
            }
            true
        });
        repo.complete &= refs_read;
    }
    let known: HashSet<String> = repo
        .revisions
        .iter()
        .map(|revision| revision.commit.clone())
        .collect();
    repo.active = pointed_at
        .into_iter()
        .filter(|commit| known.contains(commit))
        .collect();
    repo.activity = newest(times);
    repo
}

/// A repository's model: what its active revisions need, each real file once however many names it
/// goes by.
fn build_model(repo: &Repo, runtime: RuntimeKind, usage: &UsageRecords) -> Option<RawModel> {
    let mut files: Vec<FileRec> = Vec::new();
    let mut seen = HashSet::new();
    let mut incomplete = false;
    for revision in repo.active_revisions() {
        for (logical, link) in &revision.files {
            let record = match link {
                Link::File(record) => Some(record.as_ref().clone()),
                Link::Blob(name) => repo.blobs.get(name).and_then(|blob| blob.record.clone()),
                Link::Dangling => None,
            };
            let Some(mut record) = record else {
                incomplete = true;
                continue;
            };
            if seen.insert(record.path.clone()) {
                record.logical = logical.clone();
                files.push(record);
            }
        }
    }
    if files.is_empty() {
        return None;
    }
    // oMLX serves this cache, under an id made of the repository's name.
    let omlx_id = repo.id.replacen('/', "--", 1);
    Some(RawModel {
        name: repo.id.clone(),
        runtime,
        detected: metadata::detect(&repo.id, &files, false),
        root: repo.dir.clone(),
        used: usage.omlx.get(&omlx_id).copied(),
        incomplete: incomplete || metadata::missing_parts(&files),
        files,
    })
}

/// Who links to each store payload, across every kind of repository.
fn store_links(repos: &[Repo]) -> HashMap<PathBuf, usize> {
    let mut links: HashMap<PathBuf, usize> = HashMap::new();
    for repo in repos {
        for blob in repo.blobs.values() {
            if let Some(store) = &blob.store {
                *links.entry(store.clone()).or_default() += 1;
            }
        }
    }
    links
}

/// What must go to remove these blobs from `repo`: the entry in the repository and, if this was the
/// last link to it, the payload in the store. A blob another repository links to keeps its bytes.
fn blob_removal(
    repo: &Repo,
    names: impl IntoIterator<Item = String>,
    links: &HashMap<PathBuf, usize>,
    can_collect_store: bool,
) -> (Vec<PathBuf>, Vec<FileRec>) {
    let mut paths = Vec::new();
    let mut records = Vec::new();
    let mut going: HashMap<PathBuf, usize> = HashMap::new();
    for name in names {
        let Some(blob) = repo.blobs.get(&name) else {
            continue;
        };
        paths.push(repo.blobs_dir().join(&name));
        match &blob.store {
            None => records.extend(blob.record.clone()),
            Some(store) => {
                let count = going.entry(store.clone()).or_default();
                *count += 1;
                if can_collect_store && links.get(store) == Some(&*count) {
                    paths.push(store.clone());
                    paths.push(manifest_of(store));
                    records.extend(blob.record.clone());
                }
            }
        }
    }
    (paths, records)
}

pub(crate) fn scan_hub(
    hub: &Path,
    runtime: RuntimeKind,
    roots: &Roots,
    ctx: &ScanContext,
    usage: &UsageRecords,
) -> StoreScan {
    let mut out = StoreScan::default();
    let Ok(entries) = fs::read_dir(hub) else {
        return out;
    };
    let mut repos = Vec::new();
    let mut all_read = true;
    for entry in entries.flatten() {
        if ctx.is_cancelled() {
            return out;
        }
        let path = entry.path();
        let name = name_of(&path);
        let Some((kind, id)) = parse_repo_dir(&name) else {
            continue;
        };
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => {}
            // A repository somewhere else could hold links we can't account for.
            _ => {
                all_read = false;
                continue;
            }
        }
        let repo = read_repo(path, kind, id, hub, roots, ctx);
        all_read &= repo.complete;
        repos.push(repo);
    }
    repos.sort_by(|a, b| a.dir.cmp(&b.dir));

    let links = store_links(&repos);
    let store_ok = store_is_valid(hub);
    // A payload can only be called unused if every repository was seen.
    let can_collect_store = store_ok && all_read;

    for repo in &repos {
        match repo.kind {
            RepoKind::Model => out.models.extend(build_model(repo, runtime, usage)),
            RepoKind::Dataset => out.dataset_repos.push(repo.dir.clone()),
            RepoKind::Space => {}
        }
        collect_repo_leftovers(repo, runtime, &links, can_collect_store, ctx, &mut out);
    }

    if can_collect_store {
        collect_store_orphans(hub, runtime, &links, ctx, &mut out);
    }
    collect_stale_locks(hub, ctx, &mut out);
    out
}

fn collect_repo_leftovers(
    repo: &Repo,
    runtime: RuntimeKind,
    links: &HashMap<PathBuf, usize>,
    can_collect_store: bool,
    ctx: &ScanContext,
    out: &mut StoreScan,
) {
    if !repo.incomplete_files.is_empty() {
        let newest_write = newest(repo.incomplete_files.iter().map(|file| file.modified));
        if older_than(newest_write, ctx.now, ABANDONED_MIN_DAYS) {
            out.leftovers.push(Leftover {
                kind: LeftoverKind::Partial,
                runtime,
                key: format!("{}:{}", runtime.key(), repo.id),
                label: repo.id.clone(),
                paths: repo
                    .incomplete_files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect(),
                records: repo.incomplete_files.clone(),
                newest: newest_write,
            });
        }
    }

    // A download in progress updates the ref before all of its links exist. Until the repository has
    // gone quiet, what's linked from where doesn't mean anything yet.
    if !repo.complete || !older_than(repo.activity, ctx.now, ABANDONED_MIN_DAYS) {
        return;
    }

    let used_by_any = repo.blobs_used_by(repo.revisions.iter());
    let unreferenced: Vec<String> = repo
        .blobs
        .keys()
        .filter(|name| !used_by_any.contains(*name))
        .cloned()
        .collect();
    if !unreferenced.is_empty() {
        let (paths, records) = blob_removal(repo, unreferenced, links, can_collect_store);
        out.leftovers.push(Leftover {
            kind: LeftoverKind::HfUnreferenced,
            runtime,
            key: format!("{}:{}", runtime.key(), repo.id),
            label: repo.id.clone(),
            paths,
            newest: newest(records.iter().map(|record| record.modified)),
            records,
        });
    }

    let detached: Vec<&Revision> = repo.detached_revisions().collect();
    if !detached.is_empty() {
        let used_by_active = repo.blobs_used_by(repo.active_revisions());
        let only_old: Vec<String> = repo
            .blobs_used_by(detached.iter().copied())
            .into_iter()
            .filter(|name| !used_by_active.contains(name))
            .collect();
        let (blob_paths, records) = blob_removal(repo, only_old, links, can_collect_store);
        let mut paths: Vec<PathBuf> = detached
            .iter()
            .map(|revision| revision.dir.clone())
            .collect();
        paths.extend(blob_paths);
        out.leftovers.push(Leftover {
            kind: LeftoverKind::HfOldRevision,
            runtime,
            key: format!("{}:{}", runtime.key(), repo.id),
            label: repo.id.clone(),
            paths,
            newest: newest(records.iter().map(|record| record.modified)),
            records,
        });
    }
}

/// Payloads no repository links to. As in `huggingface_hub` itself, one without its `.refs` file
/// isn't touched: missing metadata is never grounds for removing anything.
fn collect_store_orphans(
    hub: &Path,
    runtime: RuntimeKind,
    links: &HashMap<PathBuf, usize>,
    ctx: &ScanContext,
    out: &mut StoreScan,
) {
    let store = hub.join("blobs");
    let mut records = Vec::new();
    let mut paths = Vec::new();
    walk(&store, 2, ctx, &mut |path, meta| {
        if meta.is_dir() {
            return true;
        }
        if !meta.is_file() || store_payload(hub, path).is_none() || links.contains_key(path) {
            return false;
        }
        let manifest = manifest_of(path);
        if !manifest.is_file() || !older_than(meta.modified().ok(), ctx.now, ABANDONED_MIN_DAYS) {
            return false;
        }
        records.extend(FileRec::stat(path, name_of(path)));
        paths.push(path.to_path_buf());
        paths.push(manifest);
        false
    });
    if !paths.is_empty() {
        out.leftovers.push(Leftover {
            kind: LeftoverKind::HfStoreOrphan,
            runtime,
            key: format!("hf-store:{}", hub.display()),
            label: "Hugging Face".to_string(),
            paths,
            newest: newest(records.iter().map(|record| record.modified)),
            records,
        });
    }
}

/// `hub/.locks/<repo>/`. They are left behind by design, and hold nothing but a name. A folder only
/// counts if nothing in it has changed for a week.
fn collect_stale_locks(hub: &Path, ctx: &ScanContext, out: &mut StoreScan) {
    let locks = hub.join(".locks");
    let Ok(entries) = fs::read_dir(&locks) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            continue;
        }
        let mut times = vec![fs::metadata(&path)
            .ok()
            .and_then(|meta| meta.modified().ok())];
        let complete = walk(&path, 4, ctx, &mut |_, meta| {
            times.push(meta.modified().ok());
            true
        });
        if complete && older_than(newest(times), ctx.now, ABANDONED_MIN_DAYS) {
            out.stale_locks.push(path);
        }
    }
    out.stale_locks.sort();
}

/// What to trash to really remove a repository, and how many bytes that frees: the folder, plus the
/// store payloads nothing else links to. Reads the cache now rather than trusting an earlier scan,
/// since this decides what gets removed. If any other repository can't be read, no payload is
/// included: that leaves bytes behind, but never breaks another model.
pub(crate) fn repo_removal(repo_dir: &Path) -> (Vec<PathBuf>, u64) {
    let cancel = CancelToken::new();
    let mut paths = vec![repo_dir.to_path_buf()];
    let mut bytes = measure(repo_dir, &cancel).bytes;
    let Some(hub) = repo_dir.parent() else {
        return (paths, bytes);
    };
    if !store_is_valid(hub) {
        return (paths, bytes);
    }

    let links_in = |dir: &Path| -> Option<Vec<PathBuf>> {
        let blobs = dir.join("blobs");
        let entries = match fs::read_dir(&blobs) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Some(Vec::new()),
            Err(_) => return None,
        };
        Some(
            entries
                .flatten()
                .filter_map(|entry| link_target(&entry.path()))
                .filter_map(|target| store_payload(hub, &target))
                .collect(),
        )
    };

    let Some(mut exclusive) =
        links_in(repo_dir).map(|links| links.into_iter().collect::<HashSet<_>>())
    else {
        return (paths, bytes);
    };
    let Ok(entries) = fs::read_dir(hub) else {
        return (paths, bytes);
    };
    for entry in entries.flatten() {
        let other = entry.path();
        if other == repo_dir || parse_repo_dir(&name_of(&other)).is_none() {
            continue;
        }
        let readable = fs::symlink_metadata(&other).is_ok_and(|meta| meta.is_dir());
        match links_in(&other).filter(|_| readable) {
            Some(links) => {
                for link in links {
                    exclusive.remove(&link);
                }
            }
            None => return (paths, bytes),
        }
    }
    let mut exclusive: Vec<PathBuf> = exclusive.into_iter().collect();
    exclusive.sort();
    for payload in exclusive {
        if let Some(record) = FileRec::stat(&payload, name_of(&payload)) {
            bytes += record.bytes;
            paths.push(payload.clone());
            let manifest = manifest_of(&payload);
            if manifest.is_file() {
                paths.push(manifest);
            }
        }
    }
    (paths, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locate::Detail;
    use crate::test_support::{sha, Fixture};
    use crate::types::ModelFormat;

    fn scan_fixture(fx: &Fixture) -> StoreScan {
        let ctx = fx.ctx();
        let roots = Roots::resolve_with(&fx.home, false, Detail::Light, &|_| None);
        scan_hub(
            &fx.home.join(".cache/huggingface/hub"),
            RuntimeKind::HuggingFace,
            &roots,
            &ctx,
            &UsageRecords::default(),
        )
    }

    fn only_model(found: &StoreScan) -> &RawModel {
        assert_eq!(found.models.len(), 1, "{:?}", found.models);
        &found.models[0]
    }

    #[test]
    fn decodes_repo_names() {
        assert_eq!(
            parse_repo_dir("models--org--name"),
            Some((RepoKind::Model, "org/name".into()))
        );
        assert_eq!(
            parse_repo_dir("models--gpt2"),
            Some((RepoKind::Model, "gpt2".into()))
        );
        assert_eq!(
            parse_repo_dir("models--org--a--b"),
            Some((RepoKind::Model, "org/a--b".into()))
        );
        assert_eq!(
            parse_repo_dir("datasets--org--d"),
            Some((RepoKind::Dataset, "org/d".into()))
        );
        assert_eq!(parse_repo_dir("blobs"), None);
        assert_eq!(parse_repo_dir("models--"), None);
    }

    #[test]
    fn counts_blobs_not_symlinks() {
        let fx = Fixture::new();
        let repo = fx.hf_repo("mlx-community/m");
        let etag = sha(b"pesos grandes");
        repo.blob(&etag, &[9u8; 100_000]);
        repo.snapshot(
            "aaa",
            &[
                ("model.safetensors", &etag),
                ("copia/model.safetensors", &etag),
            ],
        );
        repo.snapshot("bbb", &[("model.safetensors", &etag)]);
        repo.main("aaa");
        repo.main_also("bbb");

        let found = scan_fixture(&fx);
        let model = only_model(&found);
        assert_eq!(model.name, "mlx-community/m");
        assert_eq!(model.files.len(), 1);
        assert_eq!(model.files[0].len, 100_000);
        assert_eq!(model.files[0].sha256.as_deref(), Some(etag.as_str()));
        assert_eq!(
            model.files[0].logical.rsplit('/').next(),
            Some("model.safetensors")
        );
        assert_eq!(model.detected.format, ModelFormat::Mlx);
    }

    #[test]
    fn sha1_etags_are_not_taken_for_digests() {
        let fx = Fixture::new();
        let repo = fx.hf_repo("org/m");
        let etag = "a".repeat(40);
        repo.blob(&etag, b"{}");
        repo.snapshot("aaa", &[("config.json", &etag)]);
        repo.main("aaa");
        let found = scan_fixture(&fx);
        assert_eq!(only_model(&found).files[0].sha256, None);
    }

    #[test]
    fn the_shared_store_is_followed_and_its_xet_name_is_not_a_digest() {
        let fx = Fixture::new();
        let repo = fx.hf_repo("org/m");
        let etag = sha(b"contenido real");
        let payload = repo.store_blob(&etag, &"b".repeat(64), b"contenido real");
        repo.snapshot("aaa", &[("model.safetensors", &etag)]);
        repo.main("aaa");

        let found = scan_fixture(&fx);
        let file = &only_model(&found).files[0];
        assert_eq!(file.path, payload, "the real file, not the symlink");
        assert_eq!(file.sha256.as_deref(), Some(etag.as_str()));
        assert_ne!(
            file.sha256.as_deref(),
            payload.file_name().and_then(|n| n.to_str())
        );
        assert!(found.leftovers.is_empty());
    }

    #[test]
    fn dangling_links_do_not_fail_and_mark_the_model_incomplete() {
        let fx = Fixture::new();
        let repo = fx.hf_repo("org/m");
        let etag = sha(b"esta");
        repo.blob(&etag, b"esta");
        repo.snapshot(
            "aaa",
            &[
                ("a.safetensors", &etag),
                ("b.safetensors", &sha(b"no esta")),
            ],
        );
        repo.main("aaa");
        // And a repo-level link into the store that leads nowhere.
        repo.store_link_only(&sha(b"sin destino"), &"c".repeat(64));

        let found = scan_fixture(&fx);
        let model = only_model(&found);
        assert_eq!(model.files.len(), 1);
        assert!(model.incomplete);
    }

    #[test]
    fn a_snapshot_with_shards_missing_is_incomplete_like_the_real_interrupted_download() {
        let fx = Fixture::new();
        let repo = fx.hf_repo("org/m");
        let index = br#"{"weight_map":{"a":"model-00001-of-00008.safetensors","b":"model-00002-of-00008.safetensors"}}"#;
        let (index_etag, shard_etag) = ("a".repeat(40), sha(b"shard"));
        repo.blob(&index_etag, index);
        repo.blob(&shard_etag, b"shard");
        repo.snapshot(
            "aaa",
            &[
                ("model.safetensors.index.json", &index_etag),
                ("model-00002-of-00008.safetensors", &shard_etag),
            ],
        );
        repo.main("aaa");
        assert!(only_model(&scan_fixture(&fx)).incomplete);
    }

    #[test]
    fn a_repo_holding_only_refs_is_not_a_model() {
        // Seen on the real machine: what's left when a download goes straight to a local folder.
        let fx = Fixture::new();
        let repo = fx.hf_repo("org/vacio");
        repo.main("aaa");
        assert!(scan_fixture(&fx).models.is_empty());
    }

    #[test]
    fn blobs_no_snapshot_uses_are_flagged_once_the_repo_is_quiet() {
        let fx = Fixture::new();
        let repo = fx.hf_repo("org/m");
        let (used, unused) = (sha(b"usado"), sha(b"nadie"));
        repo.blob(&used, b"usado");
        let orphan = repo.blob(&unused, &[1u8; 5000]);
        repo.snapshot("aaa", &[("model.safetensors", &used)]);
        repo.main("aaa");

        assert!(scan_fixture(&fx).leftovers.is_empty());
        fx.age_tree(".cache/huggingface/hub", 30);
        let found = scan_fixture(&fx);
        assert_eq!(found.leftovers.len(), 1);
        assert_eq!(found.leftovers[0].kind, LeftoverKind::HfUnreferenced);
        assert_eq!(found.leftovers[0].paths, vec![orphan]);
        assert!(found.leftovers[0].bytes() >= 5000);
    }

    #[test]
    fn old_revisions_are_flagged_keeping_what_the_current_one_shares() {
        let fx = Fixture::new();
        let repo = fx.hf_repo("org/m");
        let (shared, old, new) = (sha(b"compartido"), sha(b"viejo"), sha(b"nuevo"));
        let shared_blob = repo.blob(&shared, b"compartido");
        let old_blob = repo.blob(&old, &[2u8; 9000]);
        let new_blob = repo.blob(&new, b"nuevo");
        let old_snapshot = repo.snapshot("viejo", &[("a", &shared), ("b", &old)]);
        repo.snapshot("nuevo", &[("a", &shared), ("b", &new)]);
        repo.main("nuevo");
        fx.age_tree(".cache/huggingface/hub", 30);

        let found = scan_fixture(&fx);
        let leftover = found
            .leftovers
            .iter()
            .find(|leftover| leftover.kind == LeftoverKind::HfOldRevision)
            .unwrap();
        assert!(leftover.paths.contains(&old_snapshot));
        assert!(leftover.paths.contains(&old_blob));
        assert!(!leftover.paths.contains(&shared_blob));
        assert!(!leftover.paths.contains(&new_blob));
        assert_eq!(
            only_model(&found).files.len(),
            2,
            "the model is the current revision"
        );
    }

    #[test]
    fn a_repo_with_no_resolving_ref_keeps_all_its_revisions() {
        let fx = Fixture::new();
        let repo = fx.hf_repo("org/m");
        let etag = sha(b"pesos");
        repo.blob(&etag, b"pesos");
        repo.snapshot("fijada", &[("a", &etag)]);
        repo.main("no-existe");
        fx.age_tree(".cache/huggingface/hub", 30);
        let found = scan_fixture(&fx);
        assert!(found.leftovers.is_empty());
        assert_eq!(only_model(&found).files.len(), 1);
    }

    #[test]
    fn store_payloads_used_only_by_a_dataset_are_not_orphans() {
        let fx = Fixture::new();
        let dataset = fx.hf_repo_kind("datasets", "org/d");
        let etag = sha(b"datos");
        let payload = dataset.store_blob(&etag, &"d".repeat(64), b"datos");
        dataset.snapshot("aaa", &[("train.parquet", &etag)]);
        dataset.main("aaa");
        fx.age_tree(".cache/huggingface/hub", 30);
        let found = scan_fixture(&fx);
        assert!(found.models.is_empty());
        assert!(found.leftovers.is_empty(), "{:?}", found.leftovers);
        assert!(payload.exists());
        assert_eq!(found.dataset_repos.len(), 1);
    }

    #[test]
    fn store_orphans_need_a_manifest_and_a_complete_view() {
        let fx = Fixture::new();
        let repo = fx.hf_repo("org/m");
        let etag = sha(b"usado");
        repo.store_blob(&etag, &"e".repeat(64), b"usado");
        repo.snapshot("aaa", &[("m", &etag)]);
        repo.main("aaa");
        let orphan = repo.store_payload_only(&"f".repeat(64), &[3u8; 7000], true);
        let no_manifest = repo.store_payload_only(&"9".repeat(64), &[4u8; 7000], false);
        fx.age_tree(".cache/huggingface/hub", 30);

        let found = scan_fixture(&fx);
        let leftover = found
            .leftovers
            .iter()
            .find(|leftover| leftover.kind == LeftoverKind::HfStoreOrphan)
            .unwrap();
        assert!(leftover.paths.contains(&orphan));
        assert!(leftover.paths.contains(&manifest_of(&orphan)));
        assert!(!leftover.paths.contains(&no_manifest));
    }

    #[test]
    fn nothing_is_called_a_store_orphan_without_the_marker() {
        let fx = Fixture::new();
        let repo = fx.hf_repo("org/m");
        repo.store_payload_only(&"f".repeat(64), &[3u8; 7000], true);
        fs::remove_file(
            fx.home
                .join(".cache/huggingface/hub/blobs")
                .join(STORE_MARKER),
        )
        .unwrap();
        fx.age_tree(".cache/huggingface/hub", 30);
        assert!(scan_fixture(&fx).leftovers.is_empty());
    }

    #[test]
    fn a_blob_link_only_counts_as_the_last_when_no_other_repo_links_there() {
        let fx = Fixture::new();
        let a = fx.hf_repo("org/a");
        let b = fx.hf_repo("org/b");
        let etag = sha(b"igual en ambos");
        let xet = "a".repeat(64);
        let payload = a.store_blob(&etag, &xet, b"igual en ambos");
        b.store_link_only(&etag, &xet);
        a.snapshot("aaa", &[("otro", &etag)]);
        a.main("aaa");
        // b has it, with no snapshot: an unreferenced link in b. a still needs the payload.
        fx.age_tree(".cache/huggingface/hub", 30);
        let found = scan_fixture(&fx);
        let leftover = found
            .leftovers
            .iter()
            .find(|leftover| leftover.kind == LeftoverKind::HfUnreferenced)
            .unwrap();
        assert_eq!(leftover.label, "org/b");
        assert!(!leftover.paths.contains(&payload));
        assert_eq!(leftover.bytes(), 0, "removing the link frees nothing");
    }

    #[test]
    fn incomplete_downloads_after_a_week_only() {
        let fx = Fixture::new();
        let repo = fx.hf_repo("org/m");
        let partial = repo.blob(&format!("{}.incomplete", sha(b"x")), &[1u8; 8000]);
        assert!(scan_fixture(&fx).leftovers.is_empty());
        fx.age_tree(".cache/huggingface/hub", 8);
        let found = scan_fixture(&fx);
        assert_eq!(found.leftovers.len(), 1);
        assert_eq!(found.leftovers[0].kind, LeftoverKind::Partial);
        assert_eq!(found.leftovers[0].paths, vec![partial]);
    }

    #[test]
    fn locks_are_stale_only_if_nothing_in_the_folder_is_recent() {
        let fx = Fixture::new();
        fx.write(".cache/huggingface/hub/.locks/models--a--old/x.lock", b"");
        fx.write(".cache/huggingface/hub/.locks/models--a--mix/old.lock", b"");
        fx.age_tree(".cache/huggingface/hub/.locks", 30);
        fx.write(".cache/huggingface/hub/.locks/models--a--mix/new.lock", b"");
        let locks = scan_fixture(&fx).stale_locks;
        assert_eq!(locks.len(), 1);
        assert!(locks[0].ends_with("models--a--old"));
    }

    #[test]
    fn removal_takes_exclusive_payloads_and_leaves_shared_ones() {
        let fx = Fixture::new();
        let a = fx.hf_repo("org/a");
        let b = fx.hf_repo("org/b");
        let d = fx.hf_repo_kind("datasets", "org/d");
        let exclusive = a.store_blob(&sha(b"solo a"), &"1".repeat(64), &[1u8; 6000]);
        let with_b = a.store_blob(&sha(b"a y b"), &"2".repeat(64), &[2u8; 6000]);
        b.store_link_only(&sha(b"a y b"), &"2".repeat(64));
        let with_dataset = a.store_blob(&sha(b"a y d"), &"3".repeat(64), &[3u8; 6000]);
        d.store_link_only(&sha(b"a y d"), &"3".repeat(64));

        let (paths, bytes) = repo_removal(a.dir());
        assert!(paths.contains(&a.dir().to_path_buf()));
        assert!(paths.contains(&exclusive));
        assert!(paths.contains(&manifest_of(&exclusive)));
        assert!(!paths.contains(&with_b));
        assert!(!paths.contains(&with_dataset));
        assert!(bytes >= 6000);
    }

    #[test]
    fn removal_leaves_payloads_when_the_marker_is_gone() {
        let fx = Fixture::new();
        let a = fx.hf_repo("org/a");
        a.store_blob(&sha(b"x"), &"1".repeat(64), b"x");
        fs::remove_file(
            fx.home
                .join(".cache/huggingface/hub/blobs")
                .join(STORE_MARKER),
        )
        .unwrap();
        let (paths, _) = repo_removal(a.dir());
        assert_eq!(paths, vec![a.dir().to_path_buf()]);
    }

    #[test]
    fn a_link_leading_into_a_protected_folder_is_not_followed() {
        let fx = Fixture::new();
        let secret = fx.write("Documents/privado.bin", b"privado");
        let repo = fx.hf_repo("org/m");
        repo.snapshot_link("aaa", "model.safetensors", &secret);
        repo.main("aaa");
        let found = scan_fixture(&fx);
        assert!(found.models.is_empty(), "{:?}", found.models);
    }
}

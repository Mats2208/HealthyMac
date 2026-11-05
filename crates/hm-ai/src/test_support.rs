//! Fixture homes with the real shapes of each runtime's folders.

use std::fs::{self, File};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use hm_core::fsutil::{sha256_file, CancelToken};
use hm_core::{Config, ScanContext};

pub(crate) fn sha(bytes: &[u8]) -> String {
    let file = tempfile::NamedTempFile::new().unwrap();
    fs::write(file.path(), bytes).unwrap();
    sha256_file(file.path(), &CancelToken::new()).unwrap()
}

pub(crate) struct Fixture {
    _dir: tempfile::TempDir,
    pub home: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        Self { _dir: dir, home }
    }

    pub fn ctx(&self) -> ScanContext {
        ScanContext::with_home(&self.home, Config::default())
    }

    pub fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.home.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    /// Sets a file's modification and access time to `days` ago. Doesn't follow symlinks in that it
    /// isn't asked to: callers pass real files, and `age_tree` filters.
    pub fn set_age(&self, path: &Path, days: u64) {
        let when = SystemTime::now() - Duration::from_secs(days * 86_400);
        let times = fs::FileTimes::new().set_modified(when).set_accessed(when);
        let file = File::open(path).unwrap();
        file.set_times(times).unwrap();
    }

    /// Ages everything under a home-relative folder, deepest first so a parent's time isn't bumped
    /// again. Symlinks are left alone.
    pub fn age_tree(&self, relative: &str, days: u64) {
        fn visit(fx: &Fixture, path: &Path, days: u64) {
            let meta = fs::symlink_metadata(path).unwrap();
            if meta.file_type().is_symlink() {
                return;
            }
            if meta.is_dir() {
                for entry in fs::read_dir(path).unwrap() {
                    visit(fx, &entry.unwrap().path(), days);
                }
            }
            fx.set_age(path, days);
        }
        visit(self, &self.home.join(relative), days);
    }

    /// Writes a blob under its digest and returns the path.
    pub fn ollama_blob(&self, bytes: &[u8]) -> PathBuf {
        self.write(
            &format!(".ollama/models/blobs/sha256-{}", sha(bytes)),
            bytes,
        )
    }

    /// `path` is `<registry>/<namespace>/<model>/<tag>`. Each layer is `(kind, bytes)`, with a kind
    /// like `model` or `license`. A config blob is added.
    pub fn ollama_model(&self, path: &str, layers: &[(&str, &[u8])]) -> PathBuf {
        let config = br#"{"model_format":"gguf","model_family":"llama","model_type":"7.2B","file_type":"Q4_0"}"#;
        self.ollama_blob(config);
        let mut entries = Vec::new();
        for (kind, bytes) in layers {
            self.ollama_blob(bytes);
            entries.push(format!(
                r#"{{"mediaType":"application/vnd.ollama.image.{kind}","digest":"sha256:{}","size":{}}}"#,
                sha(bytes),
                bytes.len()
            ));
        }
        let manifest = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.docker.distribution.manifest.v2+json",
                "config":{{"mediaType":"application/vnd.docker.container.image.v1+json","digest":"sha256:{}","size":{}}},
                "layers":[{}]}}"#,
            sha(config),
            config.len(),
            entries.join(",")
        );
        self.write(
            &format!(".ollama/models/manifests/{path}"),
            manifest.as_bytes(),
        )
    }

    pub fn hf_repo(&self, id: &str) -> HfRepo {
        self.hf_repo_kind("models", id)
    }

    pub fn hf_repo_kind(&self, kind: &str, id: &str) -> HfRepo {
        let hub = self.home.join(".cache/huggingface/hub");
        let dir = hub.join(format!("{kind}--{}", id.replace('/', "--")));
        fs::create_dir_all(&dir).unwrap();
        HfRepo { hub, dir }
    }
}

pub(crate) struct HfRepo {
    hub: PathBuf,
    dir: PathBuf,
}

impl HfRepo {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A real file in the repository's own `blobs/`, the older layout.
    pub fn blob(&self, etag: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join("blobs").join(etag);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    fn mark_store(&self) {
        let store = self.hub.join("blobs");
        fs::create_dir_all(&store).unwrap();
        fs::write(store.join(".huggingface-shared-blobs"), "1\n").unwrap();
    }

    /// A payload in the shared store, read-only as the real ones are.
    pub fn store_payload_only(&self, xet: &str, bytes: &[u8], with_manifest: bool) -> PathBuf {
        self.mark_store();
        let path = self.hub.join("blobs").join(&xet[..2]).join(xet);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        if with_manifest {
            fs::write(path.with_file_name(format!("{xet}.refs")), "x\n").unwrap();
        }
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    /// `blobs/<etag>` as a relative symlink into the store, whether or not the payload exists.
    pub fn store_link_only(&self, etag: &str, xet: &str) {
        self.mark_store();
        let link = self.dir.join("blobs").join(etag);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(format!("../../blobs/{}/{xet}", &xet[..2]), link).unwrap();
    }

    /// A blob in the shared layout: the payload, and the repository's link to it.
    pub fn store_blob(&self, etag: &str, xet: &str, bytes: &[u8]) -> PathBuf {
        let payload = self.store_payload_only(xet, bytes, true);
        self.store_link_only(etag, xet);
        payload
    }

    /// Creates a snapshot of `(file name, etag)` as relative symlinks, nested names included.
    pub fn snapshot(&self, commit: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = self.dir.join("snapshots").join(commit);
        fs::create_dir_all(&root).unwrap();
        for (name, etag) in files {
            let link = root.join(name);
            fs::create_dir_all(link.parent().unwrap()).unwrap();
            let depth = name.matches('/').count();
            let up = "../".repeat(depth + 2);
            symlink(format!("{up}blobs/{etag}"), link).unwrap();
        }
        root
    }

    /// A snapshot file linking to an absolute path anywhere.
    pub fn snapshot_link(&self, commit: &str, name: &str, target: &Path) {
        let root = self.dir.join("snapshots").join(commit);
        fs::create_dir_all(&root).unwrap();
        symlink(target, root.join(name)).unwrap();
    }

    pub fn main(&self, commit: &str) {
        let refs = self.dir.join("refs");
        fs::create_dir_all(&refs).unwrap();
        fs::write(refs.join("main"), commit).unwrap();
    }

    /// A second ref, in a nested folder as pull request refs are.
    pub fn main_also(&self, commit: &str) {
        let refs = self.dir.join("refs/pr");
        fs::create_dir_all(&refs).unwrap();
        fs::write(refs.join("1"), commit).unwrap();
    }
}

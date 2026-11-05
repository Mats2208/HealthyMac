//! Fixture homes for tests. Nothing here ever touches the real home directory.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use hm_core::{Config, Finding, ScanContext};

pub(crate) struct Fixture {
    pub dir: tempfile::TempDir,
    pub ctx: ScanContext,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = ScanContext::with_home(dir.path(), Config::default());
        Self { dir, ctx }
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    pub fn mkdir(&self, relative: &str) -> PathBuf {
        let path = self.path(relative);
        fs::create_dir_all(&path).expect("mkdir");
        path
    }

    /// Real, allocated bytes, each file with its own content so APFS does not share blocks.
    pub fn write(&self, relative: &str, bytes: usize) -> PathBuf {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(&path, vec![7u8; bytes]).expect("write");
        path
    }

    pub fn write_text(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(&path, text).expect("write");
        path
    }

    /// `has_full_disk_access` probes these folders, so a readable one reads as granted.
    pub fn grant_full_disk_access(&self) {
        self.mkdir("Library/Safari");
    }

    /// A minimal `.app` bundle.
    pub fn app(&self, relative: &str, bundle_id: &str, executable: &str) -> PathBuf {
        let path = self.mkdir(relative);
        self.write_text(
            &format!("{relative}/Contents/Info.plist"),
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{bundle_id}</string>
<key>CFBundleExecutable</key><string>{executable}</string>
<key>CFBundleShortVersionString</key><string>1.2.3</string>
</dict></plist>"#
            ),
        );
        self.write(&format!("{relative}/Contents/MacOS/{executable}"), 64);
        path
    }
}

/// Runs a test on a large stack. For trees at `PATH_MAX`, where creating and removing the fixture
/// overflows a normal test thread. The code under test still recurses on rayon's workers, which keep
/// their usual stack, so it is not helped by this.
pub(crate) fn with_big_stack(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(test)
        .expect("spawn")
        .join()
        .expect("test thread");
}

/// Makes a directory unreadable for the duration of a test. Restored on drop, so the tempdir can
/// still be cleaned up.
pub(crate) struct LockedDir(PathBuf);

impl LockedDir {
    pub fn new(path: &Path) -> Self {
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).expect("chmod");
        Self(path.to_path_buf())
    }
}

impl Drop for LockedDir {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

pub(crate) fn find<'a>(findings: &'a [Finding], key: &str) -> Option<&'a Finding> {
    findings
        .iter()
        .find(|finding| finding.id.ends_with(&format!(":{key}")))
}

pub(crate) fn targets(finding: &Finding) -> Vec<PathBuf> {
    finding
        .target_paths()
        .iter()
        .map(|path| path.to_path_buf())
        .collect()
}

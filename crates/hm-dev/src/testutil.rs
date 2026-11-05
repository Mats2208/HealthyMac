//! A fixture home for tests. Nothing here ever touches the real one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use std::time::{Duration, SystemTime};

use hm_core::{Config, ScanContext};

/// Keeps git's behaviour in tests independent of the machine's global config. A developer's global
/// ignore file listing `dist/` would otherwise make "not ignored" tests depend on who runs them.
fn isolate_git_config() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("XDG_CONFIG_HOME", "/nonexistent");
    });
}

pub(crate) struct Fixture {
    _dir: tempfile::TempDir,
    pub(crate) home: PathBuf,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        isolate_git_config();
        let dir = tempfile::tempdir().expect("temporary directory");
        // On macOS the temporary directory is a symlink, and sweeps report resolved paths.
        let home = dir
            .path()
            .canonicalize()
            .expect("canonical temporary directory");
        Self { _dir: dir, home }
    }

    pub(crate) fn ctx(&self) -> ScanContext {
        ScanContext::with_home(&self.home, Config::default())
    }

    pub(crate) fn file(&self, relative: &str, content: &[u8]) -> PathBuf {
        let path = self.home.join(relative);
        fs::create_dir_all(path.parent().expect("a parent")).expect("create parents");
        fs::write(&path, content).expect("write file");
        path
    }

    pub(crate) fn dir(&self, relative: &str) -> PathBuf {
        let path = self.home.join(relative);
        fs::create_dir_all(&path).expect("create directory");
        path
    }

    pub(crate) fn symlink(&self, target: &str, link: &str) {
        let link = self.home.join(link);
        fs::create_dir_all(link.parent().expect("a parent")).expect("create parents");
        std::os::unix::fs::symlink(self.home.join(target), link).expect("create symlink");
    }

    pub(crate) fn set_mtime(&self, relative: &str, when: SystemTime) {
        let file = fs::File::options()
            .write(true)
            .open(self.home.join(relative))
            .expect("open file");
        file.set_modified(when).expect("set modification time");
    }

    /// Backdates a file so that it looks untouched for this many days.
    pub(crate) fn set_age(&self, relative: &str, days: u64) {
        self.set_mtime(
            relative,
            SystemTime::now() - Duration::from_secs(86_400 * days),
        );
    }

    /// Runs git inside the fixture, in a directory that is created if needed.
    pub(crate) fn git(&self, relative: &str, args: &[&str]) {
        let dir = self.dir(relative);
        let output = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} in {}: {}",
            Path::new(relative).display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

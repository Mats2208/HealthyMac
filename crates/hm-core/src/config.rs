//! User configuration, stored as JSON.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalMode {
    /// Move to the Trash. Recoverable, but no space is freed until the Trash is emptied.
    Trash,
    /// Delete immediately. Only honoured for [`crate::Risk::Safe`] findings that regenerate on
    /// their own. Everything else still goes to the Trash.
    DeleteRegenerable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Folders swept for development projects. When empty, [`Config::effective_project_roots`]
    /// falls back to common names under the home directory.
    pub project_roots: Vec<PathBuf>,
    /// A project counts as idle after this many days without activity.
    pub project_idle_days: u32,
    /// Minimum size for the large-files scanner.
    pub large_file_min_bytes: u64,
    /// Paths the user never wants touched. Enforced by the safety policy.
    pub excluded_paths: Vec<PathBuf>,
    pub removal_mode: RemovalMode,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            project_roots: Vec::new(),
            project_idle_days: 14,
            large_file_min_bytes: 1_000_000_000,
            excluded_paths: Vec::new(),
            removal_mode: RemovalMode::Trash,
        }
    }
}

/// Deliberately excludes `Documents`, `Desktop` and `Downloads`: those are TCC-protected, and
/// touching them would raise a permission prompt in the middle of a scan. They are only swept if
/// the user adds them to `project_roots`.
const DEFAULT_PROJECT_ROOT_NAMES: &[&str] = &[
    "Coding",
    "Code",
    "code",
    "Projects",
    "projects",
    "Developer",
    "dev",
    "Dev",
    "src",
    "repos",
    "Repos",
    "workspace",
    "Workspace",
    "git",
    "GitHub",
    "github",
    "Sites",
];

impl Config {
    pub fn default_path(home: &Path) -> PathBuf {
        home.join("Library/Application Support/HealthyMac/config.json")
    }

    /// Loads the config, returning defaults when the file does not exist.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("config inválida: {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("no se pudo leer {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    /// The configured roots, or the common folder names under `home` that exist.
    pub fn effective_project_roots(&self, home: &Path) -> Vec<PathBuf> {
        if !self.project_roots.is_empty() {
            return self.project_roots.clone();
        }
        let mut roots: Vec<PathBuf> = Vec::new();
        for name in DEFAULT_PROJECT_ROOT_NAMES {
            let candidate = home.join(name);
            if !candidate.is_dir() {
                continue;
            }
            // APFS is usually case-insensitive, so "Code" and "code" can be the same folder.
            let canonical = candidate.canonicalize().unwrap_or(candidate);
            if !roots.contains(&canonical) {
                roots.push(canonical);
            }
        }
        roots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::load(&dir.path().join("nope.json")).unwrap();
        assert_eq!(cfg.removal_mode, RemovalMode::Trash);
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/config.json");
        let cfg = Config {
            project_idle_days: 3,
            ..Config::default()
        };
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().project_idle_days, 3);
    }

    #[test]
    fn default_roots_skip_tcc_protected_folders() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("Documents")).unwrap();
        fs::create_dir(dir.path().join("Coding")).unwrap();
        let roots = Config::default().effective_project_roots(dir.path());
        assert_eq!(roots.len(), 1);
        assert!(roots[0].ends_with("Coding"));
    }
}

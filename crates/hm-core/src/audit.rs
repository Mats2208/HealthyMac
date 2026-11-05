//! Append-only record of everything the cleaner has done, one JSON object per line.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::clean::CleanOutcome;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEntry {
    pub at: SystemTime,
    pub title: String,
    pub outcome: CleanOutcome,
}

#[derive(Clone, Debug)]
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn default_path(home: &Path) -> PathBuf {
        home.join("Library/Logs/HealthyMac/history.jsonl")
    }

    pub fn append(&self, entry: &AuditEntry) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut line = serde_json::to_vec(entry)?;
        line.push(b'\n');
        file.write_all(&line)?;
        Ok(())
    }

    /// All readable entries, oldest first. Corrupt lines are skipped.
    pub fn read_all(&self) -> Result<Vec<AuditEntry>> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };
        Ok(text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect())
    }
}

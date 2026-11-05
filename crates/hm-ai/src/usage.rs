//! When was a model last used? This is what decides whether a model gets called stale, so it only
//! ever trusts a record the runtime itself wrote.
//!
//! **Why not access time.** It looks like the obvious signal and it is wrong in both directions.
//! Measured on this project's reference machine (macOS 27, an APFS Data volume mounted without
//! `noatime`): a plain `read()` of a file whose atime was 20 days old left it unchanged, while
//! `mmap`ing the same file did update it. So a runtime loading weights with plain reads can use a
//! model daily and leave it looking untouched, which is how a model in use would get flagged. And
//! Spotlight, backups and this very scanner's own reads of headers can make an unused model look
//! fresh. Nothing can tell the cases apart, so atime isn't consulted at all.
//!
//! **Why not modification time.** It says when the files were downloaded. It's only offered as the
//! weakest answer, labelled [`crate::types::UsageSignal::Modified`], and never leads to a model
//! being flagged.
//!
//! **What's left**, per runtime:
//! - LM Studio: `.internal/model-data.json` keeps `lastLoadedTimestamp` per model.
//! - oMLX: `usage.sqlite3` keeps hourly per-model totals for 400 days, for its own folders and the
//!   Hugging Face models it serves.
//! - Ollama: no history, but its server log names the blob each runner loads. Logs are dropped
//!   oldest first, so a line that is present really is the last use if no later line follows it.
//!   A model with no line says nothing: the record of it may simply have rotated away.
//! - Hugging Face, Whisper, ComfyUI and the rest: nothing is recorded. Those models are never
//!   flagged as stale, however old they look, since nothing supports the claim.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::files::{is_sha256, read_capped, sqlite_query};
use crate::locate::Roots;

const MODEL_DATA_CAP: u64 = 64 * 1024 * 1024;
const LOG_TAIL_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Default)]
pub(crate) struct UsageRecords {
    /// Keyed by LM Studio's model id: `publisher/model` or `publisher/model/file.gguf`.
    pub lm_studio: HashMap<String, SystemTime>,
    /// LM Studio's short API identifiers to its model ids, for matching what is loaded.
    pub lm_identifiers: HashMap<String, String>,
    /// Keyed by oMLX's model id: the folder name, or `org--name` for Hugging Face models.
    pub omlx: HashMap<String, SystemTime>,
    /// Keyed by the SHA-256 of the blob a runner loaded.
    pub ollama_blobs: HashMap<String, SystemTime>,
}

impl UsageRecords {
    pub fn load(roots: &Roots) -> Self {
        let mut records = Self {
            ollama_blobs: read_ollama_logs(&roots.ollama_logs),
            ..Self::default()
        };
        if roots.allowed_quietly(&roots.lm_home) {
            let internal = roots.lm_home.join(".internal");
            records.lm_studio = read_capped(&internal.join("model-data.json"), MODEL_DATA_CAP)
                .map(|bytes| parse_lm_model_data(&bytes))
                .unwrap_or_default();
            records.lm_identifiers =
                read_capped(&internal.join("model-index-cache.json"), MODEL_DATA_CAP)
                    .map(|bytes| parse_lm_index(&bytes))
                    .unwrap_or_default();
        }
        if roots.allowed_quietly(&roots.omlx.base) {
            records.omlx = read_omlx_usage(&roots.omlx.base.join("usage.sqlite3"));
        }
        records
    }
}

fn from_millis(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

/// The file is superjson: `{"json": [[id, {..}], ..], "meta": ..}`. A failed load attempt counts:
/// someone wanted that model, so it isn't forgotten. An entry with neither timestamp says nothing,
/// because loading through a hub alias records the time on the alias and not on the concrete model.
fn parse_lm_model_data(bytes: &[u8]) -> HashMap<String, SystemTime> {
    let mut records = HashMap::new();
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return records;
    };
    let Some(entries) = value.get("json").and_then(|json| json.as_array()) else {
        return records;
    };
    for entry in entries {
        let Some([id, data]) = entry.as_array().map(Vec::as_slice) else {
            continue;
        };
        let Some(id) = id.as_str() else {
            continue;
        };
        let newest = ["lastLoadedTimestamp", "lastAttemptedToLoadTimestamp"]
            .iter()
            .filter_map(|key| data.get(key)?.as_u64())
            .max();
        if let Some(newest) = newest {
            records.insert(id.to_string(), from_millis(newest));
        }
    }
    records
}

fn parse_lm_index(bytes: &[u8]) -> HashMap<String, String> {
    let mut identifiers = HashMap::new();
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return identifiers;
    };
    let Some(models) = value.get("models").and_then(|models| models.as_array()) else {
        return identifiers;
    };
    for model in models {
        let Some(id) = model
            .get("indexedModelIdentifier")
            .and_then(|id| id.as_str())
        else {
            continue;
        };
        identifiers.insert(id.to_lowercase(), id.to_string());
        if let Some(short) = model
            .get("defaultIdentifier")
            .and_then(|short| short.as_str())
        {
            identifiers.insert(short.to_lowercase(), id.to_string());
        }
        let auto = model
            .get("autoIdentifiers")
            .and_then(|auto| auto.as_array());
        for short in auto
            .into_iter()
            .flatten()
            .filter_map(|short| short.as_str())
        {
            identifiers.insert(short.to_lowercase(), id.to_string());
        }
    }
    identifiers
}

fn read_omlx_usage(database: &Path) -> HashMap<String, SystemTime> {
    if !database.is_file() {
        return HashMap::new();
    }
    let sql = "SELECT model_id || char(9) || MAX(timestamp_hour) \
               FROM model_usage_hourly GROUP BY model_id;";
    let Some(output) = sqlite_query(database, sql) else {
        return HashMap::new();
    };
    output
        .lines()
        .filter_map(|line| {
            let (id, hour) = line.rsplit_once('\t')?;
            Some((
                id.to_string(),
                UNIX_EPOCH + Duration::from_secs(hour.parse().ok()?),
            ))
        })
        .collect()
}

fn read_ollama_logs(dir: &Path) -> HashMap<String, SystemTime> {
    let mut records: HashMap<String, SystemTime> = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return records;
    };
    for entry in entries.flatten().take(32) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("server") || !name.ends_with(".log") {
            continue;
        }
        if let Some(text) = read_tail(&entry.path()) {
            collect_ollama_log(&text, &mut records);
        }
    }
    records
}

/// The end of a log, dropping a first line that may be cut in half.
fn read_tail(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(LOG_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(LOG_TAIL_BYTES).read_to_end(&mut bytes).ok()?;
    if start > 0 {
        let first_break = bytes.iter().position(|byte| *byte == b'\n')?;
        bytes.drain(..=first_break);
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Looks for lines like these, older and current versions respectively:
///
/// `time=2025-03-01T10:00:00.123-03:00 level=INFO source=server.go:380 msg="starting llama server"
/// cmd="... --model /Users/x/.ollama/models/blobs/sha256-abc... --ctx-size 8192"`
///
/// `time=... level=INFO msg="using llama-server for model" model=/.../blobs/sha256-abc...`
fn collect_ollama_log(text: &str, records: &mut HashMap<String, SystemTime>) {
    const MARKER: &str = "blobs/sha256-";
    for line in text.lines() {
        let loads_a_model = line.contains("--model ")
            || line.contains("using llama-server")
            || line.contains("model=");
        if !loads_a_model
            || line.contains("remov")
            || line.contains("prun")
            || line.contains("delet")
        {
            continue;
        }
        let Some(rest) = line.strip_prefix("time=") else {
            continue;
        };
        let Some(time) = rest.split_whitespace().next().and_then(parse_rfc3339) else {
            continue;
        };
        for (at, _) in line.match_indices(MARKER) {
            let Some(digest) = line.get(at + MARKER.len()..at + MARKER.len() + 64) else {
                continue;
            };
            if is_sha256(digest) {
                let entry = records.entry(digest.to_string()).or_insert(time);
                *entry = (*entry).max(time);
            }
        }
    }
}

/// `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`.
pub(crate) fn parse_rfc3339(text: &str) -> Option<SystemTime> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    if text.get(4..5)? != "-" || text.get(7..8)? != "-" || text.get(10..11)? != "T" {
        return None;
    }
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // Days since the epoch, from Howard Hinnant's `days_from_civil`.
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let year_of_era = shifted.rem_euclid(400);
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;

    let zone = text.get(19..)?;
    let zone = zone.get(zone.find(['Z', 'z', '+', '-'])?..)?;
    let offset = match zone.chars().next()? {
        'Z' | 'z' => 0,
        sign => {
            let hours: i64 = zone.get(1..3)?.parse().ok()?;
            let minutes: i64 = zone.get(4..6)?.parse().ok()?;
            let magnitude = hours * 3600 + minutes * 60;
            if sign == '-' {
                -magnitude
            } else {
                magnitude
            }
        }
    };
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset;
    u64::try_from(seconds)
        .ok()
        .map(|secs| UNIX_EPOCH + Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "8eeb52dfb3bb9aefdf9d1ef24b3bdbcfbe82238798c4b918278320b6fcef18fe";

    #[test]
    fn reads_lm_studio_load_times_from_its_superjson_shape() {
        let records = parse_lm_model_data(
            br#"{"json":[
                ["a/loaded",{"lastLoadedTimestamp":1792583849155,"transitive":false}],
                ["a/attempted",{"lastLoadedTimestamp":1000,"lastAttemptedToLoadTimestamp":2000}],
                ["a/never",{"transitive":false}],
                ["bad"],
                [1,{"lastLoadedTimestamp":5}]
            ],"meta":{}}"#,
        );
        assert_eq!(records.len(), 2);
        assert_eq!(records["a/loaded"], from_millis(1792583849155));
        assert_eq!(records["a/attempted"], from_millis(2000));
        assert!(parse_lm_model_data(b"{nope").is_empty());
        assert!(parse_lm_model_data(br#"{"json":{"map":1}}"#).is_empty());
    }

    #[test]
    fn maps_lm_studio_short_identifiers() {
        let identifiers = parse_lm_index(
            br#"{"models":[{"indexedModelIdentifier":"Org/Model-MLX-4bit",
                "defaultIdentifier":"model-mlx","autoIdentifiers":["org/model-mlx@4bit"]}]}"#,
        );
        assert_eq!(identifiers["model-mlx"], "Org/Model-MLX-4bit");
        assert_eq!(identifiers["org/model-mlx@4bit"], "Org/Model-MLX-4bit");
        assert_eq!(identifiers["org/model-mlx-4bit"], "Org/Model-MLX-4bit");
    }

    #[test]
    fn reads_the_real_shape_of_the_omlx_database() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("with space/usage.sqlite3");
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        let created = hm_core::exec::run(
            Path::new("/usr/bin/sqlite3"),
            &[
                database.to_str().unwrap(),
                "CREATE TABLE model_usage_hourly (timestamp_hour INTEGER, model_id TEXT, \
                 PRIMARY KEY (timestamp_hour, model_id)) WITHOUT ROWID; \
                 INSERT INTO model_usage_hourly VALUES (1000,'m'),(5000,'m'),(2000,'org--n');",
            ],
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(created.success(), "{}", created.stderr);
        let usage = read_omlx_usage(&database);
        assert_eq!(usage["m"], UNIX_EPOCH + Duration::from_secs(5000));
        assert_eq!(usage["org--n"], UNIX_EPOCH + Duration::from_secs(2000));

        let garbage = dir.path().join("garbage.sqlite3");
        std::fs::write(&garbage, b"esto no es una base de datos").unwrap();
        assert!(read_omlx_usage(&garbage).is_empty());
        assert!(read_omlx_usage(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn takes_the_newest_load_from_ollama_logs_in_both_formats() {
        let log = format!(
            "time=2025-03-01T10:00:00.123-03:00 level=INFO source=server.go:380 msg=\"starting llama server\" cmd=\"/ollama runner --model /u/.ollama/models/blobs/sha256-{DIGEST} --ctx-size 8192\"\n\
             time=2025-03-05T10:00:00Z level=INFO msg=\"using llama-server for model\" model=/u/.ollama/models/blobs/sha256-{DIGEST}\n\
             time=2026-01-01T00:00:00Z level=INFO msg=\"removing unused layer\" model=/u/.ollama/models/blobs/sha256-{DIGEST}\n\
             not a log line blobs/sha256-{DIGEST} --model x\n\
             time=garbage --model blobs/sha256-{DIGEST}\n\
             time=2025-03-06T00:00:00Z --model blobs/sha256-cortado\n"
        );
        let mut records = HashMap::new();
        collect_ollama_log(&log, &mut records);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[DIGEST],
            parse_rfc3339("2025-03-05T10:00:00Z").unwrap()
        );
    }

    #[test]
    fn parses_rfc3339_with_offsets() {
        assert_eq!(
            parse_rfc3339("1970-01-02T00:00:00Z"),
            Some(UNIX_EPOCH + Duration::from_secs(86_400))
        );
        assert_eq!(
            parse_rfc3339("2026-11-05T10:03:16.5-03:00"),
            parse_rfc3339("2026-11-05T13:03:16Z")
        );
        assert_eq!(parse_rfc3339("2026-13-05T10:03:16Z"), None);
        assert_eq!(parse_rfc3339("mañana"), None);
        assert_eq!(parse_rfc3339("2026-11-05T10:03:16"), None);
        assert_eq!(parse_rfc3339("1960-01-01T00:00:00Z"), None);
    }

    #[test]
    fn only_reads_the_end_of_a_huge_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.log");
        std::fs::write(&path, "vieja\n").unwrap();
        let file = File::options().write(true).open(&path).unwrap();
        file.set_len(LOG_TAIL_BYTES * 4).unwrap();
        let mut file = file;
        file.seek(SeekFrom::End(0)).unwrap();
        std::io::Write::write_all(&mut file, b"primera cortada\nnueva\n").unwrap();
        drop(file);
        let tail = read_tail(&path).unwrap();
        assert!(tail.ends_with("nueva\n"));
        assert!(!tail.contains("vieja"));
    }
}

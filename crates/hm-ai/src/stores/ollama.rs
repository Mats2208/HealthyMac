//! Ollama: `manifests/<registry>/<namespace>/<model>/<tag>` are JSON naming layers, and the layers
//! are content-addressed files in `blobs/`. Tags share layers freely.
//!
//! Orphan detection fails closed. One manifest that can't be read could be the only one referencing
//! any blob, so if any is unreadable nothing is called an orphan.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use hm_core::ScanContext;
use serde::Deserialize;

use super::{newest, Leftover, LeftoverKind, RawModel, StoreScan, ABANDONED_MIN_DAYS};
use crate::files::{is_sha256, name_of, older_than, read_capped, walk, FileRec};
use crate::gguf;
use crate::metadata::{parse_size_label, Detected};
use crate::types::{ModelFormat, RuntimeKind};
use crate::usage::UsageRecords;

const MANIFEST_CAP: u64 = 4 * 1024 * 1024;
const CONFIG_CAP: u64 = 4 * 1024 * 1024;
const MODEL_LAYER: &str = "application/vnd.ollama.image.model";

#[derive(Deserialize)]
struct Manifest {
    config: Option<Layer>,
    #[serde(default)]
    layers: Vec<Layer>,
}

#[derive(Deserialize)]
struct Layer {
    digest: String,
    #[serde(rename = "mediaType", default)]
    media_type: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ConfigBlob {
    model_format: String,
    model_family: String,
    model_type: String,
    file_type: String,
}

/// The hex of `sha256:<hex>`, or `None` for anything else. A manifest is input from a file, and this
/// is what stands between a crafted digest like `sha256:../../../x` and a path.
fn digest_hex(digest: &str) -> Option<&str> {
    digest
        .strip_prefix("sha256:")
        .or_else(|| digest.strip_prefix("sha256-"))
        .filter(|hex| is_sha256(hex))
}

/// Blobs are `sha256-<hex>`. Very old versions wrote `sha256:<hex>`.
fn blob_record(blobs: &Path, hex: &str, logical: &str) -> Option<FileRec> {
    [format!("sha256-{hex}"), format!("sha256:{hex}")]
        .iter()
        .find_map(|name| FileRec::stat(&blobs.join(name), logical))
        .map(|record| record.with_sha256(Some(hex.to_string())))
}

enum BlobName<'a> {
    Complete(&'a str),
    Partial(&'a str),
    Other,
}

/// A partial is `sha256-<hex>-partial`, with `-partial-<n>` files for the state of each part.
fn classify_blob(name: &str) -> BlobName<'_> {
    let Some(rest) = name
        .strip_prefix("sha256-")
        .or_else(|| name.strip_prefix("sha256:"))
    else {
        return BlobName::Other;
    };
    let Some(hex) = rest.get(..64).filter(|hex| is_sha256(hex)) else {
        return BlobName::Other;
    };
    match rest.get(64..) {
        Some("") => BlobName::Complete(hex),
        Some(tail) if tail.starts_with("-partial") => BlobName::Partial(hex),
        _ => BlobName::Other,
    }
}

/// `registry.ollama.ai/library/llama3/latest` is what `ollama list` calls `llama3:latest`.
fn display_name(parts: &[String]) -> Option<String> {
    let [registry, namespace, model, tag] = parts else {
        return None;
    };
    Some(if registry == "registry.ollama.ai" {
        if namespace == "library" {
            format!("{model}:{tag}")
        } else {
            format!("{namespace}/{model}:{tag}")
        }
    } else {
        format!("{registry}/{namespace}/{model}:{tag}")
    })
}

fn describe(root_blobs: &Path, manifest: &Manifest, config: Option<&ConfigBlob>) -> Detected {
    let mut detected = Detected::default();
    if let Some(config) = config {
        detected.format = match config.model_format.as_str() {
            "gguf" => ModelFormat::Gguf,
            "safetensors" => ModelFormat::Safetensors,
            _ => ModelFormat::Unknown,
        };
        detected.architecture =
            Some(config.model_family.clone()).filter(|family| !family.is_empty());
        detected.quantization = Some(config.file_type.clone()).filter(|kind| !kind.is_empty());
        detected.parameter_count = parse_size_label(&config.model_type);
    }
    let weights = manifest
        .layers
        .iter()
        .find(|layer| layer.media_type == MODEL_LAYER)
        .and_then(|layer| digest_hex(&layer.digest))
        .and_then(|hex| blob_record(root_blobs, hex, "model"));
    if let Some(info) = weights.and_then(|file| gguf::read_info(&file.path).ok()) {
        detected.format = ModelFormat::Gguf;
        detected.architecture = info.architecture.or(detected.architecture);
        detected.quantization = info.quantization.or(detected.quantization);
        detected.parameter_count = info.parameter_count.or(detected.parameter_count);
        detected.context_length = info.context_length;
    }
    detected
}

pub(crate) fn scan(root: &Path, ctx: &ScanContext, usage: &UsageRecords) -> StoreScan {
    let mut out = StoreScan::default();
    let manifests = root.join("manifests");
    let blobs = root.join("blobs");

    let mut files: Vec<(PathBuf, Vec<String>)> = Vec::new();
    let mut complete = manifests.is_dir();
    complete &= walk(&manifests, 8, ctx, &mut |path, meta| {
        let hidden = name_of(path).starts_with('.');
        if meta.is_file() && !hidden {
            let parts = path
                .strip_prefix(&manifests)
                .map(|relative| {
                    relative
                        .components()
                        .map(|part| part.as_os_str().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            files.push((path.to_path_buf(), parts));
        }
        !hidden
    });

    let mut referenced: HashSet<String> = HashSet::new();
    for (path, parts) in files {
        let manifest = read_capped(&path, MANIFEST_CAP)
            .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok())
            .filter(|manifest| manifest.config.is_some() || !manifest.layers.is_empty());
        let Some(manifest) = manifest else {
            complete = false;
            continue;
        };

        let digests: Vec<(&str, &str)> = manifest
            .layers
            .iter()
            .chain(manifest.config.iter())
            .map(|layer| (layer.digest.as_str(), layer.media_type.as_str()))
            .collect();
        let mut model_files = Vec::new();
        let mut missing = false;
        for (digest, media_type) in digests {
            let Some(hex) = digest_hex(digest) else {
                complete = false;
                missing = true;
                continue;
            };
            referenced.insert(hex.to_string());
            let logical = media_type
                .rsplit('.')
                .next()
                .filter(|kind| !kind.is_empty());
            match blob_record(&blobs, hex, logical.unwrap_or("blob")) {
                Some(record) => model_files.push(record),
                None => missing = true,
            }
        }

        // Ollama itself only looks at this exact depth. Anything else still counts as references
        // above, as there's no telling what it is, but isn't a model.
        let Some(name) = display_name(&parts) else {
            continue;
        };
        let config = manifest
            .config
            .as_ref()
            .and_then(|layer| digest_hex(&layer.digest))
            .and_then(|hex| blob_record(&blobs, hex, "config"))
            .and_then(|file| read_capped(&file.path, CONFIG_CAP))
            .and_then(|bytes| serde_json::from_slice::<ConfigBlob>(&bytes).ok());
        let used = manifest
            .layers
            .iter()
            .filter(|layer| layer.media_type == MODEL_LAYER)
            .filter_map(|layer| digest_hex(&layer.digest))
            .filter_map(|hex| usage.ollama_blobs.get(hex).copied())
            .max();
        if !model_files.is_empty() || missing {
            out.models.push(RawModel {
                name,
                runtime: RuntimeKind::Ollama,
                detected: describe(&blobs, &manifest, config.as_ref()),
                root: root.to_path_buf(),
                files: model_files,
                used,
                incomplete: missing,
            });
        }
    }

    let Ok(entries) = fs::read_dir(&blobs) else {
        return out;
    };
    let mut complete_blobs: Vec<(String, FileRec)> = Vec::new();
    let mut partials: HashMap<String, Vec<FileRec>> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = name_of(&path);
        match classify_blob(&name) {
            BlobName::Complete(hex) => {
                if let Some(record) = FileRec::stat(&path, name.clone()) {
                    complete_blobs.push((hex.to_string(), record));
                }
            }
            BlobName::Partial(hex) => {
                if let Some(record) = FileRec::stat(&path, name.clone()) {
                    partials.entry(hex.to_string()).or_default().push(record);
                }
            }
            BlobName::Other => {}
        }
    }

    let mut pull_in_progress = false;
    for (hex, records) in partials {
        let newest_write = newest(records.iter().map(|record| record.modified));
        if !older_than(newest_write, ctx.now, ABANDONED_MIN_DAYS) {
            pull_in_progress = true;
            continue;
        }
        out.leftovers.push(Leftover {
            kind: LeftoverKind::Partial,
            runtime: RuntimeKind::Ollama,
            key: format!("ollama:{hex}"),
            label: format!("sha256-{}…", &hex[..12]),
            paths: records.iter().map(|record| record.path.clone()).collect(),
            records,
            newest: newest_write,
        });
    }

    if complete && !pull_in_progress {
        let orphans: Vec<FileRec> = complete_blobs
            .into_iter()
            .filter(|(hex, record)| {
                !referenced.contains(hex)
                    && older_than(record.modified, ctx.now, ABANDONED_MIN_DAYS)
            })
            .map(|(_, record)| record)
            .collect();
        if !orphans.is_empty() {
            out.leftovers.push(Leftover {
                kind: LeftoverKind::OllamaOrphan,
                runtime: RuntimeKind::Ollama,
                key: format!("ollama:{}", root.display()),
                label: "Ollama".to_string(),
                paths: orphans.iter().map(|record| record.path.clone()).collect(),
                newest: newest(orphans.iter().map(|record| record.modified)),
                records: orphans,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{sha, Fixture};

    fn scan_fixture(fx: &Fixture) -> StoreScan {
        let ctx = fx.ctx();
        scan(
            &fx.home.join(".ollama/models"),
            &ctx,
            &UsageRecords::default(),
        )
    }

    #[test]
    fn finds_tagged_models_with_their_real_names() {
        let fx = Fixture::new();
        fx.ollama_model(
            "registry.ollama.ai/library/llama3/latest",
            &[("model", b"pesos")],
        );
        fx.ollama_model("registry.ollama.ai/fer/mio/v1", &[("model", b"otros")]);
        fx.ollama_model("hf.co/org/repo/Q4_K_M", &[("model", b"mas")]);
        let mut names: Vec<_> = scan_fixture(&fx)
            .models
            .into_iter()
            .map(|m| m.name)
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["fer/mio:v1", "hf.co/org/repo:Q4_K_M", "llama3:latest"]
        );
    }

    #[test]
    fn digests_are_taken_from_the_blob_names() {
        let fx = Fixture::new();
        fx.ollama_model(
            "registry.ollama.ai/library/m/latest",
            &[("model", b"pesos")],
        );
        let models = scan_fixture(&fx).models;
        assert!(models[0]
            .files
            .iter()
            .any(|f| f.sha256.as_deref() == Some(sha(b"pesos").as_str())));
    }

    #[test]
    fn the_config_blob_counts_as_referenced() {
        let fx = Fixture::new();
        fx.ollama_model(
            "registry.ollama.ai/library/m/latest",
            &[("model", b"pesos")],
        );
        fx.age_tree(".ollama/models", 30);
        assert!(scan_fixture(&fx).leftovers.is_empty());
    }

    #[test]
    fn unreferenced_blobs_are_orphans_only_once_old_enough() {
        let fx = Fixture::new();
        fx.ollama_model(
            "registry.ollama.ai/library/m/latest",
            &[("model", b"pesos")],
        );
        let orphan = fx.ollama_blob(b"nadie me usa");
        assert!(
            scan_fixture(&fx).leftovers.is_empty(),
            "a pull may be writing its manifest last"
        );
        fx.age_tree(".ollama/models", 30);
        let leftovers = scan_fixture(&fx).leftovers;
        assert_eq!(leftovers.len(), 1);
        assert_eq!(leftovers[0].kind, LeftoverKind::OllamaOrphan);
        assert_eq!(leftovers[0].paths, vec![orphan]);
    }

    #[test]
    fn one_unreadable_manifest_means_nothing_is_called_an_orphan() {
        let fx = Fixture::new();
        fx.ollama_model(
            "registry.ollama.ai/library/good/latest",
            &[("model", b"pesos")],
        );
        fx.ollama_blob(b"quizas referenciado por el manifiesto roto");
        fx.write(
            ".ollama/models/manifests/registry.ollama.ai/library/bad/latest",
            b"{not json",
        );
        fx.age_tree(".ollama/models", 30);
        let found = scan_fixture(&fx);
        assert_eq!(found.models.len(), 1, "the good model is still listed");
        assert!(found.leftovers.is_empty());
    }

    #[test]
    fn a_finder_droplet_is_not_a_broken_manifest() {
        let fx = Fixture::new();
        fx.ollama_model(
            "registry.ollama.ai/library/m/latest",
            &[("model", b"pesos")],
        );
        fx.write(
            ".ollama/models/manifests/registry.ollama.ai/library/.DS_Store",
            b"\0\0Bud1",
        );
        fx.ollama_blob(b"huerfano");
        fx.age_tree(".ollama/models", 30);
        assert_eq!(scan_fixture(&fx).leftovers.len(), 1);
    }

    #[test]
    fn a_missing_manifests_folder_tells_us_nothing_about_orphans() {
        let fx = Fixture::new();
        fx.ollama_blob(b"sin manifiestos");
        fx.age_tree(".ollama/models", 30);
        assert!(scan_fixture(&fx).leftovers.is_empty());
    }

    #[test]
    fn crafted_digests_never_become_paths() {
        let fx = Fixture::new();
        let secret = fx.write("secreto.gguf", b"datos");
        let manifest = format!(
            r#"{{"schemaVersion":2,"config":{{"digest":"sha256:{}"}},
                "layers":[{{"mediaType":"{MODEL_LAYER}","digest":"sha256:../../../secreto.gguf"}}]}}"#,
            "0".repeat(64)
        );
        fx.write(
            ".ollama/models/manifests/registry.ollama.ai/library/evil/latest",
            manifest.as_bytes(),
        );
        let found = scan_fixture(&fx);
        assert!(found.models[0].files.iter().all(|file| file.path != secret));
        assert!(found.models[0].incomplete);
    }

    #[test]
    fn missing_blobs_mark_the_model_incomplete_rather_than_failing() {
        let fx = Fixture::new();
        fx.ollama_model(
            "registry.ollama.ai/library/m/latest",
            &[("model", b"pesos")],
        );
        fs::remove_file(
            fx.home
                .join(".ollama/models/blobs")
                .join(format!("sha256-{}", sha(b"pesos"))),
        )
        .unwrap();
        let models = scan_fixture(&fx).models;
        assert!(models[0].incomplete);
    }

    #[test]
    fn partials_are_abandoned_after_a_week_and_block_orphan_detection_until_then() {
        let fx = Fixture::new();
        let hex = sha(b"descargando");
        let data = fx.write(
            &format!(".ollama/models/blobs/sha256-{hex}-partial"),
            &[1u8; 5000],
        );
        let part = fx.write(
            &format!(".ollama/models/blobs/sha256-{hex}-partial-0"),
            b"{}",
        );
        fx.ollama_model(
            "registry.ollama.ai/library/m/latest",
            &[("model", b"pesos")],
        );
        let orphan = fx.ollama_blob(b"capa terminada de la misma descarga");
        fx.set_age(&orphan, 30);

        let found = scan_fixture(&fx);
        assert!(found.leftovers.is_empty(), "the pull is still going");

        fx.age_tree(".ollama/models", 8);
        let found = scan_fixture(&fx);
        let partial = found
            .leftovers
            .iter()
            .find(|leftover| leftover.kind == LeftoverKind::Partial)
            .unwrap();
        let mut paths = partial.paths.clone();
        paths.sort();
        assert_eq!(paths, vec![data, part]);
        assert!(found
            .leftovers
            .iter()
            .any(|leftover| leftover.kind == LeftoverKind::OllamaOrphan));
    }

    #[test]
    fn one_recent_part_keeps_the_whole_partial_out() {
        let fx = Fixture::new();
        let hex = sha(b"x");
        let data = fx.write(
            &format!(".ollama/models/blobs/sha256-{hex}-partial"),
            &[1u8; 100],
        );
        fx.write(
            &format!(".ollama/models/blobs/sha256-{hex}-partial-0"),
            b"{}",
        );
        fx.set_age(&data, 30);
        assert!(scan_fixture(&fx).leftovers.is_empty());
    }

    #[test]
    fn reads_metadata_from_the_gguf_layer_and_falls_back_to_its_config() {
        let fx = Fixture::new();
        let gguf = {
            let mut builder = crate::gguf::tests::Builder::default();
            builder
                .string("general.architecture", "qwen3")
                .u32("general.file_type", 15)
                .u32("qwen3.context_length", 32768)
                .tensor("t", &[7]);
            builder.build(3)
        };
        fx.ollama_model("registry.ollama.ai/library/a/latest", &[("model", &gguf)]);
        fx.ollama_model(
            "registry.ollama.ai/library/b/latest",
            &[("model", b"no es gguf")],
        );
        let mut models = scan_fixture(&fx).models;
        models.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(models[0].detected.format, ModelFormat::Gguf);
        assert_eq!(models[0].detected.architecture.as_deref(), Some("qwen3"));
        assert_eq!(models[0].detected.context_length, Some(32768));
        assert_eq!(models[1].detected.quantization.as_deref(), Some("Q4_0"));
        assert_eq!(models[1].detected.parameter_count, Some(7_200_000_000));
    }

    #[test]
    fn the_log_record_of_its_weights_is_the_models_usage() {
        let fx = Fixture::new();
        fx.ollama_model(
            "registry.ollama.ai/library/m/latest",
            &[("model", b"pesos")],
        );
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let usage = UsageRecords {
            ollama_blobs: HashMap::from([(sha(b"pesos"), when)]),
            ..UsageRecords::default()
        };
        let ctx = fx.ctx();
        let found = scan(&fx.home.join(".ollama/models"), &ctx, &usage);
        assert_eq!(found.models[0].used, Some(when));
    }
}

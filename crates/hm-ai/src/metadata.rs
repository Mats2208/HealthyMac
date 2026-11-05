//! Works out what a model is from its files, reading only small files and headers.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;

use crate::files::{read_capped, FileRec};
use crate::gguf;
use crate::types::ModelFormat;

const CONFIG_CAP: u64 = 8 * 1024 * 1024;
const INDEX_CAP: u64 = 64 * 1024 * 1024;
const SAFETENSORS_HEADER_CAP: u64 = 64 * 1024 * 1024;
const MAX_GGUF_VARIANTS_READ: usize = 8;

#[derive(Clone, Debug, Default)]
pub(crate) struct Detected {
    pub format: ModelFormat,
    pub quantization: Option<String>,
    pub architecture: Option<String>,
    pub parameter_count: Option<u64>,
    pub context_length: Option<u64>,
}

pub(crate) fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

fn base_name(logical: &str) -> &str {
    logical.rsplit('/').next().unwrap_or(logical)
}

pub(crate) fn is_gguf(logical: &str) -> bool {
    extension(logical) == "gguf"
}

/// The vision projector that accompanies a GGUF model. Not a variant of it.
pub(crate) fn is_projector(logical: &str) -> bool {
    let name = base_name(logical).to_ascii_lowercase();
    name.starts_with("mmproj") || name.contains("-mmproj")
}

/// Whether a file holds weights rather than configuration or a tokenizer.
pub(crate) fn is_weight(logical: &str, len: u64) -> bool {
    let name = base_name(logical);
    if logical
        .split('/')
        .any(|part| part.ends_with(".mlmodelc") || part.ends_with(".mlpackage"))
    {
        return true;
    }
    match extension(name).as_str() {
        "gguf" | "safetensors" | "sft" | "npz" | "pt" | "pth" | "ckpt" | "onnx" | "onnx_data" => {
            true
        }
        // Many things are called `.bin`. Below this it's a tokenizer or a config, not weights.
        "bin" => len >= 1024 * 1024,
        _ => false,
    }
}

/// Splits `x-00001-of-00003` into `x` and 3.
fn split_shard_suffix(stem: &str) -> Option<(&str, u32)> {
    // "-NNNNN-of-NNNNN" is exactly 15 bytes. `get` refuses to cut through a multibyte character.
    let at = stem.len().checked_sub(15).filter(|at| *at > 0)?;
    let (base, tail) = (stem.get(..at)?, stem.get(at..)?);
    let bytes = tail.as_bytes();
    let digits = |range: &[u8]| range.iter().all(u8::is_ascii_digit);
    let looks_right = bytes[0] == b'-'
        && &bytes[6..10] == b"-of-"
        && digits(&bytes[1..6])
        && digits(&bytes[10..15]);
    if !looks_right {
        return None;
    }
    Some((base, tail.get(10..)?.parse().ok()?))
}

/// The part of a GGUF name shared by all shards of a split model: `x-00001-of-00003.gguf` is `x`.
pub(crate) fn gguf_variant(logical: &str) -> String {
    let stem = logical.strip_suffix(".gguf").unwrap_or(logical);
    split_shard_suffix(stem)
        .map_or(stem, |(base, _)| base)
        .to_string()
}

/// Whether a model's own files say that some of it isn't here: an index naming shards that aren't
/// present, or a split GGUF with fewer shards than its names announce. Partial downloads of big
/// models leave exactly this, with everything else looking fine.
pub(crate) fn missing_parts(files: &[FileRec]) -> bool {
    let present: HashSet<&str> = files.iter().map(|file| file.logical.as_str()).collect();

    for index in files
        .iter()
        .filter(|file| file.logical.ends_with(".safetensors.index.json"))
    {
        let Some(value) = read_capped(&index.path, INDEX_CAP)
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        else {
            continue;
        };
        let folder = index.logical.rsplit_once('/').map(|(folder, _)| folder);
        let shards = value.get("weight_map").and_then(|map| map.as_object());
        for shard in shards
            .into_iter()
            .flat_map(|map| map.values())
            .filter_map(|shard| shard.as_str())
        {
            let found = match folder {
                Some(folder) => present.contains(format!("{folder}/{shard}").as_str()),
                None => present.contains(shard),
            };
            if !found {
                return true;
            }
        }
    }

    let mut shards: HashMap<&str, (u32, u32)> = HashMap::new();
    for file in files.iter().filter(|file| is_gguf(&file.logical)) {
        let stem = file.logical.strip_suffix(".gguf").unwrap_or(&file.logical);
        if let Some((base, total)) = split_shard_suffix(stem) {
            let entry = shards.entry(base).or_insert((0, total));
            entry.0 += 1;
        }
    }
    shards.values().any(|(found, total)| found < total)
}

#[derive(Deserialize)]
struct SafetensorsHeader {
    #[serde(rename = "__metadata__", default)]
    metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Whether a `.safetensors` says it was written by MLX. `mlx_lm` records it in the header.
fn safetensors_says_mlx(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut length = [0u8; 8];
    if file.read_exact(&mut length).is_err() {
        return false;
    }
    let length = u64::from_le_bytes(length);
    if length > SAFETENSORS_HEADER_CAP {
        return false;
    }
    let mut header = vec![0u8; length as usize];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    serde_json::from_slice::<SafetensorsHeader>(&header)
        .ok()
        .and_then(|header| header.metadata)
        .and_then(|metadata| {
            metadata
                .get("format")?
                .as_str()
                .map(|format| format == "mlx")
        })
        .unwrap_or(false)
}

fn config_context_length(config: &serde_json::Value) -> Option<u64> {
    const KEYS: [&str; 4] = [
        "max_position_embeddings",
        "n_positions",
        "max_sequence_length",
        "n_ctx",
    ];
    let read = |value: &serde_json::Value| KEYS.iter().find_map(|key| value.get(key)?.as_u64());
    read(config).or_else(|| config.get("text_config").and_then(read))
}

/// MLX records `quantization: {bits, group_size}` at the top level of `config.json`.
fn config_mlx_bits(config: &serde_json::Value) -> Option<u64> {
    config.get("quantization")?.get("bits")?.as_u64()
}

/// Reads conventions like `-4bit`, `-bf16` from a model's name.
fn quantization_from_model_name(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    for token in lower.split(['-', '_', '/', '.']) {
        if matches!(token, "bf16" | "fp16" | "f16" | "f32" | "fp32") {
            return Some(token.to_string());
        }
        if let Some(bits) = token.strip_suffix("bit") {
            if !bits.is_empty() && bits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Some(token.to_string());
            }
        }
    }
    None
}

fn format_of(file: &FileRec, is_mlx: bool, is_ctranslate2: bool) -> Option<ModelFormat> {
    if !is_weight(&file.logical, file.len) {
        return None;
    }
    if file
        .logical
        .split('/')
        .any(|part| part.ends_with(".mlmodelc") || part.ends_with(".mlpackage"))
    {
        return Some(ModelFormat::CoreMl);
    }
    let name = base_name(&file.logical);
    Some(match extension(name).as_str() {
        "gguf" => ModelFormat::Gguf,
        "npz" => {
            if name.contains(".mlx.") || name == "weights.npz" || is_mlx {
                ModelFormat::Mlx
            } else {
                ModelFormat::Unknown
            }
        }
        "safetensors" | "sft" => {
            if is_mlx {
                ModelFormat::Mlx
            } else {
                ModelFormat::Safetensors
            }
        }
        "onnx" | "onnx_data" => ModelFormat::Onnx,
        "bin" if name.starts_with("ggml-") => ModelFormat::Ggml,
        "bin" if is_ctranslate2 && name == "model.bin" => ModelFormat::CTranslate2,
        _ => ModelFormat::PyTorch,
    })
}

/// Fills in everything knowable from `files`. `logical` names matter here, not the paths: a Hugging
/// Face blob is named by its hash. `assume_mlx` is for runtimes that only ever run MLX.
pub(crate) fn detect(model_name: &str, files: &[FileRec], assume_mlx: bool) -> Detected {
    let config: Option<serde_json::Value> = files
        .iter()
        .find(|file| file.logical == "config.json")
        .and_then(|file| read_capped(&file.path, CONFIG_CAP))
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());

    let name_says_mlx = model_name.to_ascii_lowercase().contains("mlx");
    let is_mlx = assume_mlx
        || name_says_mlx
        || config
            .as_ref()
            .is_some_and(|config| config_mlx_bits(config).is_some())
        || files
            .iter()
            .find(|file| extension(&file.logical) == "safetensors")
            .is_some_and(|file| safetensors_says_mlx(&file.path));
    let is_ctranslate2 = files.iter().any(|file| {
        let name = base_name(&file.logical);
        name.starts_with("vocabulary.") || name.starts_with("shared_vocabulary.")
    });

    // A repository can ship several formats. What it mostly is, is what it's called.
    let mut totals: Vec<(ModelFormat, u64)> = Vec::new();
    for file in files {
        if let Some(format) = format_of(file, is_mlx, is_ctranslate2) {
            match totals.iter_mut().find(|(known, _)| *known == format) {
                Some((_, total)) => *total += file.len,
                None => totals.push((format, file.len)),
            }
        }
    }
    let format = totals
        .iter()
        .max_by_key(|(_, total)| *total)
        .map(|(format, _)| *format)
        .unwrap_or_default();

    let mut detected = Detected {
        format,
        ..Detected::default()
    };

    if let Some(config) = &config {
        detected.architecture = config
            .get("model_type")
            .and_then(|kind| kind.as_str())
            .map(str::to_string);
        detected.context_length = config_context_length(config);
        if format == ModelFormat::Mlx {
            detected.quantization = config_mlx_bits(config).map(|bits| format!("{bits}bit"));
        }
    }
    if format == ModelFormat::Mlx && detected.quantization.is_none() {
        detected.quantization = quantization_from_model_name(model_name);
    }
    if let Some(total) = files
        .iter()
        .find(|file| file.logical == "model.safetensors.index.json")
        .and_then(|file| read_capped(&file.path, CONFIG_CAP))
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|index| index.get("metadata")?.get("total_parameters")?.as_u64())
    {
        detected.parameter_count = Some(total);
    }
    if format == ModelFormat::Gguf {
        apply_gguf(files, &mut detected);
    }
    detected
}

fn apply_gguf(files: &[FileRec], detected: &mut Detected) {
    let mut ggufs: Vec<&FileRec> = files
        .iter()
        .filter(|file| is_gguf(&file.logical) && !is_projector(&file.logical))
        .collect();
    ggufs.sort_by(|a, b| a.logical.cmp(&b.logical));

    let mut variants: Vec<(String, Vec<&FileRec>)> = Vec::new();
    for file in ggufs {
        let key = gguf_variant(&file.logical);
        match variants.iter_mut().find(|(known, _)| *known == key) {
            Some((_, shards)) => shards.push(file),
            None => variants.push((key, vec![file])),
        }
    }

    let mut quantizations = BTreeSet::new();
    for (index, (_, shards)) in variants.iter().take(MAX_GGUF_VARIANTS_READ).enumerate() {
        let Some(first) = shards.first() else {
            continue;
        };
        let Ok(info) = gguf::read_info(&first.path) else {
            continue;
        };
        let quantization = info
            .quantization
            .clone()
            .or_else(|| gguf::quantization_from_name(&first.logical));
        quantizations.extend(quantization);
        if index == 0 {
            detected.architecture = info.architecture.or(detected.architecture.take());
            detected.context_length = info.context_length.or(detected.context_length);
            let split = info.split_count.is_some_and(|count| count > 1);
            detected.parameter_count = if split {
                // Each shard only knows about its own tensors.
                shards
                    .iter()
                    .map(|shard| gguf::read_info(&shard.path).ok()?.parameter_count)
                    .sum::<Option<u64>>()
            } else {
                info.parameter_count
            };
        }
    }
    if !quantizations.is_empty() {
        detected.quantization = Some(quantizations.into_iter().collect::<Vec<_>>().join(", "));
    }
}

/// Ollama's `config.model_type`: `"7.2B"` or `"300M"`.
pub(crate) fn parse_size_label(label: &str) -> Option<u64> {
    let label = label.trim();
    let (at, unit) = label.char_indices().last()?;
    let scale = match unit {
        'T' | 't' => 1e12,
        'B' | 'b' => 1e9,
        'M' | 'm' => 1e6,
        'K' | 'k' => 1e3,
        _ => return None,
    };
    let value: f64 = label[..at].parse().ok()?;
    (value.is_finite() && value > 0.0).then_some((value * scale) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gguf::tests::Builder;
    use std::fs;

    fn file(dir: &Path, logical: &str, bytes: &[u8]) -> FileRec {
        let path = dir.join(logical.replace('/', "_"));
        fs::write(&path, bytes).unwrap();
        FileRec::stat(&path, logical).unwrap()
    }

    fn sparse(dir: &Path, logical: &str, len: u64) -> FileRec {
        let path = dir.join(logical.replace('/', "_"));
        File::create(&path).unwrap().set_len(len).unwrap();
        FileRec::stat(&path, logical).unwrap()
    }

    fn safetensors(metadata: &str) -> Vec<u8> {
        let header = format!(
            r#"{{"__metadata__":{metadata},"t":{{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}}}"#
        );
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend(header.as_bytes());
        bytes.extend([0u8; 4]);
        bytes
    }

    #[test]
    fn gguf_comes_from_the_logical_name_even_when_the_blob_is_a_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = Builder::default();
        builder
            .string("general.architecture", "qwen3")
            .u32("general.file_type", 15)
            .u32("qwen3.context_length", 4096)
            .tensor("t", &[10]);
        let path = dir.path().join("8eeb52dfb3bb");
        fs::write(&path, builder.build(3)).unwrap();
        let record = FileRec::stat(&path, "model-Q4_K_M.gguf").unwrap();
        let detected = detect("org/model", &[record], false);
        assert_eq!(detected.format, ModelFormat::Gguf);
        assert_eq!(detected.architecture.as_deref(), Some("qwen3"));
        assert_eq!(detected.quantization.as_deref(), Some("Q4_K_M"));
        assert_eq!(detected.context_length, Some(4096));
        assert_eq!(detected.parameter_count, Some(10));
    }

    #[test]
    fn several_gguf_quantizations_are_all_named_and_shards_summed() {
        let dir = tempfile::tempdir().unwrap();
        let shard = |ftype: u32, dims: u64| {
            let mut builder = Builder::default();
            builder
                .string("general.architecture", "x")
                .u32("general.file_type", ftype)
                .u32("split.count", 2)
                .tensor("t", &[dims]);
            builder.build(3)
        };
        let files = vec![
            file(dir.path(), "m-Q8_0-00001-of-00002.gguf", &shard(7, 100)),
            file(dir.path(), "m-Q8_0-00002-of-00002.gguf", &shard(7, 50)),
            file(dir.path(), "m-Q4_K_M.gguf", &{
                let mut builder = Builder::default();
                builder.u32("general.file_type", 15).tensor("t", &[5]);
                builder.build(3)
            }),
            file(dir.path(), "mmproj-model-f16.gguf", &{
                let mut builder = Builder::default();
                builder.u32("general.file_type", 1);
                builder.build(3)
            }),
        ];
        let detected = detect("org/m-GGUF", &files, false);
        assert_eq!(detected.quantization.as_deref(), Some("Q4_K_M, Q8_0"));
    }

    #[test]
    fn a_split_model_sums_every_shard() {
        let dir = tempfile::tempdir().unwrap();
        let shard = |dims: u64| {
            let mut builder = Builder::default();
            builder.u32("split.count", 2).tensor("t", &[dims]);
            builder.build(3)
        };
        let files = vec![
            file(dir.path(), "m-00001-of-00002.gguf", &shard(100)),
            file(dir.path(), "m-00002-of-00002.gguf", &shard(50)),
        ];
        assert_eq!(detect("org/m", &files, false).parameter_count, Some(150));
    }

    #[test]
    fn corrupt_gguf_keeps_the_format_and_leaves_the_rest_empty() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file(dir.path(), "model.gguf", b"GGUF trash")];
        let detected = detect("org/m", &files, false);
        assert_eq!(detected.format, ModelFormat::Gguf);
        assert_eq!(detected.parameter_count, None);
    }

    #[test]
    fn mlx_is_recognised_by_its_header_config_name_or_file_naming() {
        let dir = tempfile::tempdir().unwrap();
        let header = vec![file(
            dir.path(),
            "model.safetensors",
            &safetensors(r#"{"format":"mlx"}"#),
        )];
        assert_eq!(detect("org/plain", &header, false).format, ModelFormat::Mlx);

        let plain = vec![file(
            dir.path(),
            "plain.safetensors",
            &safetensors(r#"{"format":"pt"}"#),
        )];
        assert_eq!(
            detect("org/plain", &plain, false).format,
            ModelFormat::Safetensors
        );
        assert_eq!(detect("org/plain", &plain, true).format, ModelFormat::Mlx);
        assert_eq!(
            detect("mlx-community/plain", &plain, false).format,
            ModelFormat::Mlx
        );

        let npz = vec![sparse(
            dir.path(),
            "splitted_model/model.layers.0.mlx.npz",
            2048,
        )];
        assert_eq!(detect("org/x", &npz, false).format, ModelFormat::Mlx);
    }

    #[test]
    fn reads_mlx_quantization_and_nested_context_from_config() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            file(
                dir.path(),
                "config.json",
                br#"{"model_type":"qwen3_5","quantization":{"bits":4,"group_size":64},
                     "text_config":{"max_position_embeddings":262144}}"#,
            ),
            file(
                dir.path(),
                "model.safetensors",
                &safetensors(r#"{"format":"mlx"}"#),
            ),
        ];
        let detected = detect("org/m", &files, false);
        assert_eq!(detected.quantization.as_deref(), Some("4bit"));
        assert_eq!(detected.architecture.as_deref(), Some("qwen3_5"));
        assert_eq!(detected.context_length, Some(262144));
    }

    #[test]
    fn quantization_falls_back_to_the_model_name_conventions() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file(dir.path(), "model.safetensors", &safetensors("{}"))];
        let quant = |name| detect(name, &files, true).quantization;
        assert_eq!(
            quant("mlx-community/Kokoro-82M-bf16").as_deref(),
            Some("bf16")
        );
        assert_eq!(quant("mlx-community/x-mlx-6bit").as_deref(), Some("6bit"));
        assert_eq!(quant("mlx-community/x"), None);
    }

    #[test]
    fn a_repository_is_called_what_most_of_its_bytes_are() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            sparse(dir.path(), "pytorch_model.bin", 2 << 30),
            sparse(dir.path(), "onnx/model.onnx", 1 << 30),
            file(dir.path(), "config.json", b"{}"),
        ];
        assert_eq!(detect("org/m", &files, false).format, ModelFormat::PyTorch);
    }

    #[test]
    fn faster_whisper_is_not_mistaken_for_pytorch_nor_ggml_for_either() {
        let dir = tempfile::tempdir().unwrap();
        let ct2 = vec![
            sparse(dir.path(), "model.bin", 100 << 20),
            file(dir.path(), "vocabulary.txt", b"a"),
        ];
        assert_eq!(
            detect("Systran/faster-whisper-base", &ct2, false).format,
            ModelFormat::CTranslate2
        );
        let ggml = vec![sparse(dir.path(), "ggml-base.bin", 100 << 20)];
        assert_eq!(detect("x", &ggml, false).format, ModelFormat::Ggml);
    }

    #[test]
    fn core_ml_lives_in_folders_of_anything() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![sparse(
            dir.path(),
            "m.mlmodelc/weights/weight.bin",
            100 << 20,
        )];
        assert_eq!(detect("x", &files, false).format, ModelFormat::CoreMl);
    }

    #[test]
    fn hostile_or_broken_metadata_is_harmless() {
        let dir = tempfile::tempdir().unwrap();
        let mut huge_header = u64::MAX.to_le_bytes().to_vec();
        huge_header.extend(b"{}");
        let files = vec![
            file(dir.path(), "config.json", b"{not json"),
            file(dir.path(), "model.safetensors.index.json", b"[]"),
            file(dir.path(), "model.safetensors", &huge_header),
        ];
        assert_eq!(
            detect("org/m", &files, false).format,
            ModelFormat::Safetensors
        );
        assert_eq!(detect("empty", &[], false).format, ModelFormat::Unknown);
    }

    #[test]
    fn an_index_naming_absent_shards_means_parts_are_missing() {
        let dir = tempfile::tempdir().unwrap();
        let index = file(
            dir.path(),
            "model.safetensors.index.json",
            br#"{"metadata":{},"weight_map":{"a":"model-00001-of-00002.safetensors","b":"model-00002-of-00002.safetensors"}}"#,
        );
        let first = file(dir.path(), "model-00001-of-00002.safetensors", b"x");
        let second = file(dir.path(), "model-00002-of-00002.safetensors", b"x");
        assert!(missing_parts(&[index.clone(), first.clone()]));
        assert!(!missing_parts(&[index, first, second]));
    }

    #[test]
    fn an_index_in_a_subfolder_is_read_relative_to_that_folder() {
        let dir = tempfile::tempdir().unwrap();
        let index = file(
            dir.path(),
            "unet/m.safetensors.index.json",
            br#"{"weight_map":{"a":"m-1.safetensors"}}"#,
        );
        let shard = file(dir.path(), "unet/m-1.safetensors", b"x");
        assert!(!missing_parts(&[index.clone(), shard]));
        let elsewhere = file(dir.path(), "m-1.safetensors", b"x");
        assert!(missing_parts(&[index, elsewhere]));
    }

    #[test]
    fn a_broken_index_proves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let junk = file(dir.path(), "model.safetensors.index.json", b"[1,2");
        let empty = file(dir.path(), "x.safetensors.index.json", b"{}");
        assert!(!missing_parts(&[junk, empty]));
    }

    #[test]
    fn a_split_gguf_must_have_all_the_shards_its_name_announces() {
        let dir = tempfile::tempdir().unwrap();
        let one = file(dir.path(), "m-Q8_0-00001-of-00002.gguf", b"x");
        let two = file(dir.path(), "m-Q8_0-00002-of-00002.gguf", b"x");
        let single = file(dir.path(), "m-Q4.gguf", b"x");
        assert!(missing_parts(std::slice::from_ref(&one)));
        assert!(!missing_parts(&[one, two, single]));
    }

    #[test]
    fn variants_ignore_shard_numbers() {
        assert_eq!(gguf_variant("m-Q4-00001-of-00003.gguf"), "m-Q4");
        assert_eq!(gguf_variant("m-Q4-00003-of-00003.gguf"), "m-Q4");
        assert_eq!(gguf_variant("m-Q4.gguf"), "m-Q4");
        assert_eq!(gguf_variant("é-00001-of-00003.gguf"), "é");
        // Multibyte text exactly where a shard suffix would start must not split a character.
        assert_eq!(gguf_variant("ééééééééééééééééé.gguf"), "ééééééééééééééééé");
    }

    #[test]
    fn size_labels() {
        assert_eq!(parse_size_label("7.2B"), Some(7_200_000_000));
        assert_eq!(parse_size_label("300M"), Some(300_000_000));
        assert_eq!(parse_size_label(""), None);
        assert_eq!(parse_size_label("muchos"), None);
        assert_eq!(parse_size_label("-3B"), None);
        // A multibyte last character must not panic while splitting.
        assert_eq!(parse_size_label("3é"), None);
    }
}

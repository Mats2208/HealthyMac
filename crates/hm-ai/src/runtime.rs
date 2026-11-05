//! Live status of local model runtimes.
//!
//! Read-only GETs to localhost with short timeouts, one thread per runtime, so a poll costs about as
//! long as the slowest single runtime and not their sum. Nothing here ever starts a runtime, and
//! what it talks to has to prove it's the expected program: port 8000 is the default of oMLX and of
//! half the web frameworks in the world.

use std::path::{Path, PathBuf};

use hm_core::ScanContext;
use serde_json::Value;

use crate::files::read_capped;
use crate::http;
use crate::locate::{is_real_home, Detail, Roots};
use crate::types::{LoadedModel, RuntimeKind, RuntimeStatus};

const OLLAMA_PORT: u16 = 11434;
const LM_STUDIO_PORT: u16 = 1234;
const LLAMA_SERVER_PORT: u16 = 8080;

/// Fast enough to poll every few seconds. Never starts a runtime.
pub fn status(ctx: &ScanContext) -> Vec<RuntimeStatus> {
    let roots = Roots::resolve(ctx, Detail::Light);
    let ports = Ports::resolve(&roots);
    status_with(ctx, &roots, &ports)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Ports {
    /// `None` when `OLLAMA_HOST` points at another machine, which this never contacts.
    pub ollama: Option<u16>,
    pub lm_studio: u16,
    pub omlx: u16,
    pub llama_cpp: u16,
}

impl Ports {
    pub fn resolve(roots: &Roots) -> Self {
        Self {
            ollama: ollama_port(roots.ollama_host.as_deref()),
            lm_studio: if roots.allowed_quietly(&roots.lm_home) {
                lm_studio_port(&roots.lm_home)
            } else {
                LM_STUDIO_PORT
            },
            omlx: roots.omlx.port,
            llama_cpp: LLAMA_SERVER_PORT,
        }
    }
}

fn ollama_port(host: Option<&str>) -> Option<u16> {
    let Some(host) = host.map(str::trim).filter(|host| !host.is_empty()) else {
        return Some(OLLAMA_PORT);
    };
    let (default_port, rest) = if let Some(rest) = host.strip_prefix("http://") {
        (80, rest)
    } else if let Some(rest) = host.strip_prefix("https://") {
        (443, rest)
    } else {
        (OLLAMA_PORT, host)
    };
    let rest = rest.split('/').next().unwrap_or_default();
    let (name, port) = match rest.rsplit_once(':') {
        Some((name, port)) if !name.ends_with(':') && !port.contains(']') => {
            (name, port.parse().ok()?)
        }
        _ => (rest, default_port),
    };
    matches!(
        name,
        "" | "127.0.0.1" | "localhost" | "0.0.0.0" | "[::1]" | "[::]"
    )
    .then_some(port)
}

fn lm_studio_port(lm_home: &Path) -> u16 {
    read_capped(
        &lm_home.join(".internal/http-server-config.json"),
        64 * 1024,
    )
    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    .and_then(|config| u16::try_from(config.get("port")?.as_u64()?).ok())
    .unwrap_or(LM_STUDIO_PORT)
}

#[derive(Debug, Default)]
struct Probe {
    running: bool,
    version: Option<String>,
    loaded: Vec<LoadedModel>,
    loaded_count: Option<u64>,
    loaded_memory: Option<u64>,
}

fn probe_ollama(port: u16) -> Probe {
    let version = http::get(port, "/api/version")
        .ok()
        .filter(|response| response.status == 200)
        .and_then(|response| response.json())
        .and_then(|json| json.get("version")?.as_str().map(str::to_string));
    let Some(version) = version else {
        return Probe::default();
    };
    let loaded: Vec<LoadedModel> = http::get(port, "/api/ps")
        .ok()
        .filter(|response| response.status == 200)
        .and_then(|response| response.json())
        .and_then(|json| {
            Some(
                json.get("models")?
                    .as_array()?
                    .iter()
                    .filter_map(|model| {
                        Some(LoadedModel {
                            name: model.get("name")?.as_str()?.to_string(),
                            memory_bytes: model.get("size").and_then(Value::as_u64),
                            gpu_memory_bytes: model.get("size_vram").and_then(Value::as_u64),
                        })
                    })
                    .collect(),
            )
        })
        .unwrap_or_default();
    Probe {
        running: true,
        version: Some(version),
        loaded_count: Some(loaded.len() as u64),
        loaded,
        loaded_memory: None,
    }
}

fn data_array(json: &Value) -> Option<&Vec<Value>> {
    json.get("data")?.as_array()
}

fn probe_lm_studio(port: u16) -> Probe {
    // No answer at all means there's no point asking again. That would only double the wait.
    let Ok(response) = http::get(port, "/api/v0/models") else {
        return Probe::default();
    };
    if matches!(response.status, 401 | 403) {
        // It's there and wants a token. Not one worth asking the user for just to show a list.
        return Probe {
            running: true,
            ..Probe::default()
        };
    }
    if response.status == 200 {
        if let Some(models) = response.json().as_ref().and_then(data_array) {
            let loaded: Vec<LoadedModel> = models
                .iter()
                .filter(|model| model.get("state").and_then(Value::as_str) == Some("loaded"))
                .filter_map(|model| {
                    Some(LoadedModel {
                        name: model.get("id")?.as_str()?.to_string(),
                        memory_bytes: None,
                        gpu_memory_bytes: None,
                    })
                })
                .collect();
            return Probe {
                running: true,
                loaded_count: Some(loaded.len() as u64),
                loaded,
                ..Probe::default()
            };
        }
    }
    // Older versions only have the OpenAI route. With just-in-time loading it lists everything
    // downloaded, so it can't say what's loaded and doesn't pretend to.
    let compatible = http::get(port, "/v1/models")
        .ok()
        .filter(|response| response.status == 200)
        .and_then(|response| response.json())
        .is_some_and(|json| data_array(&json).is_some());
    Probe {
        running: compatible,
        ..Probe::default()
    }
}

/// `/health` needs no API key, and carries `engine_pool`, which is what identifies oMLX. The names
/// of loaded models need the key if one is set, and this doesn't use the user's key: without it the
/// count and the memory total are what's known.
fn probe_omlx(port: u16) -> Probe {
    let Some(health) = http::get(port, "/health")
        .ok()
        .filter(|response| matches!(response.status, 200 | 503))
        .and_then(|response| response.json())
    else {
        return Probe::default();
    };
    let is_omlx = matches!(
        health.get("status").and_then(Value::as_str),
        Some("healthy" | "loading")
    ) && health.get("engine_pool").is_some();
    if !is_omlx {
        return Probe::default();
    }
    let pool = health.get("engine_pool");
    let loaded_count = pool.and_then(|pool| pool.get("loaded_count")?.as_u64());
    let loaded_memory = pool.and_then(|pool| pool.get("current_model_memory")?.as_u64());
    let mut loaded = Vec::new();
    if loaded_count.is_some_and(|count| count > 0) {
        let detail = http::get(port, "/v1/models/status")
            .ok()
            .filter(|response| response.status == 200)
            .and_then(|response| response.json());
        let models = detail
            .as_ref()
            .and_then(|json| json.get("models")?.as_array());
        for model in models.into_iter().flatten() {
            if model.get("loaded").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            if let Some(name) = model.get("id").and_then(Value::as_str) {
                loaded.push(LoadedModel {
                    name: name.to_string(),
                    memory_bytes: model.get("actual_size").and_then(Value::as_u64),
                    gpu_memory_bytes: None,
                });
            }
        }
    }
    Probe {
        running: true,
        version: None,
        loaded,
        loaded_count,
        loaded_memory,
    }
}

/// Port 8080 could be anything, so only a list saying it's from `llamacpp` counts.
fn probe_llama_cpp(port: u16) -> Probe {
    let json = http::get(port, "/v1/models")
        .ok()
        .filter(|response| response.status == 200)
        .and_then(|response| response.json());
    let Some(models) = json.as_ref().and_then(data_array) else {
        return Probe::default();
    };
    let is_llama =
        |model: &&Value| model.get("owned_by").and_then(Value::as_str) == Some("llamacpp");
    if !models.iter().any(|model| is_llama(&model)) {
        return Probe::default();
    }
    let loaded: Vec<LoadedModel> = models
        .iter()
        .filter(is_llama)
        .filter_map(|model| {
            Some(LoadedModel {
                name: model.get("id")?.as_str()?.to_string(),
                memory_bytes: None,
                gpu_memory_bytes: None,
            })
        })
        .collect();
    Probe {
        running: true,
        loaded_count: Some(loaded.len() as u64),
        loaded,
        ..Probe::default()
    }
}

/// Where an app is. System folders only count for the real user, so fixture homes stay hermetic.
pub(crate) fn app_path(ctx: &ScanContext, bundle: &str) -> Option<PathBuf> {
    let mine = ctx.home.join("Applications").join(bundle);
    if mine.exists() {
        return Some(mine);
    }
    let system = Path::new("/Applications").join(bundle);
    (is_real_home(ctx) && system.exists()).then_some(system)
}

fn tool_exists(ctx: &ScanContext, tool: &str) -> bool {
    if is_real_home(ctx) {
        ctx.which(tool).is_some()
    } else {
        [".local/bin", "bin"]
            .iter()
            .any(|dir| ctx.home.join(dir).join(tool).exists())
    }
}

/// The app's version, from an XML `Info.plist`. Reading a small file is fine for a poll, and spares
/// a dependency.
fn app_version(app: &Path) -> Option<String> {
    let bytes = read_capped(&app.join("Contents/Info.plist"), 1024 * 1024)?;
    let text = String::from_utf8_lossy(&bytes);
    let after = text.split("<key>CFBundleShortVersionString</key>").nth(1)?;
    let start = after.find("<string>")? + "<string>".len();
    let rest = after.get(start..)?;
    rest.get(..rest.find("</string>")?).map(str::to_string)
}

fn row(
    kind: RuntimeKind,
    installed: bool,
    storage: Option<PathBuf>,
    port: Option<u16>,
    probe: Probe,
) -> RuntimeStatus {
    RuntimeStatus {
        kind,
        installed,
        running: probe.running,
        endpoint: port
            .filter(|_| probe.running)
            .map(|port| format!("http://127.0.0.1:{port}")),
        version: probe.version,
        loaded: probe.loaded,
        storage_path: storage,
        storage_bytes: 0,
        loaded_count: probe.loaded_count,
        loaded_memory_bytes: probe.loaded_memory,
    }
}

pub(crate) fn status_with(ctx: &ScanContext, roots: &Roots, ports: &Ports) -> Vec<RuntimeStatus> {
    let (ollama, lm_studio, omlx, llama_cpp) = std::thread::scope(|scope| {
        let ollama = scope.spawn(|| ports.ollama.map(probe_ollama).unwrap_or_default());
        let lm_studio = scope.spawn(|| probe_lm_studio(ports.lm_studio));
        let omlx = scope.spawn(|| probe_omlx(ports.omlx));
        let llama_cpp = scope.spawn(|| probe_llama_cpp(ports.llama_cpp));
        (
            ollama.join().unwrap_or_default(),
            lm_studio.join().unwrap_or_default(),
            omlx.join().unwrap_or_default(),
            llama_cpp.join().unwrap_or_default(),
        )
    });

    let lm_app = app_path(ctx, "LM Studio.app");
    let omlx_app = app_path(ctx, "oMLX.app");
    let ollama_app = app_path(ctx, "Ollama.app");

    let mut lm_studio = lm_studio;
    if lm_studio.version.is_none() {
        lm_studio.version = lm_app.as_deref().and_then(app_version);
    }
    let mut omlx = omlx;
    if omlx.version.is_none() {
        omlx.version = omlx_app.as_deref().and_then(app_version);
    }

    let mut rows = vec![
        row(
            RuntimeKind::Ollama,
            ollama_app.is_some() || tool_exists(ctx, "ollama"),
            roots.ollama.first().cloned(),
            ports.ollama,
            ollama,
        ),
        row(
            RuntimeKind::LmStudio,
            lm_app.is_some()
                || (roots.allowed_quietly(&roots.lm_home)
                    && roots.lm_home.join("bin/lms").exists())
                || tool_exists(ctx, "lms"),
            roots.lm_models.first().cloned(),
            Some(ports.lm_studio),
            lm_studio,
        ),
        row(
            RuntimeKind::OMlx,
            omlx_app.is_some()
                || (roots.allowed_quietly(&roots.omlx.base)
                    && roots.omlx.base.join("bin/omlx").exists())
                || tool_exists(ctx, "omlx"),
            roots.omlx.model_dirs.first().cloned(),
            Some(ports.omlx),
            omlx,
        ),
        row(
            RuntimeKind::LlamaCpp,
            tool_exists(ctx, "llama-server") || tool_exists(ctx, "llama-cli"),
            Some(roots.llama_cache.clone()),
            Some(ports.llama_cpp),
            llama_cpp,
        ),
    ];

    let others: [(RuntimeKind, bool, Option<PathBuf>); 7] = [
        (
            RuntimeKind::HuggingFace,
            tool_exists(ctx, "hf") || tool_exists(ctx, "huggingface-cli"),
            Some(roots.hub.clone()),
        ),
        (
            RuntimeKind::Whisper,
            tool_exists(ctx, "whisper") || tool_exists(ctx, "whisper-cli"),
            roots.whisper.first().cloned(),
        ),
        (
            RuntimeKind::Jan,
            app_path(ctx, "Jan.app").is_some(),
            roots.jan.first().cloned(),
        ),
        (
            RuntimeKind::Gpt4All,
            app_path(ctx, "GPT4All.app").is_some() || ctx.home.join("gpt4all/bin").exists(),
            Some(roots.gpt4all.clone()),
        ),
        (
            RuntimeKind::ComfyUi,
            app_path(ctx, "ComfyUI.app").is_some(),
            roots.comfy.first().cloned(),
        ),
        (
            RuntimeKind::Automatic1111,
            ctx.home.join("stable-diffusion-webui/webui.sh").exists(),
            roots.a1111.first().cloned(),
        ),
        (
            RuntimeKind::DrawThings,
            app_path(ctx, "Draw Things.app").is_some(),
            roots.draw_things.clone(),
        ),
    ];
    rows.extend(
        others
            .into_iter()
            .filter(|(_, installed, _)| *installed)
            .map(|(kind, installed, storage)| {
                row(kind, installed, storage, None, Probe::default())
            }),
    );
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::tests::{closed_port, http_response, FakeServer};
    use crate::test_support::Fixture;
    use std::time::{Duration, Instant};

    fn roots(fx: &Fixture) -> Roots {
        Roots::resolve_with(&fx.home, false, Detail::Light, &|_| None)
    }

    fn all_closed() -> Ports {
        Ports {
            ollama: Some(closed_port()),
            lm_studio: closed_port(),
            omlx: closed_port(),
            llama_cpp: closed_port(),
        }
    }

    fn find(rows: &[RuntimeStatus], kind: RuntimeKind) -> &RuntimeStatus {
        rows.iter().find(|row| row.kind == kind).unwrap()
    }

    #[test]
    fn nothing_running_is_reported_quickly_with_storage_left_at_zero() {
        let fx = Fixture::new();
        let ctx = fx.ctx();
        let started = Instant::now();
        let rows = status_with(&ctx, &roots(&fx), &all_closed());
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(rows
            .iter()
            .all(|row| !row.running && row.endpoint.is_none()));
        assert!(rows.iter().all(|row| row.storage_bytes == 0));
        assert!(!find(&rows, RuntimeKind::Ollama).installed);
    }

    #[test]
    fn hung_servers_do_not_add_up() {
        // All four accept and then say nothing. Run in parallel, this costs one timeout, not four.
        let listeners: Vec<_> = (0..4)
            .map(|_| std::net::TcpListener::bind("127.0.0.1:0").unwrap())
            .collect();
        let port = |index: usize| listeners[index].local_addr().unwrap().port();
        let ports = Ports {
            ollama: Some(port(0)),
            lm_studio: port(1),
            omlx: port(2),
            llama_cpp: port(3),
        };
        let fx = Fixture::new();
        let ctx = fx.ctx();
        let started = Instant::now();
        let rows = status_with(&ctx, &roots(&fx), &ports);
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );
        assert!(rows.iter().all(|row| !row.running));
    }

    #[test]
    fn apps_and_cli_tools_count_as_installed() {
        let fx = Fixture::new();
        fx.write("Applications/Ollama.app/Contents/Info.plist", b"");
        fx.write(
            "Applications/LM Studio.app/Contents/Info.plist",
            b"<plist><dict><key>CFBundleShortVersionString</key>\n<string>0.4.23+1</string></dict></plist>",
        );
        fx.write(".omlx/bin/omlx", b"#!/bin/sh");
        fx.write(".local/bin/llama-server", b"");
        fx.write("Applications/Draw Things.app/Contents/Info.plist", b"");
        let ctx = fx.ctx();
        let rows = status_with(&ctx, &roots(&fx), &all_closed());
        assert!(find(&rows, RuntimeKind::Ollama).installed);
        assert_eq!(
            find(&rows, RuntimeKind::LmStudio).version.as_deref(),
            Some("0.4.23+1")
        );
        assert!(find(&rows, RuntimeKind::OMlx).installed);
        assert!(find(&rows, RuntimeKind::LlamaCpp).installed);
        assert!(find(&rows, RuntimeKind::DrawThings).installed);
        assert!(!rows.iter().any(|row| row.kind == RuntimeKind::Jan));
    }

    #[test]
    fn ollama_reports_version_and_loaded_memory() {
        let server = FakeServer::json(&[
            ("/api/version", 200, r#"{"version":"0.12.3"}"#),
            (
                "/api/ps",
                200,
                r#"{"models":[{"name":"llama3:latest","model":"llama3:latest","size":6000,"size_vram":5000},{"bad":1}]}"#,
            ),
        ]);
        let probe = probe_ollama(server.port);
        assert!(probe.running);
        assert_eq!(probe.version.as_deref(), Some("0.12.3"));
        assert_eq!(probe.loaded.len(), 1);
        assert_eq!(probe.loaded[0].name, "llama3:latest");
        assert_eq!(probe.loaded[0].memory_bytes, Some(6000));
        assert_eq!(probe.loaded[0].gpu_memory_bytes, Some(5000));
    }

    #[test]
    fn something_else_on_the_ollama_port_is_not_ollama() {
        let server = FakeServer::json(&[("/api/version", 200, r#"{"build":"otro"}"#)]);
        assert!(!probe_ollama(server.port).running);
        let html = FakeServer::raw(&[("/api/version", http_response(200, "<html>hola</html>"))]);
        assert!(!probe_ollama(html.port).running);
    }

    #[test]
    fn ollama_still_counts_as_running_if_only_its_list_fails() {
        let server = FakeServer::json(&[
            ("/api/version", 200, r#"{"version":"1"}"#),
            ("/api/ps", 500, "{}"),
        ]);
        let probe = probe_ollama(server.port);
        assert!(probe.running);
        assert!(probe.loaded.is_empty());
    }

    #[test]
    fn lm_studio_native_api_says_what_is_loaded() {
        let server = FakeServer::json(&[(
            "/api/v0/models",
            200,
            r#"{"data":[{"id":"qwen3.8-27b-mlx","state":"loaded"},{"id":"otro","state":"not-loaded"}]}"#,
        )]);
        let probe = probe_lm_studio(server.port);
        assert!(probe.running);
        assert_eq!(probe.loaded.len(), 1);
        assert_eq!(probe.loaded[0].name, "qwen3.8-27b-mlx");
    }

    #[test]
    fn lm_studio_falls_back_to_the_openai_route_without_claiming_anything_is_loaded() {
        let server = FakeServer::json(&[(
            "/v1/models",
            200,
            r#"{"data":[{"id":"todos-los-descargados"}]}"#,
        )]);
        let probe = probe_lm_studio(server.port);
        assert!(probe.running);
        assert!(probe.loaded.is_empty());
        assert_eq!(probe.loaded_count, None);
    }

    #[test]
    fn lm_studio_behind_a_token_is_running_with_nothing_more_known() {
        let server = FakeServer::json(&[("/api/v0/models", 401, r#"{"error":"token"}"#)]);
        let probe = probe_lm_studio(server.port);
        assert!(probe.running);
        assert_eq!(probe.loaded_count, None);
    }

    #[test]
    fn omlx_is_identified_by_its_health_shape_not_by_its_port() {
        let foreign = FakeServer::json(&[("/health", 200, r#"{"status":"healthy"}"#)]);
        assert!(
            !probe_omlx(foreign.port).running,
            "another dev server with a /health"
        );

        let django = FakeServer::raw(&[]);
        assert!(!probe_omlx(django.port).running);

        let loading =
            FakeServer::json(&[("/health", 503, r#"{"status":"loading","engine_pool":null}"#)]);
        assert!(probe_omlx(loading.port).running);
    }

    #[test]
    fn omlx_behind_an_api_key_still_reports_count_and_memory() {
        let server = FakeServer::json(&[
            (
                "/health",
                200,
                r#"{"status":"healthy","engine_pool":{"loaded_count":1,"current_model_memory":17000000000}}"#,
            ),
            ("/v1/models/status", 401, r#"{"detail":"API key required"}"#),
        ]);
        let probe = probe_omlx(server.port);
        assert!(probe.running);
        assert_eq!(probe.loaded_count, Some(1));
        assert_eq!(probe.loaded_memory, Some(17_000_000_000));
        assert!(probe.loaded.is_empty());
    }

    #[test]
    fn omlx_without_a_key_names_what_is_loaded() {
        let server = FakeServer::json(&[
            (
                "/health",
                200,
                r#"{"status":"healthy","engine_pool":{"loaded_count":1}}"#,
            ),
            (
                "/v1/models/status",
                200,
                r#"{"models":[{"id":"Qwen3.8","loaded":true,"actual_size":17},{"id":"otro","loaded":false}]}"#,
            ),
        ]);
        let probe = probe_omlx(server.port);
        assert_eq!(probe.loaded.len(), 1);
        assert_eq!(probe.loaded[0].name, "Qwen3.8");
        assert_eq!(probe.loaded[0].memory_bytes, Some(17));
    }

    #[test]
    fn llama_server_only_counts_if_it_says_it_is_llama() {
        let llama = FakeServer::json(&[(
            "/v1/models",
            200,
            r#"{"data":[{"id":"m.gguf","owned_by":"llamacpp"}]}"#,
        )]);
        let probe = probe_llama_cpp(llama.port);
        assert!(probe.running);
        assert_eq!(probe.loaded[0].name, "m.gguf");
        let other = FakeServer::json(&[(
            "/v1/models",
            200,
            r#"{"data":[{"id":"gpt","owned_by":"openai"}]}"#,
        )]);
        assert!(!probe_llama_cpp(other.port).running);
    }

    #[test]
    fn a_running_runtime_is_reported_with_its_endpoint() {
        let server = FakeServer::json(&[
            ("/api/version", 200, r#"{"version":"1"}"#),
            ("/api/ps", 200, r#"{"models":[]}"#),
        ]);
        let fx = Fixture::new();
        let ctx = fx.ctx();
        let ports = Ports {
            ollama: Some(server.port),
            ..all_closed()
        };
        let rows = status_with(&ctx, &roots(&fx), &ports);
        let row = find(&rows, RuntimeKind::Ollama);
        assert!(row.running);
        assert_eq!(
            row.endpoint,
            Some(format!("http://127.0.0.1:{}", server.port))
        );
    }

    #[test]
    fn ollama_host_is_honoured_only_for_this_machine() {
        assert_eq!(ollama_port(None), Some(11434));
        assert_eq!(ollama_port(Some("127.0.0.1:11500")), Some(11500));
        assert_eq!(ollama_port(Some("0.0.0.0")), Some(11434));
        assert_eq!(ollama_port(Some(":9000")), Some(9000));
        assert_eq!(ollama_port(Some("http://localhost:9000/")), Some(9000));
        assert_eq!(ollama_port(Some("http://localhost")), Some(80));
        assert_eq!(ollama_port(Some("[::1]:9000")), Some(9000));
        assert_eq!(ollama_port(Some("gpu.example.com:11434")), None);
        assert_eq!(ollama_port(Some("10.0.0.2")), None);
    }

    #[test]
    fn lm_studio_port_comes_from_its_config() {
        let fx = Fixture::new();
        fx.write(
            ".lmstudio/.internal/http-server-config.json",
            br#"{"port":4321}"#,
        );
        assert_eq!(lm_studio_port(&fx.home.join(".lmstudio")), 4321);
        fx.write(".lmstudio/.internal/http-server-config.json", b"{bad");
        assert_eq!(lm_studio_port(&fx.home.join(".lmstudio")), LM_STUDIO_PORT);
    }
}

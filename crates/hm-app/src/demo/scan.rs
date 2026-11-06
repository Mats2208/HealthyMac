//! A pretend scan: the same events as the real one, on a similar schedule.

use std::time::Duration;

use hm_core::{Action, Area, CancelToken, Finding, Recovery, Risk, ScanEvent};

use super::{days_ago, hours_ago, HOME, MB};
use crate::state::ScannerMeta;

const SCANNERS: &[(&str, &str, &str, Area)] = &[
    (
        "sys.user_caches",
        "Cachés de apps",
        "Lo que las apps guardan para ir más rápido. Lo regeneran solas.",
        Area::System,
    ),
    (
        "sys.logs",
        "Registros",
        "Diarios que ya nadie va a leer.",
        Area::System,
    ),
    (
        "sys.updates",
        "Actualizaciones descargadas",
        "Instaladores que ya hicieron su trabajo.",
        Area::System,
    ),
    (
        "sys.trash",
        "Papelera",
        "Lo que ya tiraste y sigue ocupando espacio.",
        Area::System,
    ),
    (
        "sys.time_machine",
        "Instantáneas locales",
        "Copias de Time Machine guardadas en el propio disco.",
        Area::System,
    ),
    (
        "dev.xcode.derived_data",
        "Xcode: DerivedData",
        "Compilaciones intermedias. Xcode las rehace.",
        Area::Developer,
    ),
    (
        "dev.xcode.device_support",
        "Xcode: soporte de dispositivos",
        "Símbolos de versiones de iOS que ya no usas.",
        Area::Developer,
    ),
    (
        "dev.xcode.simulators",
        "Simuladores",
        "Runtimes y dispositivos simulados que no se usan.",
        Area::Developer,
    ),
    (
        "dev.node_modules",
        "node_modules abandonados",
        "Dependencias de proyectos parados. npm install las trae de vuelta.",
        Area::Developer,
    ),
    (
        "dev.rust_target",
        "Compilaciones de Rust",
        "Carpetas target de proyectos parados.",
        Area::Developer,
    ),
    (
        "dev.pkg_caches",
        "Cachés de paquetes",
        "npm, pnpm, Cargo, pip y Homebrew.",
        Area::Developer,
    ),
    (
        "dev.docker",
        "Docker",
        "Imágenes sin usar y caché de build.",
        Area::Developer,
    ),
    (
        "ai.hf_cache",
        "Caché de Hugging Face",
        "Modelos descargados por scripts y que puede que no vuelvas a usar.",
        Area::LocalAi,
    ),
    (
        "ai.duplicates",
        "Modelos repetidos",
        "El mismo archivo guardado por runtimes distintos.",
        Area::LocalAi,
    ),
    (
        "ai.partial",
        "Descargas a medias",
        "Modelos que se quedaron sin terminar.",
        Area::LocalAi,
    ),
    (
        "ai.comfy",
        "Salidas de ComfyUI",
        "Imágenes generadas que se acumulan.",
        Area::LocalAi,
    ),
    (
        "apps.leftovers",
        "Restos de apps",
        "Lo que dejaron las apps que ya no están.",
        Area::Apps,
    ),
    (
        "files.large",
        "Archivos grandes",
        "Lo más voluminoso fuera de tus proyectos.",
        Area::Files,
    ),
    (
        "files.installers",
        "Instaladores en Descargas",
        "Los .dmg y .pkg que ya no hacen falta.",
        Area::Files,
    ),
    (
        "files.fda",
        "Acceso a tus carpetas",
        "Lo que veríamos con Acceso total al disco.",
        Area::Files,
    ),
];

pub fn meta(_stress: bool) -> Vec<ScannerMeta> {
    SCANNERS
        .iter()
        .map(|(id, name, description, area)| ScannerMeta {
            id,
            name,
            description,
            area: *area,
        })
        .collect()
}

fn cache(
    scanner: &str,
    area: Area,
    key: &str,
    title: &str,
    path: &str,
    size: u64,
    days: u64,
) -> Finding {
    Finding::new(scanner, area, key, title)
        .trash(format!("{HOME}/{path}"))
        .size(size)
        .items(size / 180_000 + 3)
        .last_used(days_ago(days))
        .risk(Risk::Safe)
        .recovery(Recovery::Automatic)
        .selected(true)
}

fn findings_for(scanner: &'static str) -> Vec<Finding> {
    match scanner {
        "sys.user_caches" => vec![
            cache(scanner, Area::System, "spotify", "Spotify", "Library/Caches/com.spotify.client", 7_400 * MB, 1)
                .explanation("Música guardada para reproducir sin esperas. Spotify vuelve a descargar lo que necesite."),
            cache(scanner, Area::System, "chrome", "Google Chrome", "Library/Caches/Google/Chrome", 3_100 * MB, 0)
                .explanation("Páginas e imágenes guardadas. Algunas webs cargarán un poco más lentas la primera vez.")
                .blocked_by(&["Google Chrome"]),
            cache(scanner, Area::System, "slack", "Slack", "Library/Caches/com.tinyspeck.slackmacgap", 920 * MB, 2)
                .explanation("Archivos e imágenes de tus canales. Slack los recupera cuando hacen falta."),
            cache(scanner, Area::System, "figma", "Figma", "Library/Caches/com.figma.Desktop", 640 * MB, 9)
                .explanation("Copias locales de archivos abiertos. Sigue todo en la nube."),
            cache(scanner, Area::System, "webkit", "WebKit", "Library/Caches/com.apple.WebKit.Networking", 310 * MB, 3)
                .explanation("Caché de red de las apps que usan WebKit."),
        ],
        "sys.logs" => vec![
            cache(scanner, Area::System, "logs", "Registros de apps", "Library/Logs", 1_240 * MB, 1)
                .explanation("Registros de actividad de tus apps. Solo sirven para diagnosticar problemas pasados."),
            cache(scanner, Area::System, "crash", "Informes de fallos", "Library/Logs/DiagnosticReports", 180 * MB, 12)
                .explanation("Informes de cuando alguna app se cerró sola."),
        ],
        "sys.updates" => vec![
            Finding::new(scanner, Area::System, "macos", "Instalador de macOS Tahoe")
                .trash("/Applications/Instalar macOS Tahoe.app")
                .size(16_800 * MB)
                .items(1)
                .last_used(days_ago(61))
                .risk(Risk::Moderate)
                .recovery(Recovery::Redownload)
                .selected(true)
                .explanation("La actualización ya está instalada. Si algún día quieres otro USB de arranque, tendrás que volver a bajarlo."),
        ],
        "sys.trash" => vec![
            Finding::new(scanner, Area::System, "trash", "Papelera")
                .subtitle(format!("{HOME}/.Trash"))
                .size(5_200 * MB)
                .items(214)
                .last_used(days_ago(4))
                .risk(Risk::Review)
                .recovery(Recovery::None)
                .action(Action::EmptyTrash)
                .explanation("Vaciarla es definitivo: lo que haya dentro no se puede recuperar. Por eso no va marcada."),
        ],
        "sys.time_machine" => vec![
            Finding::new(scanner, Area::System, "tm", "Instantáneas locales de Time Machine")
                .subtitle("3 instantáneas de los últimos días")
                .size(21_000 * MB)
                .explanation("macOS las borra solo cuando necesita espacio, así que no hace falta tocarlas. Te lo contamos para que sepas qué ocupa ese hueco."),
        ],
        "dev.xcode.derived_data" => vec![
            cache(scanner, Area::Developer, "dd", "DerivedData", "Library/Developer/Xcode/DerivedData", 38_400 * MB, 2)
                .recovery(Recovery::Rebuild)
                .blocked_by(&["Xcode"])
                .explanation("Resultados intermedios de compilación. La próxima compilación tardará más, y ya."),
        ],
        "dev.xcode.device_support" => vec![
            cache(scanner, Area::Developer, "ios17", "iOS 17.5 (21F90)", "Library/Developer/Xcode/iOS DeviceSupport/iPhone15,3 17.5 (21F90)", 7_900 * MB, 240)
                .risk(Risk::Moderate).recovery(Recovery::Redownload)
                .explanation("Símbolos de depuración. Se vuelven a copiar cuando conectas un dispositivo con esa versión."),
            cache(scanner, Area::Developer, "ios18", "iOS 18.1 (22B83)", "Library/Developer/Xcode/iOS DeviceSupport/iPhone15,3 18.1 (22B83)", 6_300 * MB, 120)
                .risk(Risk::Moderate).recovery(Recovery::Redownload)
                .explanation("Símbolos de depuración. Se vuelven a copiar cuando conectas un dispositivo con esa versión."),
        ],
        "dev.xcode.simulators" => vec![
            Finding::new(scanner, Area::Developer, "unavailable", "Simuladores no disponibles")
                .subtitle("xcrun simctl delete unavailable")
                .size(4_300 * MB)
                .items(11)
                .risk(Risk::Safe)
                .recovery(Recovery::Redownload)
                .selected(true)
                .action(Action::Command { program: "xcrun".into(), args: vec!["simctl".into(), "delete".into(), "unavailable".into()] })
                .explanation("Dispositivos de versiones de iOS que ya no tienes. Los limpia el propio Xcode."),
            cache(scanner, Area::Developer, "runtime16", "Runtime iOS 16.4", "Library/Developer/CoreSimulator/Profiles/Runtimes/iOS 16.4.simruntime", 7_100 * MB, 400)
                .risk(Risk::Moderate).recovery(Recovery::Redownload).selected(false)
                .explanation("Un simulador completo de iOS 16. Se vuelve a descargar desde Xcode si te hace falta."),
        ],
        "dev.node_modules" => vec![
            cache(scanner, Area::Developer, "shop", "node_modules", "Coding/tienda-web/node_modules", 2_140 * MB, 94)
                .group("tienda-web").risk(Risk::Moderate).recovery(Recovery::Redownload)
                .explanation("Se recupera con npm install. Lleva 94 días sin moverse."),
            cache(scanner, Area::Developer, "shop-admin", "node_modules", "Coding/tienda-web/admin/node_modules", 1_380 * MB, 94)
                .group("tienda-web").risk(Risk::Moderate).recovery(Recovery::Redownload)
                .explanation("Se recupera con npm install."),
            cache(scanner, Area::Developer, "blog", "node_modules", "Coding/blog-astro/node_modules", 245 * MB, 41)
                .shared_bytes(365 * MB)
                .group("blog-astro").risk(Risk::Moderate).recovery(Recovery::Redownload)
                .explanation("Se recupera con pnpm install."),
            cache(scanner, Area::Developer, "scraper-venv", ".venv", "Coding/scraper-precios/.venv", 45 * MB, 180)
                .shared_bytes(916 * MB)
                .group("scraper-precios").risk(Risk::Moderate).recovery(Recovery::Redownload)
                .explanation("Entorno de Python creado con uv. Se recupera con uv sync."),
            cache(scanner, Area::Developer, "tesis", "node_modules", "Coding/viejo/tesis-visualizaciones/node_modules", 980 * MB, 420)
                .group("tesis-visualizaciones").risk(Risk::Moderate).recovery(Recovery::Redownload)
                .explanation("Se recupera con npm install. Lleva más de un año sin moverse."),
        ],
        "dev.rust_target" => vec![
            cache(scanner, Area::Developer, "healthy", "target", "Coding/motor-audio/target", 18_200 * MB, 33)
                .group("motor-audio").recovery(Recovery::Rebuild)
                .explanation("Salida de cargo. Vuelve con cargo build, aunque tardará."),
            cache(scanner, Area::Developer, "cli", "target", "Coding/herramientas/rsync-mejor/target", 4_100 * MB, 72)
                .group("rsync-mejor").recovery(Recovery::Rebuild)
                .explanation("Salida de cargo. Vuelve con cargo build."),
        ],
        "dev.pkg_caches" => vec![
            cache(scanner, Area::Developer, "npm", "Caché de npm", ".npm/_cacache", 5_800 * MB, 1)
                .recovery(Recovery::Redownload).risk(Risk::Safe)
                .explanation("Paquetes ya descargados. npm los vuelve a bajar cuando hagan falta."),
            cache(scanner, Area::Developer, "pnpm", "Almacén de pnpm", "Library/pnpm/store", 11_400 * MB, 1)
                .recovery(Recovery::Redownload).risk(Risk::Moderate).selected(false)
                .explanation("Todos tus proyectos enlazan a este almacén. Borrarlo obliga a reinstalar la próxima vez."),
            cache(scanner, Area::Developer, "cargo", "Registro de Cargo", ".cargo/registry", 3_700 * MB, 2)
                .recovery(Recovery::Redownload)
                .explanation("Crates descargados. Cargo los recupera al compilar."),
            Finding::new(scanner, Area::Developer, "brew", "Homebrew: descargas viejas")
                .subtitle("brew cleanup --prune=all")
                .size(2_900 * MB)
                .items(87)
                .risk(Risk::Safe)
                .recovery(Recovery::Redownload)
                .selected(true)
                .action(Action::Command { program: "brew".into(), args: vec!["cleanup".into(), "--prune=all".into()] })
                .explanation("Versiones antiguas e instaladores que Homebrew ya no necesita. Lo limpia Homebrew."),
        ],
        "dev.docker" => vec![
            Finding::new(scanner, Area::Developer, "build", "Caché de build de Docker")
                .subtitle("docker builder prune -f")
                .size(12_600 * MB)
                .risk(Risk::Moderate)
                .recovery(Recovery::Rebuild)
                .selected(true)
                .action(Action::Command { program: "docker".into(), args: vec!["builder".into(), "prune".into(), "-f".into()] })
                .explanation("Capas de builds anteriores. El siguiente docker build tardará más."),
            Finding::new(scanner, Area::Developer, "images", "Imágenes de Docker sin usar")
                .subtitle("docker image prune -a -f")
                .size(9_300 * MB)
                .items(14)
                .risk(Risk::Moderate)
                .recovery(Recovery::Redownload)
                .action(Action::Command { program: "docker".into(), args: vec!["image".into(), "prune".into(), "-a".into(), "-f".into()] })
                .explanation("Ningún contenedor las usa. Se vuelven a bajar con docker pull."),
        ],
        "ai.hf_cache" => vec![
            cache(scanner, Area::LocalAi, "sdxl", "stabilityai/stable-diffusion-xl-base-1.0", ".cache/huggingface/hub/models--stabilityai--stable-diffusion-xl-base-1.0", 13_900 * MB, 88)
                .risk(Risk::Moderate).recovery(Recovery::Redownload).selected(false)
                .explanation("Se volverá a descargar si algún script lo pide: son casi 14 GB."),
            cache(scanner, Area::LocalAi, "whisper", "openai/whisper-large-v3", ".cache/huggingface/hub/models--openai--whisper-large-v3", 6_200 * MB, 40)
                .risk(Risk::Moderate).recovery(Recovery::Redownload).selected(false)
                .explanation("Se volverá a descargar si algún script lo pide."),
        ],
        "ai.duplicates" => vec![
            Finding::new(scanner, Area::LocalAi, "dup-llama", "Deduplicar llama3.3:70b entre Ollama y LM Studio")
                .subtitle(format!("{HOME}/.lmstudio/models/lmstudio-community/Llama-3.3-70B-GGUF"))
                .size(42_100 * MB)
                .items(1)
                .risk(Risk::Safe)
                .recovery(Recovery::NothingLost)
                .selected(true)
                .action(Action::CloneDedupe {
                    keep: format!("{HOME}/.ollama/models/blobs/sha256-5e1f").into(),
                    duplicates: vec![format!("{HOME}/.lmstudio/models/lmstudio-community/Llama-3.3-70B-GGUF/model.gguf").into()],
                })
                .explanation("Dos copias exactas del mismo archivo. Pasan a ser clones APFS: ambas rutas siguen funcionando y no se pierde nada."),
        ],
        "ai.partial" => vec![
            cache(scanner, Area::LocalAi, "partial", "Descarga incompleta de gemma3:27b", ".ollama/models/blobs/sha256-a1c9-partial", 3_400 * MB, 19)
                .recovery(Recovery::Redownload)
                .explanation("Una descarga que no terminó. Ollama la reanuda o la empieza de nuevo."),
        ],
        "ai.comfy" => vec![
            Finding::new(scanner, Area::LocalAi, "outputs", "Imágenes generadas con ComfyUI")
                .trash(format!("{HOME}/ComfyUI/output"))
                .size(8_100 * MB)
                .items(3_412)
                .last_used(days_ago(26))
                .risk(Risk::Review)
                .recovery(Recovery::None)
                .explanation("Son imágenes tuyas: no se regeneran solas. Mira antes de quitarlas."),
        ],
        "apps.leftovers" => vec![
            Finding::new(scanner, Area::Apps, "sketch", "Restos de Sketch")
                .trash(format!("{HOME}/Library/Application Support/com.bohemiancoding.sketch3"))
                .size(2_300 * MB)
                .items(812)
                .last_used(days_ago(300))
                .risk(Risk::Moderate)
                .recovery(Recovery::None)
                .selected(true)
                .explanation("La app ya no está instalada. Si vuelves a instalarla, empezará de cero."),
            Finding::new(scanner, Area::Apps, "parallels", "Restos de Parallels")
                .trash(format!("{HOME}/Library/Parallels"))
                .size(6_700 * MB)
                .items(40)
                .last_used(days_ago(510))
                .risk(Risk::Review)
                .recovery(Recovery::None)
                .explanation("Puede contener máquinas virtuales tuyas. Mira qué hay antes de quitarlo."),
        ],
        "files.large" => vec![
            Finding::new(scanner, Area::Files, "video", "Grabación de pantalla - reunión.mov")
                .trash(format!("{HOME}/Movies/Grabación de pantalla - reunión.mov"))
                .size(24_800 * MB)
                .items(1)
                .last_used(days_ago(190))
                .risk(Risk::Review)
                .recovery(Recovery::None)
                .explanation("Es un archivo tuyo y no se regenera. Solo tú sabes si aún lo necesitas."),
            Finding::new(scanner, Area::Files, "iso", "ubuntu-24.04-desktop-arm64.iso")
                .trash(format!("{HOME}/Downloads/ubuntu-24.04-desktop-arm64.iso"))
                .size(6_100 * MB)
                .items(1)
                .last_used(days_ago(150))
                .risk(Risk::Review)
                .recovery(Recovery::Redownload)
                .explanation("Una imagen de disco que se puede volver a descargar."),
            Finding::new(scanner, Area::Files, "zip", "copia-fotos-2019.zip")
                .trash(format!("{HOME}/Desktop/Antiguo/Muy antiguo/Cosas de la mudanza/Copias de seguridad sin ordenar/copia-fotos-2019.zip"))
                .size(18_200 * MB)
                .items(1)
                .last_used(days_ago(800))
                .risk(Risk::Review)
                .recovery(Recovery::None)
                .explanation("Podría ser la única copia de esas fotos. Asegúrate antes de quitarla."),
        ],
        "files.installers" => vec![
            cache(scanner, Area::Files, "dmg1", "Docker.dmg", "Downloads/Docker.dmg", 1_400 * MB, 60)
                .risk(Risk::Moderate).recovery(Recovery::Redownload)
                .explanation("Un instalador que ya usaste."),
            cache(scanner, Area::Files, "dmg2", "Figma-126.1.dmg", "Downloads/Figma-126.1.dmg", 310 * MB, 88)
                .risk(Risk::Moderate).recovery(Recovery::Redownload)
                .explanation("Un instalador que ya usaste."),
        ],
        "files.fda" => vec![
            Finding::new(scanner, Area::Files, "fda", "Falta el Acceso total al disco")
                .subtitle("Documentos, Escritorio, Descargas y los datos de las apps")
                .explanation("Sin ese permiso, Documentos, Escritorio, Descargas y los datos de las apps se quedan fuera del escaneo. Puedes concederlo en Ajustes, y retirarlo cuando quieras.")
                .tag(hm_core::TAG_NEEDS_FULL_DISK_ACCESS),
        ],
        _ => Vec::new(),
    }
}

fn stress_findings() -> Vec<Finding> {
    (0..6_000u64)
        .map(|index| {
            Finding::new(
                "dev.node_modules",
                Area::Developer,
                format!("stress-{index}"),
                "node_modules",
            )
            .trash(format!(
                "{HOME}/Coding/monorepo/packages/paquete-{index}/node_modules"
            ))
            .group(format!("paquete-{:03}", index / 40))
            .size(1_000_000 + index * 137_000)
            .items(index * 3)
            .last_used(hours_ago(index))
            .risk(if index % 7 == 0 {
                Risk::Review
            } else {
                Risk::Moderate
            })
            .recovery(Recovery::Redownload)
            .selected(index % 7 != 0)
            .explanation(
                "Dato de prueba, para comprobar cómo se comporta la lista con miles de filas.",
            )
        })
        .collect()
}

pub fn run(
    cancel: CancelToken,
    stress: bool,
    on_event: impl Fn(ScanEvent),
    on_finish: impl FnOnce(Vec<Finding>),
) {
    let mut all = Vec::new();
    let mut pending: Vec<&'static str> = SCANNERS.iter().map(|(id, ..)| *id).collect();
    while !pending.is_empty() && !cancel.is_cancelled() {
        // A few at a time, like a real thread pool.
        let batch: Vec<&'static str> = pending.drain(..pending.len().min(4)).collect();
        for &id in &batch {
            on_event(ScanEvent::Started { scanner_id: id });
        }
        for &id in &batch {
            std::thread::sleep(Duration::from_millis(120));
            let mut findings = findings_for(id);
            if stress && id == "dev.node_modules" {
                findings.extend(stress_findings());
            }
            on_event(ScanEvent::Finished {
                scanner_id: id,
                findings: findings.clone(),
                elapsed: Duration::from_millis(120),
            });
            all.extend(findings);
        }
    }
    on_finish(all);
}

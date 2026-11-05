//! Package manager caches.
//!
//! The rule for this table is to list only what is really regenerable. Several tools keep things
//! that look like a cache next to things that are not, so entries name exact subfolders:
//!
//! - `~/.cargo/bin`, `~/go/bin`, `~/Library/pnpm/global`, `~/.pub-cache/bin` hold global installs.
//! - `~/Library/Caches/pypoetry/virtualenvs` holds the real virtualenvs of Poetry projects.
//! - `~/.m2/repository` can hold artifacts built with `mvn install` that exist nowhere else.
//! - `~/.gradle/gradle.properties` often holds signing credentials.

use anyhow::Result;
use hm_core::{Area, Finding, Recovery, Risk, ScanContext, Scanner};

use super::{table_findings, CacheSpec, Fallback, HasTool, Removal};
use crate::util;

const ID: &str = "dev.packages";

pub(super) const SPECS: &[CacheSpec] = &[
    // Node.
    CacheSpec {
        key: "npm",
        group: "Node",
        title: "Caché de npm",
        paths: &[".npm/_cacache"],
        removal: Removal::Command {
            program: "npm",
            args: &["cache", "clean", "--force"],
            fallback: Fallback::TrashContents,
        },
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Paquetes que npm ha descargado. Los vuelve a descargar cuando hacen falta. No afecta a tus proyectos ni a los paquetes instalados con -g.",
    },
    CacheSpec {
        key: "npx",
        group: "Node",
        title: "Paquetes temporales de npx",
        paths: &[".npm/_npx"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Paquetes descargados para ejecutarlos con npx. Se vuelven a descargar la próxima vez que los uses.",
    },
    CacheSpec {
        key: "npm_logs",
        group: "Node",
        title: "Registros de npm",
        paths: &[".npm/_logs"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Automatic,
        selected: true,
        blocked_by: &[],
        explanation: "Registros de ejecuciones anteriores de npm. Solo sirven para investigar errores pasados.",
    },
    CacheSpec {
        key: "pnpm_store",
        group: "Node",
        title: "Almacén de pnpm",
        paths: &["Library/pnpm/store", ".local/share/pnpm/store", ".pnpm-store"],
        removal: Removal::Command {
            program: "pnpm",
            args: &["store", "prune"],
            fallback: Fallback::TrashContents,
        },
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        // The size shown is the whole store, and `prune` frees only what no project references.
        selected: false,
        blocked_by: &[],
        explanation: "Almacén global de paquetes de pnpm. El tamaño mostrado es el de todo el almacén: «pnpm store prune» solo quita los paquetes que ya ningún proyecto usa, así que lo que libera suele ser bastante menos. Tus proyectos y los paquetes globales no se tocan.",
    },
    CacheSpec {
        key: "pnpm_cache",
        group: "Node",
        title: "Caché de metadatos de pnpm",
        paths: &["Library/Caches/pnpm"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Automatic,
        selected: true,
        blocked_by: &[],
        explanation: "Metadatos del registro y paquetes de «pnpm dlx». pnpm los regenera solo.",
    },
    // Yarn's own command is not used: Yarn 2+ refuses `cache clean` outside a project, and the
    // version installed cannot be told without running a shim that may download one.
    CacheSpec {
        key: "yarn",
        group: "Node",
        title: "Caché de Yarn",
        paths: &["Library/Caches/Yarn", ".yarn/berry/cache"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Paquetes que Yarn ha descargado. Los vuelve a descargar cuando hacen falta.",
    },
    // `bun pm cache rm` needs a package.json in the working directory, and the cleaner has none.
    CacheSpec {
        key: "bun",
        group: "Node",
        title: "Caché de Bun",
        paths: &[".bun/install/cache"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Paquetes que Bun ha descargado. Los vuelve a descargar cuando hacen falta. No toca los paquetes globales.",
    },
    CacheSpec {
        key: "bun_runtime",
        group: "Node",
        title: "Caché de transpilación de Bun",
        paths: &["Library/Caches/bun"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Automatic,
        selected: true,
        blocked_by: &[],
        explanation: "Archivos que Bun ya transpiló, guardados para arrancar más rápido. Los regenera solo.",
    },
    CacheSpec {
        key: "deno",
        group: "Node",
        title: "Caché de Deno",
        paths: &["Library/Caches/deno"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Módulos que Deno ha descargado y compilado. Los vuelve a descargar cuando hacen falta.",
    },
    // Rust. `~/.cargo/bin` holds `cargo install` binaries and is deliberately absent.
    CacheSpec {
        key: "cargo_registry",
        group: "Rust",
        title: "Registro de Cargo",
        paths: &[".cargo/registry/cache", ".cargo/registry/src", ".cargo/registry/index"],
        removal: Removal::Folders,
        risk: Risk::Moderate,
        recovery: Recovery::Redownload,
        selected: false,
        blocked_by: &["cargo"],
        explanation: "Crates descargados y descomprimidos. Cargo los vuelve a descargar en la próxima compilación que los necesite, así que hasta entonces no podrás compilar sin conexión. Tus binarios de «cargo install» no se tocan.",
    },
    CacheSpec {
        key: "cargo_git",
        group: "Rust",
        title: "Dependencias Git de Cargo",
        paths: &[".cargo/git/db", ".cargo/git/checkouts"],
        removal: Removal::Folders,
        risk: Risk::Moderate,
        recovery: Recovery::Redownload,
        selected: false,
        blocked_by: &["cargo"],
        explanation: "Repositorios clonados por Cargo para dependencias Git. Los vuelve a clonar cuando hacen falta.",
    },
    // Go. The module cache is written read-only on purpose, so only Go itself can remove it.
    CacheSpec {
        key: "go_modules",
        group: "Go",
        title: "Módulos de Go",
        paths: &["go/pkg/mod"],
        removal: Removal::Command {
            program: "go",
            args: &["clean", "-modcache"],
            fallback: Fallback::ReportOnly(
                "Go guarda estos archivos como solo lectura y solo se pueden quitar con «go clean -modcache», pero Go no está instalado.",
            ),
        },
        risk: Risk::Moderate,
        recovery: Recovery::Redownload,
        selected: false,
        blocked_by: &[],
        explanation: "Módulos que Go ha descargado. Los vuelve a descargar en la próxima compilación que los necesite. Tus herramientas de «go install» no se tocan.",
    },
    CacheSpec {
        key: "go_build",
        group: "Go",
        title: "Caché de compilación de Go",
        paths: &["Library/Caches/go-build"],
        removal: Removal::Command {
            program: "go",
            args: &["clean", "-cache"],
            fallback: Fallback::TrashContents,
        },
        risk: Risk::Safe,
        recovery: Recovery::Automatic,
        selected: true,
        blocked_by: &[],
        explanation: "Resultados de compilaciones anteriores de Go. Se regeneran solos: la siguiente compilación solo tardará algo más.",
    },
    // JVM.
    CacheSpec {
        key: "gradle_caches",
        group: "Gradle y Maven",
        title: "Cachés de Gradle",
        paths: &[".gradle/caches"],
        removal: Removal::Contents,
        risk: Risk::Moderate,
        recovery: Recovery::Redownload,
        selected: false,
        // A Gradle daemon is a `java` process, and it holds these files open.
        blocked_by: &["java"],
        explanation: "Dependencias descargadas y compilaciones en caché de Gradle. Vuelven en la siguiente compilación, que será bastante más lenta y necesitará conexión. Tu configuración de Gradle no se toca.",
    },
    CacheSpec {
        key: "gradle_wrapper",
        group: "Gradle y Maven",
        title: "Distribuciones de Gradle",
        paths: &[".gradle/wrapper/dists"],
        removal: Removal::Contents,
        risk: Risk::Moderate,
        recovery: Recovery::Redownload,
        selected: false,
        blocked_by: &["java"],
        explanation: "Versiones de Gradle descargadas por los wrappers de tus proyectos. Cada proyecto descarga la suya de nuevo cuando la necesita.",
    },
    CacheSpec {
        key: "gradle_daemon_logs",
        group: "Gradle y Maven",
        title: "Registros del daemon de Gradle",
        paths: &[".gradle/daemon"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Automatic,
        selected: true,
        blocked_by: &["java"],
        explanation: "Registros de los daemons de Gradle. Solo sirven para investigar errores pasados.",
    },
    // Not a pure cache: `mvn install` puts artifacts here that may exist nowhere else.
    CacheSpec {
        key: "maven",
        group: "Gradle y Maven",
        title: "Repositorio local de Maven",
        paths: &[".m2/repository"],
        removal: Removal::Contents,
        risk: Risk::Review,
        recovery: Recovery::Redownload,
        selected: false,
        blocked_by: &["java"],
        explanation: "Dependencias descargadas por Maven, que se pueden volver a descargar. Pero también guarda lo que hayas publicado con «mvn install», que puede no existir en ningún otro sitio. Revísalo antes de limpiarlo.",
    },
    // Python.
    CacheSpec {
        key: "pip",
        group: "Python",
        title: "Caché de pip",
        paths: &["Library/Caches/pip"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Paquetes que pip ha descargado. Los vuelve a descargar cuando hacen falta. No afecta a lo que ya está instalado.",
    },
    CacheSpec {
        key: "pipx",
        group: "Python",
        title: "Caché de pipx",
        paths: &["Library/Caches/pipx"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Entornos temporales de «pipx run». Las herramientas instaladas con pipx no están aquí.",
    },
    CacheSpec {
        key: "uv",
        group: "Python",
        title: "Caché de uv",
        paths: &[".cache/uv"],
        removal: Removal::Command {
            program: "uv",
            args: &["cache", "clean"],
            fallback: Fallback::TrashContents,
        },
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Paquetes que uv ha descargado. Tus entornos virtuales siguen funcionando, pero como comparten archivos con esta caché, lo que se libera puede ser menos de lo indicado. Tus Python y herramientas de uv no se tocan.",
    },
    CacheSpec {
        key: "poetry",
        group: "Python",
        title: "Caché de Poetry",
        paths: &["Library/Caches/pypoetry/cache", "Library/Caches/pypoetry/artifacts"],
        removal: Removal::Folders,
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Paquetes que Poetry ha descargado. Los vuelve a descargar cuando hacen falta. Los entornos virtuales de tus proyectos no se incluyen aquí.",
    },
    // Lives under Library/Caches but is not a cache: every Poetry project's environment is here.
    CacheSpec {
        key: "poetry_virtualenvs",
        group: "Python",
        title: "Entornos virtuales de Poetry",
        paths: &["Library/Caches/pypoetry/virtualenvs"],
        removal: Removal::Contents,
        risk: Risk::Moderate,
        recovery: Recovery::Redownload,
        selected: false,
        blocked_by: &[],
        explanation: "Los entornos de tus proyectos con Poetry, aunque estén en una carpeta de cachés. Cada proyecto dejará de funcionar hasta que vuelvas a ejecutar «poetry install».",
    },
    // Apple.
    CacheSpec {
        key: "cocoapods",
        group: "Swift y CocoaPods",
        title: "Caché de CocoaPods",
        paths: &["Library/Caches/CocoaPods"],
        removal: Removal::Command {
            program: "pod",
            args: &["cache", "clean", "--all"],
            fallback: Fallback::TrashContents,
        },
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Pods que CocoaPods ha descargado. Los vuelve a descargar en el próximo «pod install». Las carpetas Pods de tus proyectos no se tocan.",
    },
    CacheSpec {
        key: "cocoapods_repos",
        group: "Swift y CocoaPods",
        title: "Repositorios de especificaciones de CocoaPods",
        paths: &[".cocoapods/repos"],
        removal: Removal::Contents,
        risk: Risk::Moderate,
        recovery: Recovery::Redownload,
        selected: false,
        blocked_by: &[],
        explanation: "Índices de pods. Se vuelven a descargar en el próximo «pod install». Si añadiste repositorios privados, tendrás que añadirlos de nuevo.",
    },
    CacheSpec {
        key: "swiftpm",
        group: "Swift y CocoaPods",
        title: "Caché de Swift Package Manager",
        paths: &["Library/Caches/org.swift.swiftpm"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &["Xcode"],
        explanation: "Paquetes de Swift descargados. Se vuelven a descargar cuando hacen falta.",
    },
    // Others.
    CacheSpec {
        key: "homebrew",
        group: "Homebrew",
        title: "Caché de Homebrew",
        paths: &["Library/Caches/Homebrew"],
        removal: Removal::Command {
            program: "brew",
            args: &["cleanup", "-s"],
            fallback: Fallback::TrashContents,
        },
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Instaladores que Homebrew ha descargado. «brew cleanup -s» los quita y además borra versiones antiguas de lo que tengas instalado, lo que puede liberar más de lo indicado. Lo que usas sigue funcionando.",
    },
    CacheSpec {
        key: "composer",
        group: "Otros",
        title: "Caché de Composer",
        paths: &[".composer/cache", "Library/Caches/composer"],
        removal: Removal::Contents,
        risk: Risk::Safe,
        recovery: Recovery::Redownload,
        selected: true,
        blocked_by: &[],
        explanation: "Paquetes de PHP descargados. Se vuelven a descargar cuando hacen falta. Los paquetes globales no se tocan.",
    },
    CacheSpec {
        key: "pub",
        group: "Otros",
        title: "Paquetes de Dart y Flutter",
        paths: &[".pub-cache/hosted", ".pub-cache/hosted-hashes", ".pub-cache/_temp"],
        removal: Removal::Folders,
        risk: Risk::Moderate,
        recovery: Recovery::Redownload,
        selected: false,
        blocked_by: &[],
        explanation: "Paquetes descargados por pub. Vuelven con «flutter pub get» en cada proyecto, que no funcionará sin conexión hasta entonces. Las herramientas activadas con «pub global» no se tocan.",
    },
    CacheSpec {
        key: "nuget",
        group: "Otros",
        title: "Paquetes de NuGet",
        paths: &[".nuget/packages"],
        removal: Removal::Contents,
        risk: Risk::Moderate,
        recovery: Recovery::Redownload,
        selected: false,
        blocked_by: &[],
        explanation: "Paquetes de .NET descargados. Vuelven con «dotnet restore» en cada proyecto.",
    },
];

pub(super) struct PackageCacheScanner;

fn findings(ctx: &ScanContext, has_tool: HasTool) -> Vec<Finding> {
    util::finalize(ctx, table_findings(ctx, ID, SPECS, has_tool))
}

impl Scanner for PackageCacheScanner {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Cachés de paquetes"
    }

    fn description(&self) -> &'static str {
        "Lo que npm, pnpm, Cargo, Go, Gradle, uv, Homebrew y compañía guardan para no descargarlo otra vez."
    }

    fn area(&self) -> Area {
        Area::Developer
    }

    fn scan(&self, ctx: &ScanContext) -> Result<Vec<Finding>> {
        let has_tool = |name: &str| util::tool(ctx, name).is_some();
        Ok(findings(ctx, &has_tool))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Fixture;
    use hm_core::Action;

    fn with_all_tools(ctx: &ScanContext) -> Vec<Finding> {
        findings(ctx, &|_| true)
    }

    fn no_tools(ctx: &ScanContext) -> Vec<Finding> {
        findings(ctx, &|_| false)
    }

    fn key<'a>(found: &'a [Finding], key: &str) -> Option<&'a Finding> {
        found
            .iter()
            .find(|finding| finding.id == format!("{ID}:{key}"))
    }

    fn all_targets(found: &[Finding]) -> Vec<String> {
        found
            .iter()
            .flat_map(|finding| finding.target_paths())
            .map(|path| path.display().to_string())
            .collect()
    }

    #[test]
    fn global_installs_are_never_offered() {
        let fx = Fixture::new();
        for global in [
            ".cargo/bin/ripgrep",
            "go/bin/gopls",
            "Library/pnpm/global/5/node_modules/x/index.js",
            "Library/pnpm/bin/tool",
            ".pub-cache/bin/fvm",
            ".composer/vendor/bin/x",
            ".local/share/uv/python/cpython/bin/python",
            ".local/share/uv/tools/ruff/bin/ruff",
        ] {
            fx.file(global, b"x");
        }
        let found = no_tools(&fx.ctx());
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn cargo_lists_only_downloads_and_leaves_binaries_alone() {
        let fx = Fixture::new();
        fx.file(".cargo/bin/ripgrep", b"x");
        fx.file(".cargo/registry/cache/index/serde.crate", b"x");
        fx.file(".cargo/registry/src/index/serde/lib.rs", b"x");
        let found = no_tools(&fx.ctx());
        let targets = all_targets(&found);
        assert!(targets.iter().all(|path| !path.contains(".cargo/bin")));
        let registry = key(&found, "cargo_registry").unwrap();
        assert_eq!(registry.risk, Risk::Moderate);
        assert!(!registry.selected_by_default);
    }

    #[test]
    fn poetry_keeps_virtualenvs_out_of_its_cache_finding() {
        let fx = Fixture::new();
        fx.file("Library/Caches/pypoetry/cache/a", b"x");
        fx.file("Library/Caches/pypoetry/artifacts/b", b"x");
        fx.file(
            "Library/Caches/pypoetry/virtualenvs/proj-py3.12/pyvenv.cfg",
            b"x",
        );
        let found = no_tools(&fx.ctx());
        let cache = key(&found, "poetry").unwrap();
        assert!(cache.selected_by_default);
        for target in cache.target_paths() {
            assert!(!target.ends_with("virtualenvs"));
        }
        let envs = key(&found, "poetry_virtualenvs").unwrap();
        assert!(!envs.selected_by_default);
        assert_eq!(envs.risk, Risk::Moderate);
    }

    #[test]
    fn maven_is_for_review_because_it_can_hold_local_only_artifacts() {
        let fx = Fixture::new();
        fx.file(".m2/repository/com/acme/lib/1.0/lib.jar", b"x");
        fx.file(".m2/settings.xml", b"<secrets/>");
        let found = no_tools(&fx.ctx());
        let maven = key(&found, "maven").unwrap();
        assert_eq!(maven.risk, Risk::Review);
        assert!(!maven.selected_by_default);
        assert!(all_targets(&found)
            .iter()
            .all(|path| !path.ends_with("settings.xml")));
    }

    #[test]
    fn gradle_never_touches_its_properties_file() {
        let fx = Fixture::new();
        fx.file(".gradle/gradle.properties", b"signing.password=hunter2");
        fx.file(".gradle/caches/modules-2/a.jar", b"x");
        let found = no_tools(&fx.ctx());
        let caches = key(&found, "gradle_caches").unwrap();
        assert_eq!(caches.blocked_by, vec!["java".to_string()]);
        assert!(all_targets(&found)
            .iter()
            .all(|path| path.ends_with(".gradle/caches")));
    }

    #[test]
    fn go_modules_need_go_because_they_are_read_only() {
        let fx = Fixture::new();
        fx.file("go/pkg/mod/github.com/x/y@v1/y.go", b"x");
        let with = with_all_tools(&fx.ctx());
        assert_eq!(
            key(&with, "go_modules").unwrap().action,
            Action::Command {
                program: "go".to_string(),
                args: vec!["clean".to_string(), "-modcache".to_string()],
            }
        );
        let without = no_tools(&fx.ctx());
        let report = key(&without, "go_modules").unwrap();
        assert_eq!(report.action, Action::ReportOnly);
        assert!(report.explanation.contains("no está instalado"));
    }

    #[test]
    fn tools_with_their_own_cleanup_use_it() {
        let fx = Fixture::new();
        fx.file(".npm/_cacache/content-v2/x", b"x");
        fx.file("Library/pnpm/store/v11/files/00/x", b"x");
        fx.file(".cache/uv/wheels/x", b"x");
        fx.file("Library/Caches/Homebrew/downloads/x", b"x");
        let found = with_all_tools(&fx.ctx());
        for (name, program) in [
            ("npm", "npm"),
            ("pnpm_store", "pnpm"),
            ("uv", "uv"),
            ("homebrew", "brew"),
        ] {
            match &key(&found, name).unwrap().action {
                Action::Command { program: used, .. } => assert_eq!(used, program),
                other => panic!("{name}: {other:?}"),
            }
        }
    }

    #[test]
    fn missing_tools_fall_back_to_the_trash() {
        let fx = Fixture::new();
        fx.file(".npm/_cacache/content-v2/x", b"x");
        fx.file("Library/pnpm/store/v11/files/00/x", b"x");
        fx.file("Library/Caches/Homebrew/downloads/x", b"x");
        let found = no_tools(&fx.ctx());
        for name in ["npm", "pnpm_store", "homebrew"] {
            assert!(
                matches!(
                    key(&found, name).unwrap().action,
                    Action::TrashContents { .. }
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn the_pnpm_store_does_not_claim_to_free_its_whole_size() {
        let fx = Fixture::new();
        fx.file("Library/pnpm/store/v11/files/00/x", b"x");
        let found = with_all_tools(&fx.ctx());
        let store = key(&found, "pnpm_store").unwrap();
        assert!(!store.selected_by_default);
        assert!(store.explanation.contains("menos"));
    }

    #[test]
    fn every_trash_target_passes_the_cleaner_safety_policy() {
        let fx = Fixture::new();
        for file in [
            ".npm/_cacache/a",
            ".npm/_npx/a",
            ".cargo/registry/cache/a",
            ".cargo/git/db/a",
            ".gradle/caches/a",
            ".m2/repository/a",
            "Library/Caches/pip/a",
            ".cache/uv/a",
            "Library/Caches/Homebrew/a",
            ".bun/install/cache/a",
            ".pub-cache/hosted/a",
        ] {
            fx.file(file, b"x");
        }
        let ctx = fx.ctx();
        let found = no_tools(&ctx);
        assert!(found.len() >= 10);
        for finding in &found {
            assert_ne!(finding.action, Action::ReportOnly, "{}", finding.id);
        }
    }
}

//! Which apps exist on this Mac, and what is running. Shared by the app list, the cache scanner and
//! the leftovers scanner.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use hm_core::{exec, ScanContext};
use plist::Value;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::gate::name_of;
use crate::scanners::policy;

/// Deep enough for `/opt/homebrew/Caskroom/<cask>/<version>/Foo.app` and for vendor folders.
const APP_SEARCH_DEPTH: usize = 4;
const MDFIND_TIMEOUT: Duration = Duration::from_secs(5);

/// Places that hold bundles with a bundle id but no `.app`, such as audio plug-ins. Their vendors
/// keep preferences under `~/Library` just as apps do, so ignoring them would call those orphans.
const PLUGIN_ROOTS: &[&str] = &[
    "Library/Audio/Plug-Ins",
    "Library/Extensions",
    "Library/Frameworks",
    "Library/Input Methods",
    "Library/Internet Plug-Ins",
    "Library/PreferencePanes",
    "Library/QuickLook",
    "Library/Spotlight",
];

const PLUGIN_EXTENSIONS: &[&str] = &[
    "aaxplugin",
    "bundle",
    "component",
    "framework",
    "kext",
    "mdimporter",
    "plugin",
    "prefpane",
    "qlgenerator",
    "vst",
    "vst3",
];

/// Bundles nested inside an app, whose ids need not share the app's prefix.
const NESTED_BUNDLE_DIRS: &[&str] = &[
    "Contents/Applications",
    "Contents/Frameworks",
    "Contents/Helpers",
    "Contents/Library/LoginItems",
];

// ---- Reading one bundle -------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub(crate) struct BundleInfo {
    /// Symlinks resolved. `/Applications/Safari.app` really lives under `/System/Cryptexes`.
    pub resolved: PathBuf,
    /// The file name without `.app`, which is what Finder shows.
    pub name: String,
    pub bundle_id: Option<String>,
    pub version: Option<String>,
    pub executable: Option<String>,
    pub executable_path: Option<PathBuf>,
    pub icon_path: Option<PathBuf>,
    /// An `Info.plist` exists but could not be parsed, so we cannot say which app this is.
    pub unreadable_plist: bool,
}

struct Layout {
    plist: PathBuf,
    executables: PathBuf,
    resources: PathBuf,
}

fn locate_layout(bundle: &Path) -> Option<Layout> {
    let contents = bundle.join("Contents");
    let plist = contents.join("Info.plist");
    if plist.exists() {
        return Some(Layout {
            plist,
            executables: contents.join("MacOS"),
            resources: contents.join("Resources"),
        });
    }
    // iOS apps on Apple silicon: `Foo.app/WrappedBundle` links to `Wrapper/Foo.app`, which has a
    // flat layout. Looking only under `Contents` makes every one of them look unreadable.
    let mut wrapped = vec![bundle.join("WrappedBundle")];
    wrapped.extend(hm_core::fsutil::children(&bundle.join("Wrapper")));
    wrapped.into_iter().find_map(|inner| {
        let plist = inner.join("Info.plist");
        plist.exists().then(|| Layout {
            plist,
            executables: inner.clone(),
            resources: inner,
        })
    })
}

pub(crate) fn read_bundle(path: &Path) -> BundleInfo {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut info = BundleInfo {
        resolved: resolved.clone(),
        name: name.clone(),
        ..BundleInfo::default()
    };
    let Some(layout) = locate_layout(&resolved) else {
        return info;
    };
    let plist = match Value::from_file(&layout.plist) {
        Ok(plist) => plist,
        Err(_) => {
            info.unreadable_plist = true;
            return info;
        }
    };
    let Some(dict) = plist.as_dictionary() else {
        info.unreadable_plist = true;
        return info;
    };
    let text = |key: &str| {
        dict.get(key)
            .and_then(Value::as_string)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    info.bundle_id = text("CFBundleIdentifier");
    info.version = text("CFBundleShortVersionString").or_else(|| text("CFBundleVersion"));

    let executable = text("CFBundleExecutable").unwrap_or(name);
    let executable_path = layout.executables.join(&executable);
    info.executable_path = executable_path.exists().then_some(executable_path);
    info.executable = Some(executable);

    info.icon_path = text("CFBundleIconFile").and_then(|icon| {
        [
            layout.resources.join(&icon),
            layout.resources.join(format!("{icon}.icns")),
        ]
        .into_iter()
        .find(|candidate| candidate.is_file())
    });
    info
}

// ---- Finding bundles ----------------------------------------------------------------------------

#[derive(Debug, Default)]
pub(crate) struct BundleSearch {
    pub paths: Vec<PathBuf>,
    /// Every folder could be read. A partial list cannot justify calling anything an orphan.
    pub complete: bool,
}

/// `.app` bundles under `root`. Does not look inside them, and does not follow symlinks except for
/// a bundle itself, which is how `/Applications/Safari.app` is installed.
pub(crate) fn find_app_bundles(root: &Path) -> BundleSearch {
    let mut search = BundleSearch {
        paths: Vec::new(),
        complete: true,
    };
    collect_apps(root, APP_SEARCH_DEPTH, &mut search);
    search
}

fn collect_apps(dir: &Path, depth: usize, search: &mut BundleSearch) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                search.complete = false;
            }
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = name_of(&path);
        if name.starts_with('.') {
            continue;
        }
        if name.to_ascii_lowercase().ends_with(".app") {
            // `is_dir` follows symlinks, which is what we want for a bundle.
            if path.is_dir() {
                search.paths.push(path);
            }
            continue;
        }
        if depth > 0 && hm_core::fsutil::is_real_dir(&path) {
            collect_apps(&path, depth - 1, search);
        }
    }
}

// ---- What is running ----------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub(crate) struct RunningProcesses {
    /// Process names, as `pgrep -x` would match them.
    pub names: HashSet<String>,
    /// The outermost `.app` each process runs from.
    pub bundles: HashSet<PathBuf>,
}

impl RunningProcesses {
    pub fn current() -> Self {
        let mut running = Self::default();
        let Ok(mut system) = System::new() else {
            return running;
        };
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
        );
        for process in system.processes().values() {
            running
                .names
                .insert(process.name().to_string_lossy().into_owned());
            if let Some(bundle) = process.exe().and_then(outermost_app) {
                // Compared with resolved paths: Safari runs from a cryptex under another name.
                running
                    .bundles
                    .insert(fs::canonicalize(&bundle).unwrap_or(bundle));
            }
        }
        running
    }
}

/// `/Applications/Foo.app/Contents/Frameworks/Foo Helper.app/...` to `/Applications/Foo.app`.
pub(crate) fn outermost_app(path: &Path) -> Option<PathBuf> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if let Component::Normal(name) = component {
            if name
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".app")
            {
                return Some(current);
            }
        }
    }
    None
}

// ---- The index ----------------------------------------------------------------------------------

/// The app behind a cache, used for `blocked_by` and for naming it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CacheOwner {
    pub display: String,
    pub process: String,
}

/// Caches named after a vendor rather than a bundle id.
const NAMED_OWNERS: &[(&str, &str, &str)] = &[
    ("com.apple.safari", "Safari", "Safari"),
    ("bravesoftware", "Brave", "Brave Browser"),
    ("firefox", "Firefox", "firefox"),
    ("google/chrome", "Google Chrome", "Google Chrome"),
    ("microsoft edge", "Microsoft Edge", "Microsoft Edge"),
    ("mozilla", "Firefox", "firefox"),
    ("opera", "Opera", "Opera"),
    ("vivaldi", "Vivaldi", "Vivaldi"),
];

/// Everything that vouches for a bundle id: installed apps and their nested helpers, plug-ins,
/// launch agents, privileged helpers and running processes.
#[derive(Clone, Debug, Default)]
pub(crate) struct InstalledApps {
    ids: HashSet<String>,
    vendors: HashSet<String>,
    owners: HashMap<String, CacheOwner>,
    /// `false` if an app folder could not be fully read or an app could not be identified.
    pub complete: bool,
    /// A real installed id, used to check that Spotlight is actually answering.
    pub spotlight_canary: Option<String>,
}

impl InstalledApps {
    /// The real machine, plus `ctx.home` for the per-user locations.
    pub fn discover(ctx: &ScanContext) -> Self {
        let roots = [
            PathBuf::from("/Applications"),
            ctx.home_path("Applications"),
            PathBuf::from("/opt/homebrew/Caskroom"),
            PathBuf::from("/usr/local/Caskroom"),
        ];
        let mut apps = Self::from_roots(&roots);
        for relative in PLUGIN_ROOTS {
            apps.add_plugins(&Path::new("/").join(relative), 2);
            apps.add_plugins(&ctx.home_path(relative), 2);
        }
        for path in hm_core::fsutil::children(Path::new("/Library/PrivilegedHelperTools")) {
            apps.add_label(&name_of(&path));
        }
        for dir in [
            ctx.home_path("Library/LaunchAgents"),
            PathBuf::from("/Library/LaunchAgents"),
            PathBuf::from("/Library/LaunchDaemons"),
        ] {
            for path in hm_core::fsutil::children(&dir) {
                apps.add_label(name_of(&path).trim_end_matches(".plist"));
            }
        }
        apps.add_running(&RunningProcesses::current());
        apps
    }

    pub fn from_roots(roots: &[PathBuf]) -> Self {
        let mut apps = Self {
            complete: true,
            ..Self::default()
        };
        for root in roots {
            let search = find_app_bundles(root);
            apps.complete &= search.complete;
            for path in &search.paths {
                apps.add_app(path);
            }
        }
        apps
    }

    fn add_app(&mut self, path: &Path) {
        let info = read_bundle(path);
        if info.unreadable_plist {
            self.complete = false;
        }
        if let Some(process) = &info.executable {
            let owner = CacheOwner {
                display: info.name.clone(),
                process: process.clone(),
            };
            self.owners
                .entry(info.name.to_lowercase())
                .or_insert_with(|| owner.clone());
            if let Some(id) = &info.bundle_id {
                self.owners.entry(id.to_lowercase()).or_insert(owner);
            }
        }
        if let Some(id) = &info.bundle_id {
            // Prefer a third-party canary: Apple's system apps may sit outside Spotlight's scope.
            let is_apple = |id: &str| id.to_ascii_lowercase().starts_with("com.apple.");
            let canary_is_replaceable = self.spotlight_canary.as_deref().is_none_or(is_apple);
            if policy::is_reverse_dns(id)
                && canary_is_replaceable
                && (self.spotlight_canary.is_none() || !is_apple(id))
            {
                self.spotlight_canary = Some(id.clone());
            }
            self.add_id(id);
        }
        for relative in NESTED_BUNDLE_DIRS {
            for nested in hm_core::fsutil::children(&info.resolved.join(relative)) {
                let extension = nested
                    .extension()
                    .map(|ext| ext.to_string_lossy().to_ascii_lowercase());
                if matches!(extension.as_deref(), Some("app" | "framework")) {
                    self.add_nested(&nested);
                }
            }
        }
    }

    fn add_nested(&mut self, bundle: &Path) {
        for plist in [
            bundle.join("Contents/Info.plist"),
            bundle.join("Resources/Info.plist"),
        ] {
            if let Some(id) = bundle_id_at(&plist) {
                self.add_id(&id);
                return;
            }
        }
    }

    fn add_plugins(&mut self, dir: &Path, depth: usize) {
        for path in hm_core::fsutil::children(dir) {
            let extension = path
                .extension()
                .map(|ext| ext.to_string_lossy().to_ascii_lowercase());
            if extension
                .as_deref()
                .is_some_and(|ext| PLUGIN_EXTENSIONS.contains(&ext))
            {
                self.add_nested(&path);
            } else if depth > 0 && hm_core::fsutil::is_real_dir(&path) {
                self.add_plugins(&path, depth - 1);
            }
        }
    }

    /// A launchd label or helper tool name. These only vouch for their vendor when they look like a
    /// bundle id at all.
    pub fn add_label(&mut self, label: &str) {
        if policy::is_reverse_dns(label) {
            self.add_id(label);
        }
    }

    pub fn add_running(&mut self, running: &RunningProcesses) {
        for bundle in &running.bundles {
            if let Some(id) = read_bundle(bundle).bundle_id {
                self.add_id(&id);
            }
        }
    }

    fn add_id(&mut self, id: &str) {
        self.ids.insert(id.to_lowercase());
        let vendor = policy::vendor_of(id);
        if !vendor.is_empty() {
            self.vendors.insert(vendor);
        }
    }

    /// Something on this Mac vouches for this id. Matching on the vendor is deliberately broad:
    /// `com.google.Keystone` with Chrome installed is not a leftover, and missing a real leftover
    /// costs a few megabytes while a false positive costs someone's data.
    pub fn covers(&self, id: &str) -> bool {
        self.ids.contains(&id.to_lowercase()) || self.vendors.contains(&policy::vendor_of(id))
    }

    /// The app a cache folder belongs to, matching by bundle id, by a parent bundle id
    /// (`com.foo.app.ShipIt`) or by exact app name.
    pub fn owner_of(&self, cache_name: &str) -> Option<CacheOwner> {
        let lower = cache_name.to_lowercase();
        if let Some(&(_, display, process)) =
            NAMED_OWNERS.iter().find(|(name, _, _)| *name == lower)
        {
            return Some(CacheOwner {
                display: display.to_string(),
                process: process.to_string(),
            });
        }
        let mut key = lower.as_str();
        loop {
            if let Some(owner) = self.owners.get(key) {
                return Some(owner.clone());
            }
            match key.rfind('.') {
                Some(dot) if key[..dot].contains('.') => key = &key[..dot],
                _ => return None,
            }
        }
    }
}

fn bundle_id_at(plist: &Path) -> Option<String> {
    let value = Value::from_file(plist).ok()?;
    value
        .as_dictionary()?
        .get("CFBundleIdentifier")?
        .as_string()
        .map(str::to_string)
}

// ---- Spotlight ----------------------------------------------------------------------------------

/// A second opinion on whether an id is installed somewhere we did not look.
pub(crate) trait AppLocator: Sync {
    /// Whether answers can be trusted at all. Spotlight returns nothing, without any error, when
    /// indexing is off, which would make every app look uninstalled.
    fn is_healthy(&self) -> bool;
    /// Whether anything from this id's vendor exists. Must answer `true` when it cannot tell.
    fn knows(&self, id: &str) -> bool;
}

pub(crate) struct Spotlight {
    mdfind: Option<PathBuf>,
    canary: Option<String>,
}

impl Spotlight {
    pub fn new(ctx: &ScanContext, canary: Option<String>) -> Self {
        Self {
            mdfind: ctx.which("mdfind"),
            canary,
        }
    }

    /// `None` if `mdfind` failed or stalled.
    fn query(&self, id: &str) -> Option<bool> {
        let mdfind = self.mdfind.as_ref()?;
        let vendor = policy::vendor_of(id);
        if vendor.is_empty() || !policy::is_reverse_dns(id) {
            return None;
        }
        let query = format!("kMDItemCFBundleIdentifier == \"{vendor}.*\"c");
        let output = exec::run(mdfind, &[&query], MDFIND_TIMEOUT).ok()?;
        if !output.success() || output.timed_out {
            return None;
        }
        Some(!output.stdout.trim().is_empty())
    }
}

impl AppLocator for Spotlight {
    fn is_healthy(&self) -> bool {
        self.canary
            .as_deref()
            .is_some_and(|canary| self.query(canary) == Some(true))
    }

    fn knows(&self, id: &str) -> bool {
        // A stall must never read as "not installed": Mole shipped exactly that bug, and it deleted
        // the data of apps that were still installed.
        self.query(id).unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Fixture, LockedDir};

    #[test]
    fn reads_a_normal_bundle() {
        let home = Fixture::new();
        let path = home.app("Applications/Foo Bar.app", "com.example.foobar", "foobar");
        let info = read_bundle(&path);
        assert_eq!(info.name, "Foo Bar");
        assert_eq!(info.bundle_id.as_deref(), Some("com.example.foobar"));
        assert_eq!(info.version.as_deref(), Some("1.2.3"));
        assert_eq!(info.executable.as_deref(), Some("foobar"));
        assert!(info.executable_path.is_some());
        assert!(!info.unreadable_plist);
    }

    #[test]
    fn malformed_plist_is_flagged_not_fatal() {
        let home = Fixture::new();
        home.write_text(
            "Applications/Bad.app/Contents/Info.plist",
            "<plist><not valid",
        );
        let info = read_bundle(&home.path("Applications/Bad.app"));
        assert!(info.unreadable_plist);
        assert_eq!(info.name, "Bad");
        assert!(info.bundle_id.is_none());
    }

    #[test]
    fn an_app_with_no_plist_is_not_a_failure() {
        let home = Fixture::new();
        home.mkdir("Applications/Empty.app");
        let info = read_bundle(&home.path("Applications/Empty.app"));
        assert!(!info.unreadable_plist);
        assert!(info.bundle_id.is_none());
    }

    #[test]
    fn reads_a_wrapped_ios_app() {
        let home = Fixture::new();
        home.write_text(
            "Applications/Game.app/Wrapper/Game.app/Info.plist",
            r#"<plist version="1.0"><dict>
            <key>CFBundleIdentifier</key><string>com.example.game</string>
            <key>CFBundleExecutable</key><string>Game</string></dict></plist>"#,
        );
        home.write("Applications/Game.app/Wrapper/Game.app/Game", 64);
        let info = read_bundle(&home.path("Applications/Game.app"));
        assert_eq!(info.bundle_id.as_deref(), Some("com.example.game"));
        assert!(info.executable_path.is_some());
    }

    #[test]
    fn finds_apps_in_vendor_folders_but_not_inside_bundles() {
        let home = Fixture::new();
        home.app("Applications/Top.app", "com.example.top", "top");
        home.app(
            "Applications/Vendor/Suite/Nested.app",
            "com.example.nested",
            "nested",
        );
        home.app(
            "Applications/Top.app/Contents/Helpers/Helper.app",
            "com.example.helper",
            "h",
        );
        let search = find_app_bundles(&home.path("Applications"));
        assert!(search.complete);
        let mut names: Vec<String> = search.paths.iter().map(|path| name_of(path)).collect();
        names.sort();
        assert_eq!(names, ["Nested.app", "Top.app"]);
    }

    #[test]
    fn missing_root_is_complete_but_unreadable_root_is_not() {
        let home = Fixture::new();
        assert!(find_app_bundles(&home.path("nope")).complete);
        let locked = home.mkdir("Applications");
        let _guard = LockedDir::new(&locked);
        assert!(!find_app_bundles(&locked).complete);
    }

    #[test]
    fn malformed_app_makes_the_index_incomplete() {
        let home = Fixture::new();
        home.write_text("Applications/Bad.app/Contents/Info.plist", "garbage");
        let apps = InstalledApps::from_roots(&[home.path("Applications")]);
        assert!(!apps.complete);
    }

    #[test]
    fn covers_by_exact_id_and_vendor() {
        let home = Fixture::new();
        home.app(
            "Applications/Chrome.app",
            "com.google.Chrome",
            "Google Chrome",
        );
        let apps = InstalledApps::from_roots(&[home.path("Applications")]);
        assert!(apps.covers("com.google.chrome"));
        assert!(apps.covers("com.google.Keystone"));
        assert!(!apps.covers("com.example.gone"));
    }

    #[test]
    fn nested_helpers_vouch_for_their_own_vendor() {
        let home = Fixture::new();
        let app = home.app("Applications/Main.app", "com.example.main", "main");
        assert!(app.is_dir());
        home.app(
            "Applications/Main.app/Contents/Library/LoginItems/Helper.app",
            "org.othervendor.helper",
            "helper",
        );
        let apps = InstalledApps::from_roots(&[home.path("Applications")]);
        assert!(apps.covers("org.othervendor.anything"));
    }

    #[test]
    fn labels_only_count_when_they_look_like_bundle_ids() {
        let mut apps = InstalledApps::default();
        apps.add_label("com.vendor.agent");
        apps.add_label("homebrew");
        assert!(apps.covers("com.vendor.other"));
        assert!(!apps.covers("homebrew"));
    }

    #[test]
    fn running_apps_vouch_for_themselves() {
        let home = Fixture::new();
        let app = home.app("Downloads/Portable.app", "com.portable.app", "portable");
        let mut apps = InstalledApps::default();
        let mut running = RunningProcesses::default();
        running.bundles.insert(app);
        apps.add_running(&running);
        assert!(apps.covers("com.portable.app"));
    }

    #[test]
    fn owner_matches_id_parent_id_and_name() {
        let home = Fixture::new();
        home.app(
            "Applications/Steam.app",
            "com.valvesoftware.steam",
            "steam_osx",
        );
        let apps = InstalledApps::from_roots(&[home.path("Applications")]);
        let expected = Some(CacheOwner {
            display: "Steam".to_string(),
            process: "steam_osx".to_string(),
        });
        assert_eq!(apps.owner_of("com.valvesoftware.steam"), expected);
        assert_eq!(apps.owner_of("com.valvesoftware.steam.ShipIt"), expected);
        assert_eq!(apps.owner_of("Steam"), expected);
        assert_eq!(apps.owner_of("com.example.none"), None);
        assert_eq!(
            apps.owner_of("Google/Chrome").map(|o| o.process),
            Some("Google Chrome".to_string())
        );
    }

    #[test]
    fn outermost_app_stops_at_the_first_bundle() {
        assert_eq!(
            outermost_app(Path::new(
                "/Applications/Foo.app/Contents/Frameworks/H.app/Contents/MacOS/H"
            )),
            Some(PathBuf::from("/Applications/Foo.app"))
        );
        assert_eq!(outermost_app(Path::new("/usr/bin/ls")), None);
    }

    #[test]
    fn spotlight_without_mdfind_is_unhealthy_and_knows_everything() {
        let spotlight = Spotlight {
            mdfind: None,
            canary: Some("com.example.app".to_string()),
        };
        assert!(!spotlight.is_healthy());
        assert!(spotlight.knows("com.example.gone"));
    }

    #[test]
    fn spotlight_failure_reads_as_known() {
        // `/usr/bin/false` stands in for an `mdfind` that errors out.
        let spotlight = Spotlight {
            mdfind: Some(PathBuf::from("/usr/bin/false")),
            canary: Some("com.example.app".to_string()),
        };
        assert!(!spotlight.is_healthy());
        assert!(spotlight.knows("com.example.gone"));
    }

    #[test]
    fn empty_spotlight_answer_reads_as_unknown_to_spotlight() {
        // `/usr/bin/true` exits 0 with no output, as `mdfind` does for no matches.
        let spotlight = Spotlight {
            mdfind: Some(PathBuf::from("/usr/bin/true")),
            canary: Some("com.example.app".to_string()),
        };
        assert!(
            !spotlight.is_healthy(),
            "a canary that is not found means Spotlight is off"
        );
        assert!(!spotlight.knows("com.example.gone"));
    }
}

//! What HealthyMac refuses to touch, and why. Pure name-based rules so they can be tested and read
//! in one place.
//!
//! Apple publishes no list of which `com.apple.*` caches are safe to remove, and the evidence that
//! exists points one way. Users report iCloud (Photos, Notes, Drive) stuck on "syncing" after
//! clearing `~/Library/Caches`, until the next restart; `CloudKit/com.apple.bird` comes straight
//! back and re-downloads; and Mole, a widely used open-source cleaner, protects `com.apple.*` as a
//! blanket rule. So Apple's caches are denied by default and only named exceptions are allowed. On
//! the machine this was developed on they add up to under 500 MB, so the rule costs almost nothing.

/// Apple caches we do remove, each with a reason. Keep this short and justified.
///
/// `com.apple.Safari` is the network cache behind Safari's own Develop > Empty Caches, so emptying
/// it is something Apple already offers the user.
const ALLOWED_APPLE_CACHES: &[&str] = &["com.apple.safari"];

/// Apple-owned caches that do not carry the `com.apple.` prefix.
const APPLE_NAMES: &[&str] = &[
    "amsdatamigratortool",
    "animoji",
    "arfilecache",
    "cctclearcutlogger",
    "cloudkit",
    "familycircle",
    "features_config",
    "gamekit",
    "geoservices",
    "gippseudonymousid",
    "identityservices",
    "installation",
    "lsmimagecache",
    "maps",
    "metadata",
    "ocr",
    "passkit",
    "sharedimagecache",
    "sirientitycache",
    "siritts",
    "speechmodelcache",
    "temporaryitems",
    "trickplay",
    "tvapp_bag",
];

/// Caches that specialised scanners own, or that a generic sweep would break. This matters beyond
/// tidiness: `dedupe_findings` drops a whole finding over one shared path, so one of these in the
/// aggregate would take every other small cache down with it.
///
/// `pypoetry` deserves a note: despite living in Caches it holds real virtualenvs, and removing it
/// breaks every Poetry project until it is reinstalled.
const SPECIALISED_CACHES: &[&str] = &[
    // Named by the architecture: other scanners own these.
    "homebrew",
    "pip",
    "ms-playwright",
    "electron",
    "electron-builder",
    "jetbrains",
    "llama.cpp",
    "huggingface",
    "healthymac",
    // Python tooling, some of which keeps live environments here.
    "pypoetry",
    "pipx",
    "pipenv",
    "virtualenv",
    "uv",
    "pdm",
    "hatch",
    "conda",
    "mamba",
    "micromamba",
    // Developer package and build caches.
    "cocoapods",
    "org.swift.swiftpm",
    "org.carthage.carthagekit",
    "yarn",
    "pnpm",
    "npm",
    "bun",
    "deno",
    "node-gyp",
    "typescript",
    "go-build",
    "turbo",
    "turborepo",
    "gopls",
    "goimports",
    "golangci-lint",
    "cargo",
    "sccache",
    "mozilla.sccache",
    "ccache",
    "bazel",
    "gradle",
    "composer",
    "cypress",
    "puppeteer",
    // Local AI: a "cache" here is often a model the user downloaded on purpose.
    "ollama",
    "lm-studio",
    "lmstudio",
    "mlx",
    "torch",
    "whisper",
    "sentence_transformers",
    "diffusers",
];

/// Bundle-id prefixes whose data must survive a sweep. Sync clients decide what to upload from
/// local state, password managers and credential tools hold secrets, VPNs hold connectivity state,
/// and endpoint-security agents report tampering as malware.
const SENSITIVE_PREFIXES: &[&str] = &[
    // Passwords and credentials.
    "com.1password",
    "com.agilebits",
    "com.authy",
    "com.bitwarden",
    "com.dashlane",
    "com.keepassx",
    "com.lastpass",
    "com.yubico",
    "org.keepassx",
    "org.keepassxc",
    // File sync and backup.
    "com.amazon.drive",
    "com.backblaze",
    "com.box.",
    "com.dropbox",
    "com.getdropbox",
    "com.google.drivefs",
    "com.google.googledrive",
    "com.google.keystone",
    "com.mega",
    "com.microsoft.onedrive",
    "com.microsoft.sharepoint",
    "com.microsoft.syncreporter",
    "com.nextcloud",
    "com.owncloud",
    "com.pcloud",
    "com.resilio",
    "com.seafile",
    "com.synology",
    "nz.co.mega",
    "ws.agile",
    // VPN and network.
    "ch.protonvpn",
    "com.cloudflare",
    "com.expressvpn",
    "com.nordvpn",
    "com.privateinternetaccess",
    "com.surfshark",
    "com.windscribe",
    "com.wireguard",
    "com.zerotier",
    "io.tailscale",
    "net.mullvad",
    "net.openvpn",
    "org.amnezia",
    // Endpoint security, content filtering and device management.
    "com.adguard",
    "com.alibaba.security",
    "com.cisco.anyconnect",
    "com.cisco.secureclient",
    "com.crowdstrike",
    "com.eset",
    "com.jamf",
    "com.jamfsoftware",
    "com.paloaltonetworks",
    "com.sentinel-labs",
    "com.sentinelone",
    // Project, asset and licence data that users have lost to cleaners before.
    "com.adobe",
    "com.displaylink",
    "com.lasersoft-imaging",
    // Input methods keep learned typing data.
    "com.googlecode.rimeime",
    "im.rime",
];

/// Substrings that mark something as sensitive whatever its prefix.
const SENSITIVE_WORDS: &[&str] = &[
    "1password",
    "adobe",
    "bitwarden",
    "credential",
    "dropbox",
    "gnupg",
    "googledrive",
    "gpg",
    "inputmethod",
    "keychain",
    "onedrive",
    "openvpn",
    "password",
    "secret",
    "ssh",
    "tailscale",
    "token",
    "wireguard",
];

/// Reverse-DNS namespaces shared by many apps or by tools with no `.app`, so "no such app installed"
/// says nothing about whether the data is still in use.
const SHARED_NAMESPACES: &[&str] = &[
    "com.android",
    "com.breakpad",
    "com.bugsnag",
    "com.crashlytics",
    "com.datadoghq",
    "com.electron",
    "com.getsentry",
    "com.github.electron",
    "com.google.sparkle",
    "com.hockeyapp",
    "com.onevcat.kingfisher",
    "com.plausiblelabs",
    "com.segment",
    "dev.healthymac",
    "io.branch",
    "io.sentry",
    "net.hockeyapp",
    "org.gnu",
    "org.graalvm",
    "org.homebrew",
    "org.llvm",
    "org.nodejs",
    "org.python",
    "org.rust-lang",
    "org.sparkle-project",
    "org.swift",
    "sh.brew",
];

/// First components that make a dotted name a bundle id rather than, say, `main.kts.compiled.cache`.
const BUNDLE_ID_ROOTS: &[&str] = &[
    "ai", "app", "ar", "at", "au", "be", "biz", "br", "ca", "cc", "ch", "cl", "cn", "co", "com",
    "cz", "de", "dev", "dk", "edu", "es", "eu", "fi", "fm", "fr", "gg", "hk", "il", "im", "in",
    "info", "io", "is", "it", "jp", "kr", "ly", "me", "mx", "net", "nl", "no", "nz", "org", "pl",
    "pt", "ru", "se", "sg", "sh", "so", "tv", "tw", "uk", "us", "ws", "xyz",
];

/// `com.example.app`: three or more clean components starting at a known root. Deliberately strict,
/// since this is what lets a folder be called the leftover of an app at all.
pub(crate) fn is_reverse_dns(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() < 3 {
        return false;
    }
    let clean = parts.iter().all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    });
    clean && BUNDLE_ID_ROOTS.contains(&parts[0].to_ascii_lowercase().as_str())
}

/// `com.example.app` to `com.example`, lowercased. Empty for a name with no dots.
pub(crate) fn vendor_of(id: &str) -> String {
    let mut parts = id.split('.');
    match (parts.next(), parts.next()) {
        (Some(root), Some(vendor)) => format!("{root}.{vendor}").to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Belongs to Apple, by prefix, by known name, or because it looks like a system daemon's cache
/// (`familycircled`, `mbuseragent`). A wrong guess here only means a cache goes unreported.
pub(crate) fn is_apple(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("com.apple.")
        || lower.starts_with("group.com.apple")
        // Part of macOS, with Apple's name nowhere in sight.
        || lower.starts_with("org.cups.")
        || lower.starts_with("org.webkit.")
        || APPLE_NAMES.contains(&lower.as_str())
        || looks_like_daemon(name)
}

fn looks_like_daemon(name: &str) -> bool {
    name.len() >= 6
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && (name.ends_with('d') || name.ends_with("agent"))
}

pub(crate) fn is_allowed_apple_cache(name: &str) -> bool {
    ALLOWED_APPLE_CACHES.contains(&name.to_ascii_lowercase().as_str())
}

pub(crate) fn is_specialised_cache(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SPECIALISED_CACHES.contains(&lower.as_str()) || lower.starts_with("com.apple.dt.")
}

pub(crate) fn is_sensitive(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        || SENSITIVE_WORDS.iter().any(|word| lower.contains(word))
        || lower.ends_with(".inputmethod")
}

pub(crate) fn is_shared_namespace(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    SHARED_NAMESPACES
        .iter()
        .any(|namespace| lower == *namespace || lower.starts_with(&format!("{namespace}.")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_dns_is_strict() {
        for good in [
            "com.example.app",
            "org.videolan.vlc",
            "dev.warp.Warp-Stable",
            "io.x.y.z",
        ] {
            assert!(is_reverse_dns(good), "{good}");
        }
        for bad in [
            "Google",
            "notion.id",
            "main.kts.compiled.cache",
            "2BUA8C4S2C.com.1password.browser-helper",
            "com..app",
            "com.example.my app",
            "com.example.",
            "",
        ] {
            assert!(!is_reverse_dns(bad), "{bad}");
        }
    }

    #[test]
    fn apple_is_denied_unless_named() {
        for name in [
            "com.apple.bird",
            "com.apple.nsurlsessiond",
            "CloudKit",
            "FamilyCircle",
            "familycircled",
        ] {
            assert!(is_apple(name), "{name}");
        }
        assert!(is_allowed_apple_cache("com.apple.Safari"));
        assert!(!is_allowed_apple_cache("com.apple.Safari.SafeBrowsing"));
        assert!(is_apple("org.cups.PrintingPrefs"));
        assert!(!is_apple("com.spotify.client"));
        assert!(!is_apple("Spotify"));
    }

    #[test]
    fn sensitive_catches_prefixes_and_words() {
        for name in [
            "com.getdropbox.dropbox",
            "OneDrive",
            "com.microsoft.OneDriveStandaloneUpdater",
            "docker-secrets-engine",
            "io.tailscale.ipn.macsys",
            "com.adobe.Photoshop",
            "Adobe Camera Raw 2",
            "com.tencent.inputmethod.QQInput",
        ] {
            assert!(is_sensitive(name), "{name}");
        }
        assert!(!is_sensitive("com.spotify.client"));
    }

    #[test]
    fn specialised_caches_match_case_insensitively() {
        for name in [
            "Homebrew",
            "JetBrains",
            "pypoetry",
            "turbo",
            "com.apple.dt.Xcode",
            "HealthyMac",
        ] {
            assert!(is_specialised_cache(name), "{name}");
        }
        assert!(!is_specialised_cache("com.spotify.client"));
    }

    #[test]
    fn vendor_is_first_two_components() {
        assert_eq!(vendor_of("com.Foo.bar.baz"), "com.foo");
        assert_eq!(vendor_of("nodots"), "");
    }

    #[test]
    fn shared_namespaces_match_whole_components() {
        assert!(is_shared_namespace("org.sparkle-project.Sparkle"));
        assert!(!is_shared_namespace("org.pythonista.app"));
    }
}

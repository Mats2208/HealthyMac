//! Walks the user's own files for the large-file and duplicate scanners.
//!
//! Skips everything that is not plainly the user's to judge: `~/Library`, hidden tool folders,
//! cloud-synced folders, build output, and the insides of bundles.

use std::fs;
use std::os::macos::fs::MetadataExt as _;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use hm_core::{fsutil, ScanContext};
use rayon::prelude::*;

use crate::gate::{self, name_of};

/// From `<sys/stat.h>`: the file's contents are in the cloud. Reading it would download it.
const SF_DATALESS: u32 = 0x4000_0000;

/// Home folders never walked. Deleting inside a sync folder propagates to the user's other devices,
/// and most of what is in there is a placeholder anyway.
const SKIPPED_HOME_FOLDERS: &[&str] = &[
    "Applications",
    "Box",
    "Box Sync",
    "Dropbox",
    "Google Drive",
    "iCloud Drive",
    "Library",
    "MEGA",
    "Nextcloud",
    "OneDrive",
    "Sync",
    "pCloud Drive",
];

/// Media folders managed by their app. Removing a file from under them breaks its library.
const MANAGED_MEDIA: &[&str] = &["Music/Music", "Music/iTunes", "Movies/TV"];

/// Dependencies and build output, identical by design and owned by hm-dev.
const SKIPPED_DIR_NAMES: &[&str] = &[
    "DerivedData",
    "Pods",
    "__pycache__",
    "bower_components",
    "node_modules",
    "site-packages",
    "venv",
];

/// Bundles whose insides are not the user's files. Never entered.
const OPAQUE_BUNDLES: &[&str] = &[
    "app",
    "appex",
    "aplibrary",
    "bundle",
    "docset",
    "dsym",
    "framework",
    "kext",
    "key",
    "lrdata",
    "mdimporter",
    "mpkg",
    "musiclibrary",
    "numbers",
    "pages",
    "photoslibrary",
    "pkg",
    "plugin",
    "prefpane",
    "qlgenerator",
    "rtfd",
    "saver",
    "tvlibrary",
    "xcarchive",
    "xcodeproj",
    "xcworkspace",
    "xpc",
];

/// Bundles that behave as one big file to the user, so they are measured and reported whole.
const UNIT_BUNDLES: &[&str] = &[
    "band",
    "fcpbundle",
    "imovielibrary",
    "logicx",
    "pvm",
    "sparsebundle",
    "utm",
    "vbox",
    "vmwarevm",
];

/// Build output, which is the same file over and over by design. Too broad for large files, where
/// the worst case is showing someone their own 2 GB `build` folder content to judge, but a duplicate
/// finding names a copy to throw away.
const BUILD_DIR_NAMES: &[&str] = &[
    "Build", "build", "dist", "out", "release", "Release", "vendor",
];

/// What the walk is for, since being wrong costs different things.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// One file for the user to judge. Includes bundles measured whole, and anything in a project.
    LargeFiles,
    /// Tells the user which copy to discard, so it stays away from anything managed. A file checked
    /// into a repository is not a loose copy: on the Mac this was developed on, the alternative
    /// suggested trashing a project's `public/` source because `dist/` had the same bytes.
    Duplicates,
}

#[derive(Clone, Debug)]
pub(crate) struct UserFile {
    pub path: PathBuf,
    pub on_disk: u64,
    /// What removing it would release. Less than `on_disk` when it shares blocks with a clone.
    pub reclaimable: u64,
    pub logical: u64,
    pub modified: Option<SystemTime>,
    pub dev: u64,
    pub ino: u64,
    /// A bundle measured as one unit, not a regular file.
    pub is_bundle: bool,
}

#[derive(Debug, Default)]
pub(crate) struct UserFiles {
    pub files: Vec<UserFile>,
    /// A gated folder exists and was skipped for lack of Full Disk Access.
    pub skipped_gated: bool,
}

/// Files in the user's home using at least `min_bytes` on disk. Dataless files use none, so this
/// also keeps anything from being pulled down from the cloud.
pub(crate) fn user_files(ctx: &ScanContext, min_bytes: u64, purpose: Purpose) -> UserFiles {
    let access = gate::has_access(ctx);
    let mut outcome = UserFiles::default();
    let mut roots = Vec::new();

    for child in fsutil::children(&ctx.home) {
        let name = name_of(&child);
        if name.starts_with('.')
            || SKIPPED_HOME_FOLDERS.contains(&name.as_str())
            || name.starts_with("OneDrive")
            || !fsutil::is_real_dir(&child)
        {
            continue;
        }
        if gate::is_guarded_folder(&ctx.home, &child) && !access {
            outcome.skipped_gated = true;
            roots.extend(gate::configured_roots_inside(ctx, &child));
            continue;
        }
        roots.push(child);
    }

    let found = Mutex::new(Vec::new());
    roots.par_iter().for_each(|root| {
        walk(ctx, root, min_bytes, purpose, access, &found);
    });
    outcome.files = found.into_inner().unwrap_or_default();
    outcome
}

fn walk(
    ctx: &ScanContext,
    dir: &Path,
    min_bytes: u64,
    purpose: Purpose,
    access: bool,
    found: &Mutex<Vec<UserFile>>,
) {
    if ctx.is_cancelled() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let entries: Vec<_> = entries.flatten().collect();
    // Cargo and many others mark their output with this. Better than guessing at `target`.
    if entries
        .iter()
        .any(|entry| entry.file_name() == "CACHEDIR.TAG")
    {
        return;
    }
    if purpose == Purpose::Duplicates && entries.iter().any(|entry| entry.file_name() == ".git") {
        return;
    }
    entries.par_iter().for_each(|entry| {
        let path = entry.path();
        let name = name_of(&path);
        if name.starts_with('.') || gate::is_excluded(ctx, &path) {
            return;
        }
        // Does not follow symlinks.
        let Ok(meta) = entry.metadata() else {
            return;
        };
        if meta.file_type().is_symlink() {
            return;
        }
        let extension = path
            .extension()
            .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();

        if meta.is_dir() {
            // Guarded folders can be at any depth, not only directly under the home folder.
            if !access && gate::is_guarded_folder(&ctx.home, &path) {
                return;
            }
            if UNIT_BUNDLES.contains(&extension.as_str()) {
                if purpose == Purpose::Duplicates {
                    return;
                }
                let info = ctx.measure(&path);
                if info.bytes >= min_bytes {
                    push(
                        found,
                        UserFile {
                            path,
                            on_disk: info.bytes,
                            reclaimable: info.exclusive(),
                            logical: info.logical,
                            modified: info.newest_modified,
                            dev: meta.dev(),
                            ino: meta.ino(),
                            is_bundle: true,
                        },
                    );
                }
                return;
            }
            if OPAQUE_BUNDLES.contains(&extension.as_str())
                || SKIPPED_DIR_NAMES.contains(&name.as_str())
                || MANAGED_MEDIA
                    .iter()
                    .any(|media| ctx.home.join(media) == path)
                || (purpose == Purpose::Duplicates && BUILD_DIR_NAMES.contains(&name.as_str()))
            {
                return;
            }
            walk(ctx, &path, min_bytes, purpose, access, found);
            return;
        }

        if !meta.is_file() || meta.st_flags() & SF_DATALESS != 0 {
            return;
        }
        let on_disk = meta.blocks() * 512;
        if on_disk >= min_bytes {
            let reclaimable = fsutil::private_size(&path).unwrap_or(on_disk);
            push(
                found,
                UserFile {
                    path,
                    on_disk,
                    reclaimable,
                    logical: meta.len(),
                    modified: meta.modified().ok(),
                    dev: meta.dev(),
                    ino: meta.ino(),
                    is_bundle: false,
                },
            );
        }
    });
}

fn push(found: &Mutex<Vec<UserFile>>, file: UserFile) {
    if let Ok(mut files) = found.lock() {
        files.push(file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;

    fn names(home: &Fixture, min: u64) -> Vec<String> {
        let mut names: Vec<String> = user_files(&home.ctx, min, Purpose::LargeFiles)
            .files
            .iter()
            .map(|file| {
                file.path
                    .strip_prefix(home.dir.path())
                    .expect("inside home")
                    .display()
                    .to_string()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn finds_user_files_and_skips_tool_folders() {
        let home = Fixture::new();
        home.write("Projects/data/video.mov", 100_000);
        home.write("Library/Application Support/x/big.bin", 100_000);
        home.write(".cache/big.bin", 100_000);
        home.write("Projects/app/node_modules/big.bin", 100_000);
        home.write("Projects/.git/objects/pack.bin", 100_000);
        home.write("Dropbox/synced.bin", 100_000);
        home.write("OneDrive - Company/synced.bin", 100_000);
        home.write("Music/Music/Media/song.bin", 100_000);
        home.write(
            "Pictures/Photos Library.photoslibrary/originals/a.bin",
            100_000,
        );
        home.write("Projects/App.app/Contents/big.bin", 100_000);
        assert_eq!(names(&home, 50_000), ["Projects/data/video.mov"]);
    }

    #[test]
    fn skips_anything_marked_with_a_cachedir_tag() {
        let home = Fixture::new();
        home.write("Projects/rs/target/CACHEDIR.TAG", 10);
        home.write("Projects/rs/target/debug/big.bin", 100_000);
        assert!(names(&home, 50_000).is_empty());
    }

    #[test]
    fn does_not_follow_symlinks() {
        let home = Fixture::new();
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::write(outside.path().join("big.bin"), vec![1u8; 100_000]).expect("write");
        home.mkdir("Projects");
        std::os::unix::fs::symlink(outside.path(), home.path("Projects/link")).expect("symlink");
        assert!(names(&home, 50_000).is_empty());
    }

    #[test]
    fn gated_folders_need_access() {
        let home = Fixture::new();
        home.write("Documents/tesis.pdf", 100_000);
        home.write("Desktop/a.bin", 100_000);
        home.write("Downloads/b.bin", 100_000);
        home.write("Music/Own Recordings/take.wav", 100_000);
        let outcome = user_files(&home.ctx, 50_000, Purpose::LargeFiles);
        assert!(outcome.files.is_empty());
        assert!(outcome.skipped_gated);

        home.grant_full_disk_access();
        let outcome = user_files(&home.ctx, 50_000, Purpose::LargeFiles);
        assert_eq!(outcome.files.len(), 4);
        assert!(!outcome.skipped_gated);
    }

    #[test]
    fn a_configured_root_inside_a_gated_folder_is_walked_without_access() {
        let mut home = Fixture::new();
        home.write("Documents/code/data.bin", 100_000);
        home.write("Documents/private.bin", 100_000);
        home.ctx.config.project_roots = vec![home.path("Documents/code")];
        let outcome = user_files(&home.ctx, 50_000, Purpose::LargeFiles);
        assert!(outcome.skipped_gated);
        assert_eq!(outcome.files.len(), 1);
        assert!(outcome.files[0].path.ends_with("code/data.bin"));
    }

    #[test]
    fn media_libraries_at_any_depth_are_only_entered_with_access() {
        let home = Fixture::new();
        home.write("Movies/Proyectos/Viaje.imovielibrary/clip.mov", 100_000);
        home.write("Movies/grabacion.mov", 100_000);
        assert_eq!(names(&home, 50_000), ["Movies/grabacion.mov"]);

        home.grant_full_disk_access();
        assert_eq!(
            names(&home, 50_000),
            [
                "Movies/Proyectos/Viaje.imovielibrary",
                "Movies/grabacion.mov"
            ]
        );
    }

    #[test]
    fn unit_bundles_count_as_one_file() {
        let home = Fixture::new();
        home.write("Projects/disk.sparsebundle/bands/0", 60_000);
        home.write("Projects/disk.sparsebundle/bands/1", 60_000);
        let files = user_files(&home.ctx, 100_000, Purpose::LargeFiles).files;
        assert_eq!(files.len(), 1);
        assert!(files[0].is_bundle);
        assert!(files[0].on_disk >= 120_000);
        assert!(user_files(&home.ctx, 100_000, Purpose::Duplicates)
            .files
            .is_empty());
    }

    #[test]
    fn duplicates_stay_out_of_repositories_and_build_output() {
        let home = Fixture::new();
        home.write("Coding/repo/.git/HEAD", 10);
        home.write("Coding/repo/public/model.wasm", 100_000);
        home.write("Coding/plain/dist/app.wasm", 100_000);
        home.write("Coding/plain/vendor/tool", 100_000);
        home.write("Coding/plain/checkpoints/model.pth", 100_000);
        let found: Vec<PathBuf> = user_files(&home.ctx, 50_000, Purpose::Duplicates)
            .files
            .into_iter()
            .map(|file| file.path)
            .collect();
        assert_eq!(found, [home.path("Coding/plain/checkpoints/model.pth")]);
        // One large file is still the user's to judge, wherever it lives.
        assert_eq!(
            user_files(&home.ctx, 50_000, Purpose::LargeFiles)
                .files
                .len(),
            4
        );
    }

    /// hm-ai only looks in its known stores, all hidden or under `~/Library`, so a model anywhere
    /// else would otherwise show up nowhere.
    #[test]
    fn a_stray_model_counts_as_a_large_file() {
        let home = Fixture::new();
        home.write("models/llama.gguf", 100_000);
        assert_eq!(names(&home, 50_000), ["models/llama.gguf"]);
    }

    #[test]
    fn unreadable_folder_is_skipped_not_fatal() {
        let home = Fixture::new();
        home.write("Projects/ok.bin", 100_000);
        let locked = home.mkdir("Projects/locked");
        let _guard = crate::test_support::LockedDir::new(&locked);
        assert_eq!(names(&home, 50_000), ["Projects/ok.bin"]);
    }

    /// Absolute paths mean `PATH_MAX` bounds the recursion, which must not overflow the stack.
    #[test]
    fn the_deepest_possible_tree_does_not_crash() {
        crate::test_support::with_big_stack(|| {
            let home = Fixture::new();
            let room = 1000usize.saturating_sub(home.path("Projects").as_os_str().len());
            let deep = "d/".repeat(room / 2);
            home.write(&format!("Projects/{deep}big.bin"), 100_000);
            assert_eq!(
                user_files(&home.ctx, 50_000, Purpose::LargeFiles)
                    .files
                    .len(),
                1
            );
        });
    }

    #[test]
    fn user_exclusions_apply() {
        let mut home = Fixture::new();
        home.write("Projects/keep/big.bin", 100_000);
        home.ctx.config.excluded_paths = vec![home.path("Projects/keep")];
        assert!(names(&home, 50_000).is_empty());
    }

    #[test]
    fn cancel_stops_the_walk() {
        let home = Fixture::new();
        home.write("Projects/big.bin", 100_000);
        home.ctx.cancel.cancel();
        assert!(names(&home, 50_000).is_empty());
    }
}

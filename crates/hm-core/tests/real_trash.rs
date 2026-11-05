//! Exercises the default removal mode against the real macOS Trash.
//!
//! Ignored by default because each run leaves tiny files in the user's Trash. Run explicitly with:
//! `cargo test -p hm-core --test real_trash -- --ignored`

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use hm_core::{
    Action, Area, CancelToken, CleanOptions, CleanStatus, Cleaner, Finding, Recovery, Risk,
};

fn home() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    (dir, path)
}

fn finding(paths: Vec<PathBuf>) -> Finding {
    Finding::new("test.real_trash", Area::System, "k", "healthymac test")
        .risk(Risk::Safe)
        .recovery(Recovery::Automatic)
        .action(Action::Trash { paths })
}

#[test]
#[ignore = "writes to the real Trash"]
fn moves_a_file_to_the_trash_and_reports_it_as_not_yet_freed() {
    let (_guard, home) = home();
    let dir = home.join("Library/Caches/hm-trash-test");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("healthymac-test-file-{}.txt", std::process::id()));
    fs::write(&file, vec![1u8; 8192]).unwrap();

    let report = Cleaner::new(&home, &[], CleanOptions::default()).clean(
        &[finding(vec![file.clone()])],
        &CancelToken::new(),
        &|_| {},
    );

    let outcome = &report.outcomes[0];
    assert_eq!(outcome.status, CleanStatus::Done, "{:?}", outcome.errors);
    assert!(!file.exists(), "the file should have left its folder");
    assert!(outcome.trashed_bytes >= 8192);
    assert_eq!(outcome.freed_bytes, 0, "the Trash does not free space yet");
}

#[test]
#[ignore = "writes to the real Trash"]
fn trashing_a_symlink_removes_the_link_and_never_its_target() {
    let (_guard, home) = home();
    let precious = home.join("Coding/precious");
    fs::create_dir_all(&precious).unwrap();
    let keepsake = precious.join("thesis.txt");
    fs::write(&keepsake, b"do not lose me").unwrap();

    let caches = home.join("Library/Caches");
    fs::create_dir_all(&caches).unwrap();
    let link = caches.join(format!("healthymac-test-link-{}", std::process::id()));
    symlink(&precious, &link).unwrap();

    let report = Cleaner::new(&home, &[], CleanOptions::default()).clean(
        &[finding(vec![link.clone()])],
        &CancelToken::new(),
        &|_| {},
    );

    let outcome = &report.outcomes[0];
    assert_eq!(outcome.status, CleanStatus::Done, "{:?}", outcome.errors);
    assert!(
        fs::symlink_metadata(&link).is_err(),
        "the link should be gone"
    );
    assert!(precious.is_dir(), "the link's target must survive");
    assert_eq!(fs::read(&keepsake).unwrap(), b"do not lose me");
}

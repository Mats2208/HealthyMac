//! Junk scanners for the System, Apps and Files areas.

mod backups;
mod caches;
mod duplicates;
mod installers;
mod large;
mod leftovers;
mod logs;
pub(crate) mod policy;
mod snapshots;
mod trash;
mod walk;

use hm_core::Scanner;

/// Every scanner this crate provides.
///
/// Order matters for one thing: `dedupe_findings` keeps the earlier of two findings in the same
/// area that target the same path. Duplicates come before large files so that a file which is both
/// keeps the finding that says which copy survives.
pub fn scanners() -> Vec<Box<dyn Scanner>> {
    vec![
        Box::new(caches::UserCaches),
        Box::new(logs::Logs),
        Box::new(trash::Trash),
        Box::new(duplicates::Duplicates),
        Box::new(installers::Installers),
        Box::new(large::LargeFiles),
        Box::new(backups::IosBackups),
        Box::new(snapshots::LocalSnapshots),
        Box::new(leftovers::AppLeftovers),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;
    use hm_core::{run_scan, Action, Risk};

    #[test]
    fn ids_are_namespaced_and_unique() {
        let all = scanners();
        let mut ids: Vec<&str> = all.iter().map(|scanner| scanner.id()).collect();
        assert!(ids.iter().all(|id| id.starts_with("sys.")
            || id.starts_with("files.")
            || id.starts_with("apps.")));
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before);
        assert!(all
            .iter()
            .all(|scanner| !scanner.name().is_empty() && !scanner.description().is_empty()));
    }

    /// The whole set against a home with nothing in it: no scanner may fail, and every finding must
    /// honour the rules that apply to all of them.
    #[test]
    fn empty_home_scans_cleanly_and_honours_the_rules() {
        let home = Fixture::new();
        let errors = std::sync::Mutex::new(Vec::new());
        let findings = run_scan(&scanners(), &home.ctx, &|event| {
            if let hm_core::ScanEvent::Failed { scanner_id, error } = event {
                errors
                    .lock()
                    .expect("lock")
                    .push(format!("{scanner_id}: {error}"));
            }
        });
        let errors = errors.into_inner().expect("lock");
        // `tmutil` and `mdfind` depend on the machine. Both may be unavailable, nothing else may.
        assert!(
            errors
                .iter()
                .all(|error| error.starts_with("sys.snapshots")),
            "{errors:?}"
        );
        for finding in &findings {
            assert!(
                !finding.explanation.is_empty(),
                "{} sin explicación",
                finding.id
            );
            if finding.risk == Risk::Review {
                assert!(
                    !finding.selected_by_default,
                    "{} preseleccionado",
                    finding.id
                );
            }
            if matches!(finding.action, Action::ReportOnly) {
                assert_eq!(finding.size, 0, "{}: informativo con tamaño", finding.id);
            }
        }
    }
}

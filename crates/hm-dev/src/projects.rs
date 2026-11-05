//! Finds development projects under the configured roots.

use std::path::Path;

use anyhow::Result;
use hm_core::ScanContext;
use rayon::prelude::*;

use crate::sweep::{parse_commit_time, shared_sweep, SweptProject};
use crate::types::{GitInfo, ProjectArtifact, ProjectInfo};
use crate::util;

/// What `git status --porcelain=v2 --branch` says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct GitStatus {
    branch: Option<String>,
    dirty: bool,
    unpushed: Option<u32>,
}

fn parse_status(output: &str) -> GitStatus {
    let mut status = GitStatus::default();
    for line in output.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            status.branch = (head != "(detached)").then(|| head.to_string());
        } else if let Some(counts) = line.strip_prefix("# branch.ab ") {
            // Absent with no upstream, or when the upstream branch is gone.
            status.unpushed = counts
                .split_whitespace()
                .next()
                .and_then(|ahead| ahead.strip_prefix('+'))
                .and_then(|ahead| ahead.parse().ok());
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            status.dirty = true;
        }
    }
    status
}

fn git_info(git: &Path, project: &SweptProject) -> GitInfo {
    let mut info = GitInfo {
        last_commit: project.last_commit,
        git_dir_bytes: project.git_dir_bytes,
        ..GitInfo::default()
    };
    let status = util::run_git(
        git,
        &project.path,
        &["status", "--porcelain=v2", "--branch"],
    )
    .filter(|output| output.success());
    if let Some(output) = status {
        let parsed = parse_status(&output.stdout);
        info.branch = parsed.branch;
        info.dirty = parsed.dirty;
        info.unpushed_commits = parsed.unpushed;
        info.status_known = true;
    }
    // Set by the sweep, but only if `git log` ran. This keeps the two views of a project consistent.
    if info.last_commit.is_none() {
        info.last_commit = util::run_git(git, &project.path, &["log", "-1", "--format=%ct"])
            .filter(|output| output.success())
            .and_then(|output| parse_commit_time(&output.stdout));
    }
    info
}

fn project_info(ctx: &ScanContext, project: &SweptProject, git: Option<&Path>) -> ProjectInfo {
    let mut artifacts: Vec<ProjectArtifact> = project
        .artifacts
        .iter()
        .map(|artifact| ProjectArtifact {
            path: artifact.path.clone(),
            label: artifact.label.clone(),
            size: artifact.info.exclusive(),
        })
        .collect();
    artifacts.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    ProjectInfo {
        name: project.name.clone(),
        path: project.path.clone(),
        kinds: project.kinds.clone(),
        last_activity: project.last_activity(),
        idle_days: project.idle_days(ctx.now),
        artifact_bytes: project.artifact_bytes(),
        artifacts,
        git: match (git, project.repo.is_some()) {
            (Some(git), true) => Some(git_info(git, project)),
            _ => None,
        },
    }
}

/// Lists projects with their artifacts and git state, largest artifacts first. Slow: call it from a
/// background thread.
pub fn list_projects(ctx: &ScanContext) -> Result<Vec<ProjectInfo>> {
    let sweep = shared_sweep(ctx);
    let git = util::tool(ctx, "git");
    let mut projects: Vec<ProjectInfo> = sweep
        .projects
        .par_iter()
        .map(|project| project_info(ctx, project, git.as_deref()))
        .collect();
    projects.sort_by(|a, b| {
        b.artifact_bytes
            .cmp(&a.artifact_bytes)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(projects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Fixture;
    use crate::types::ProjectKind;

    #[test]
    fn parses_status_on_a_branch_with_an_upstream() {
        let output = "\
# branch.oid 8fd2e6a1c3b4d5e6f7a8b9c0d1e2f3a4b5c6d7e8
# branch.head main
# branch.upstream origin/main
# branch.ab +3 -1
1 .M N... 100644 100644 100644 aaaa bbbb src/lib.rs
? scratch.txt
";
        assert_eq!(
            parse_status(output),
            GitStatus {
                branch: Some("main".to_string()),
                dirty: true,
                unpushed: Some(3),
            }
        );
    }

    #[test]
    fn parses_status_with_no_upstream_and_a_clean_tree() {
        let output = "# branch.oid 8fd2e6\n# branch.head feature/x\n";
        assert_eq!(
            parse_status(output),
            GitStatus {
                branch: Some("feature/x".to_string()),
                dirty: false,
                unpushed: None,
            }
        );
    }

    #[test]
    fn parses_status_on_a_detached_head_and_an_empty_repository() {
        let detached = "# branch.oid 8fd2e6\n# branch.head (detached)\n";
        assert_eq!(parse_status(detached).branch, None);
        let empty = "# branch.oid (initial)\n# branch.head master\n? .gitignore\n";
        let status = parse_status(empty);
        assert_eq!(status.branch.as_deref(), Some("master"));
        assert!(status.dirty);
        assert_eq!(status.unpushed, None);
    }

    #[test]
    fn an_upstream_that_is_gone_is_not_read_as_up_to_date() {
        let output = "# branch.head main\n# branch.upstream origin/main\n";
        assert_eq!(parse_status(output).unpushed, None);
    }

    #[test]
    fn malformed_status_does_not_panic() {
        assert_eq!(parse_status(""), GitStatus::default());
        assert_eq!(parse_status("# branch.ab nonsense\n").unpushed, None);
    }

    #[test]
    fn lists_kinds_artifacts_and_git_state_for_a_real_repository() {
        let fx = Fixture::new();
        fx.file("Coding/web/package.json", b"{}");
        fx.file("Coding/web/Cargo.toml", b"");
        fx.file("Coding/web/.gitignore", b"node_modules\n");
        fx.file("Coding/web/node_modules/x.js", &[1u8; 8192]);
        fx.git("Coding/web", &["init", "-q", "-b", "trunk"]);
        fx.git(
            "Coding/web",
            &["add", "package.json", "Cargo.toml", ".gitignore"],
        );
        fx.git("Coding/web", &["commit", "-q", "-m", "x"]);
        fx.file("Coding/web/untracked.txt", b"x");

        fx.file("Coding/plain/pyproject.toml", b"");

        let projects = list_projects(&fx.ctx()).unwrap();
        assert_eq!(projects.len(), 2);
        // Largest artifacts first.
        assert_eq!(projects[0].name, "web");
        assert_eq!(projects[1].name, "plain");

        let web = &projects[0];
        assert!(web.kinds.contains(&ProjectKind::Node));
        assert!(web.kinds.contains(&ProjectKind::Rust));
        assert_eq!(web.artifacts.len(), 1);
        assert_eq!(web.artifacts[0].label, "node_modules");
        assert!(web.artifact_bytes >= 8192);
        assert!(web.last_activity.is_some());
        assert_eq!(web.idle_days, Some(0));

        let git = web.git.as_ref().unwrap();
        assert!(git.status_known);
        assert_eq!(git.branch.as_deref(), Some("trunk"));
        assert!(git.dirty);
        assert_eq!(git.unpushed_commits, None, "no upstream is not an error");
        assert!(git.last_commit.is_some());
        assert!(git.git_dir_bytes > 0);

        assert!(projects[1].git.is_none());
        assert_eq!(projects[1].kinds, vec![ProjectKind::Python]);
    }

    #[test]
    fn counts_unpushed_commits_against_an_upstream() {
        let fx = Fixture::new();
        let upstream = fx.dir("elsewhere/upstream.git");
        fx.git(
            "elsewhere/upstream.git",
            &["init", "-q", "--bare", "-b", "trunk"],
        );
        fx.file("Coding/app/package.json", b"{}");
        fx.git("Coding/app", &["init", "-q", "-b", "trunk"]);
        fx.git("Coding/app", &["add", "."]);
        fx.git("Coding/app", &["commit", "-q", "-m", "one"]);
        fx.git(
            "Coding/app",
            &["remote", "add", "origin", upstream.to_str().unwrap()],
        );
        fx.git("Coding/app", &["push", "-q", "-u", "origin", "trunk"]);
        fx.git(
            "Coding/app",
            &["commit", "-q", "--allow-empty", "-m", "two"],
        );
        fx.git(
            "Coding/app",
            &["commit", "-q", "--allow-empty", "-m", "three"],
        );

        let projects = list_projects(&fx.ctx()).unwrap();
        let app = projects
            .iter()
            .find(|project| project.name == "app")
            .unwrap();
        let git = app.git.as_ref().unwrap();
        assert_eq!(git.unpushed_commits, Some(2));
        assert!(!git.dirty);
    }

    #[test]
    fn an_empty_repository_has_no_last_commit_and_is_not_a_failure() {
        let fx = Fixture::new();
        fx.file("Coding/new/README.md", b"x");
        fx.git("Coding/new", &["init", "-q"]);
        let projects = list_projects(&fx.ctx()).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].kinds, vec![ProjectKind::Other]);
        let git = projects[0].git.as_ref().unwrap();
        assert_eq!(git.last_commit, None);
        assert!(git.dirty);
    }

    #[test]
    fn a_broken_repository_still_lists_the_project() {
        let fx = Fixture::new();
        fx.file("Coding/odd/package.json", b"{}");
        fx.file("Coding/odd/node_modules/x.js", b"x");
        fx.file("Coding/odd/.git", b"gitdir: /nowhere/at/all\n");
        let projects = list_projects(&fx.ctx()).unwrap();
        assert_eq!(projects.len(), 1);
        let git = projects[0].git.as_ref().unwrap();
        assert!(!git.status_known);
        assert_eq!(git.branch, None);
        assert_eq!(git.last_commit, None);
    }

    #[test]
    fn a_monorepo_collects_kinds_from_its_packages_but_not_deep_fixtures() {
        let fx = Fixture::new();
        fx.file("Coding/mono/Cargo.toml", b"");
        fx.file("Coding/mono/apps/web/package.json", b"{}");
        fx.file("Coding/mono/tests/fixtures/deep/sample/pyproject.toml", b"");
        fx.git("Coding/mono", &["init", "-q"]);
        let projects = list_projects(&fx.ctx()).unwrap();
        assert_eq!(projects.len(), 1);
        assert!(projects[0].kinds.contains(&ProjectKind::Rust));
        assert!(projects[0].kinds.contains(&ProjectKind::Node));
        assert!(!projects[0].kinds.contains(&ProjectKind::Python));
    }

    #[test]
    fn no_project_roots_is_an_empty_list() {
        let fx = Fixture::new();
        assert!(list_projects(&fx.ctx()).unwrap().is_empty());
    }
}

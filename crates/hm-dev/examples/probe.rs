//! Runs everything in this crate against the real machine and prints what it finds. Read-only: it
//! never builds a `Cleaner`, so nothing it prints has been acted on.
//!
//! `cargo run -p hm-dev --example probe`, with `--all` to list every finding, `--explain` to print
//! their explanations, `--json` to dump them, or `--bench-sweep` to time the sweep alone.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Instant;

use hm_core::fmt::human_size;
use hm_core::{run_scan, Action, Finding, Risk, SafetyPolicy, ScanContext, ScanEvent};

fn action_kind(action: &Action) -> String {
    match action {
        Action::Trash { paths } => format!("trash×{}", paths.len()),
        Action::TrashContents { .. } => "contents".to_string(),
        Action::Command { program, args } => format!("`{program} {}`", args.join(" ")),
        Action::ReportOnly => "report".to_string(),
        other => format!("{other:?}"),
    }
}

fn print_finding(finding: &Finding) {
    println!(
        "    {:>9}  {:<8} {} {:<28} {} [{}]",
        human_size(finding.size),
        finding.risk.label(),
        if finding.selected_by_default {
            "x"
        } else {
            " "
        },
        finding.group.as_deref().unwrap_or("-"),
        finding.title,
        action_kind(&finding.action),
    );
    if std::env::args().any(|arg| arg == "--explain") {
        println!("                {}", finding.explanation);
    }
}

/// The rules that must hold on real data, whatever the fixtures say.
fn check_invariants(ctx: &ScanContext, findings: &[Finding]) -> Vec<String> {
    let policy = SafetyPolicy::new(&ctx.home, &ctx.config.excluded_paths);
    let protected = [
        "Documents",
        "Desktop",
        "Downloads",
        "Library/Containers",
        "Library/Group Containers",
    ];
    let mut problems = Vec::new();
    for finding in findings {
        if finding.risk == Risk::Review && finding.selected_by_default {
            problems.push(format!("Review preselected: {}", finding.id));
        }
        if finding.explanation.is_empty() {
            problems.push(format!("no explanation: {}", finding.id));
        }
        if !finding.scanner_id.starts_with("dev.") {
            problems.push(format!("bad namespace: {}", finding.id));
        }
        if let Action::Command { program, .. } = &finding.action {
            if policy.check_command(program).is_err() {
                problems.push(format!("program not allowed: {program} in {}", finding.id));
            }
        }
        for path in finding.target_paths() {
            for folder in protected {
                if path.starts_with(ctx.home.join(folder)) {
                    problems.push(format!("target in a guarded folder: {}", path.display()));
                }
            }
            let checked = match &finding.action {
                // The cleaner validates the children of this one, and it does not need to pass.
                Action::TrashContents { dir } => hm_core::fsutil::children(dir)
                    .first()
                    .map(|child| policy.check_removal(child)),
                _ => Some(policy.check_removal(path)),
            };
            if let Some(Err(violation)) = checked {
                problems.push(format!("{}: {violation}", finding.id));
            }
        }
    }
    problems
}

/// Times the sweep alone, and lists every project whose git status could not be read.
fn bench_sweep() -> anyhow::Result<()> {
    let ctx = ScanContext::for_current_user()?;
    let started = Instant::now();
    let projects = hm_dev::projects::list_projects(&ctx)?;
    let artifacts: usize = projects.iter().map(|project| project.artifacts.len()).sum();
    println!(
        "sweep and git state: {:.2?}, {} projects, {artifacts} artifacts",
        started.elapsed(),
        projects.len()
    );
    for project in &projects {
        if project.git.as_ref().is_some_and(|git| !git.status_known) {
            println!("  unknown git status: {}", project.path.display());
        }
    }
    let mut biggest: Vec<(u64, &str)> = projects
        .iter()
        .filter_map(|project| Some((project.git.as_ref()?.git_dir_bytes, project.name.as_str())))
        .collect();
    biggest.sort_unstable_by(|a, b| b.cmp(a));
    for (bytes, name) in biggest.iter().take(3) {
        println!("  .git {:>9}  {name}", human_size(*bytes));
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--bench-sweep") {
        return bench_sweep();
    }
    if std::env::args().any(|arg| arg == "--json") {
        let ctx = ScanContext::for_current_user()?;
        let findings = run_scan(&hm_dev::scanners(), &ctx, &|_| {});
        println!("{}", serde_json::to_string(&findings)?);
        return Ok(());
    }
    let show_all = std::env::args().any(|arg| arg == "--all");
    let ctx = ScanContext::for_current_user()?;
    println!("home: {}", ctx.home.display());
    println!("roots: {:?}", ctx.config.effective_project_roots(&ctx.home));
    println!(
        "full disk access: {}",
        hm_core::fsutil::has_full_disk_access(&ctx.home)
    );
    println!();

    let scanners = hm_dev::scanners();
    let timings = Mutex::new(BTreeMap::new());
    let started = Instant::now();
    let findings = run_scan(&scanners, &ctx, &|event| match event {
        ScanEvent::Finished {
            scanner_id,
            findings,
            elapsed,
        } => {
            if let Ok(mut timings) = timings.lock() {
                timings.insert(scanner_id, (findings.len(), elapsed));
            }
        }
        ScanEvent::Failed { scanner_id, error } => println!("FAILED {scanner_id}: {error}"),
        ScanEvent::Started { .. } => {}
    });
    let elapsed = started.elapsed();
    println!(
        "=== Scan: {elapsed:.2?}, {} findings after dedupe",
        findings.len()
    );
    if let Ok(timings) = timings.lock() {
        for (scanner, (count, took)) in timings.iter() {
            println!("  {scanner:<16} {took:>10.2?}  {count} findings");
        }
    }
    let total: u64 = findings.iter().map(|finding| finding.size).sum();
    let selected: u64 = findings
        .iter()
        .filter(|finding| finding.selected_by_default)
        .map(|finding| finding.size)
        .sum();
    println!(
        "  total listed {}, preselected {}",
        human_size(total),
        human_size(selected)
    );
    println!();

    let mut sorted = findings.clone();
    sorted.sort_by_key(|finding| std::cmp::Reverse(finding.size));
    for scanner in &scanners {
        let mine: Vec<&Finding> = sorted
            .iter()
            .filter(|finding| finding.scanner_id == scanner.id())
            .collect();
        let bytes: u64 = mine.iter().map(|finding| finding.size).sum();
        println!(
            "--- {} ({}): {} findings, {}",
            scanner.name(),
            scanner.id(),
            mine.len(),
            human_size(bytes)
        );
        let limit = if show_all { usize::MAX } else { 10 };
        for finding in mine.iter().take(limit) {
            print_finding(finding);
        }
        if mine.len() > limit {
            println!("    ... and {} more", mine.len() - limit);
        }
    }

    println!();
    let problems = check_invariants(&ctx, &findings);
    println!("=== Invariants: {} problems", problems.len());
    for problem in problems.iter().take(30) {
        println!("  {problem}");
    }

    println!();
    let started = Instant::now();
    let projects = hm_dev::projects::list_projects(&ctx)?;
    println!(
        "=== Projects: {} in {:.2?} (sweep shared with the scan above)",
        projects.len(),
        started.elapsed()
    );
    let with_git = projects
        .iter()
        .filter(|project| project.git.is_some())
        .count();
    let dirty = projects
        .iter()
        .filter(|project| project.git.as_ref().is_some_and(|git| git.dirty))
        .count();
    let unknown = projects
        .iter()
        .filter(|project| project.git.as_ref().is_some_and(|git| !git.status_known))
        .count();
    println!("  {with_git} git repositories, {dirty} dirty, {unknown} with unknown status");
    for project in projects.iter().take(8) {
        let git = project.git.as_ref();
        println!(
            "  {:>9}  {:<26} idle {:>4}  {:?}  {} {}",
            human_size(project.artifact_bytes),
            project.name,
            project
                .idle_days
                .map(|days| days.to_string())
                .unwrap_or_else(|| "?".to_string()),
            project.kinds,
            git.and_then(|git| git.branch.clone()).unwrap_or_default(),
            match git.and_then(|git| git.unpushed_commits) {
                Some(count) => format!("(+{count})"),
                None => String::new(),
            },
        );
    }

    println!();
    let started = Instant::now();
    let ports = hm_dev::ports::list_listening()?;
    println!("=== Ports: {} in {:.2?}", ports.len(), started.elapsed());
    for port in &ports {
        println!(
            "  {:>5} {:<16} {:<28} pid {:<6} {} | cwd {}",
            port.port,
            port.address,
            port.process_name,
            port.pid,
            port.service_hint.as_deref().unwrap_or("-"),
            port.cwd
                .as_ref()
                .map(|cwd| cwd.display().to_string())
                .unwrap_or_else(|| "?".to_string()),
        );
    }
    let with_command = ports.iter().filter(|port| port.command.is_some()).count();
    println!("  full command known for {with_command} of {}", ports.len());

    println!();
    let started = Instant::now();
    match hm_dev::docker::summary(&ctx) {
        Some(summary) => println!("=== Docker in {:.2?}: {summary:#?}", started.elapsed()),
        None => println!("=== Docker: no runtime installed"),
    }
    Ok(())
}

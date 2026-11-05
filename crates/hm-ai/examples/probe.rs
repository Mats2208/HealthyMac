//! Prints what the inventory, status and scanners see on this machine. Read-only apart from
//! HealthyMac's own hash cache. Never calls the cleaner.
//!
//! `cargo run -p hm-ai --example probe`

use std::time::Instant;

use hm_ai::{fit, gguf, inventory, runtime, scanners};
use hm_core::fmt::{human_age, human_size};
use hm_core::{fsutil, run_scan, ScanContext, ScanEvent};

fn main() -> anyhow::Result<()> {
    let ctx = ScanContext::for_current_user()?;
    println!("home: {}", ctx.home.display());
    println!(
        "full disk access: {}",
        fsutil::has_full_disk_access(&ctx.home)
    );

    let budget = fit::current_budget();
    println!(
        "memory: {} bytes RAM ({}), GPU limit {} bytes ({}), {:.1}%",
        budget.total_ram,
        human_size(budget.total_ram),
        budget.gpu_limit,
        human_size(budget.gpu_limit),
        budget.gpu_limit as f64 / budget.total_ram as f64 * 100.0
    );

    println!("\n== runtime::status ==");
    let started = Instant::now();
    let statuses = runtime::status(&ctx);
    println!("took {:?}", started.elapsed());
    for row in &statuses {
        println!(
            "{:<14} installed={:<5} running={:<5} version={:<10} endpoint={:<24} storage={}",
            row.kind.label(),
            row.installed,
            row.running,
            row.version.as_deref().unwrap_or("-"),
            row.endpoint.as_deref().unwrap_or("-"),
            row.storage_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
    }

    println!("\n== inventory::scan ==");
    let started = Instant::now();
    let found = inventory::scan(&ctx)?;
    println!("took {:?}", started.elapsed());
    println!(
        "{} models, {} on disk, {} reclaimable by dedupe, hashed {} files ({})",
        found.models.len(),
        human_size(found.total_bytes),
        human_size(found.reclaimable_bytes),
        found.hashed_files,
        human_size(found.hashed_bytes)
    );
    println!("skipped roots: {:?}", found.skipped_roots);
    for model in &found.models {
        println!(
            "{:>8} {:<14} {:<12?} {:<14} q={:<14} arch={:<16} ctx={:<8} fit={:<14} {}{} {} ({})",
            human_size(model.size),
            model.runtime.label(),
            model.format,
            model
                .parameter_count
                .map(|p| format!("{p}p"))
                .unwrap_or_default(),
            model.quantization.as_deref().unwrap_or("-"),
            model.architecture.as_deref().unwrap_or("-"),
            model
                .context_length
                .map(|c| c.to_string())
                .unwrap_or_default(),
            model.fit.label(),
            if model.incomplete {
                "[INCOMPLETO] "
            } else {
                ""
            },
            model.name,
            model
                .last_used
                .map(|t| human_age(t, ctx.now))
                .unwrap_or_default(),
            model.last_used_source.label(),
        );
    }

    println!("\n== duplicates ==");
    for group in &found.duplicates {
        println!(
            "{} each, {} reclaimable, sha256 {}",
            human_size(group.file_size),
            human_size(group.reclaimable),
            group.sha256
        );
        for path in &group.paths {
            println!("  {}", path.display());
        }
    }
    if found.duplicates.is_empty() {
        println!("none");
    }

    println!("\n== runtimes with storage ==");
    for row in &found.runtimes {
        println!("{:<22} {}", row.kind.label(), human_size(row.storage_bytes));
    }

    println!("\n== gguf::read_info on real files ==");
    let bundled = ctx.home_path(
        ".lmstudio/.internal/bundled-models/nomic-ai/nomic-embed-text-v1.5-GGUF/nomic-embed-text-v1.5.Q4_K_M.gguf",
    );
    let from_inventory = found
        .models
        .iter()
        .flat_map(|model| model.files.iter())
        .filter(|path| path.extension().is_some_and(|ext| ext == "gguf"))
        .cloned();
    for path in std::iter::once(bundled).chain(from_inventory) {
        let started = Instant::now();
        let info = gguf::read_info(&path);
        println!("{:?} {}\n  {:?}", started.elapsed(), path.display(), info);
    }

    println!("\n== scanners (no cleaning, ever) ==");
    let list = scanners();
    let started = Instant::now();
    let findings = run_scan(&list, &ctx, &|event: ScanEvent| {
        if let ScanEvent::Failed { scanner_id, error } = event {
            println!("FAILED {scanner_id}: {error}");
        }
    });
    println!("{} findings in {:?}", findings.len(), started.elapsed());
    for finding in &findings {
        let action = match &finding.action {
            hm_core::Action::Trash { paths } => format!("trash {} paths", paths.len()),
            hm_core::Action::TrashContents { .. } => "trash contents".to_string(),
            hm_core::Action::Command { program, args } => format!("{program} {}", args.join(" ")),
            hm_core::Action::CloneDedupe { duplicates, .. } => {
                format!("clone {} files", duplicates.len())
            }
            hm_core::Action::EmptyTrash => "empty trash".to_string(),
            hm_core::Action::ReportOnly => "report only".to_string(),
        };
        println!(
            "{:>8} {:<9} sel={:<5} {:<24} {} [{}]",
            human_size(finding.size),
            finding.risk.label(),
            finding.selected_by_default,
            finding.scanner_id,
            finding.title,
            action
        );
    }
    Ok(())
}

//! Read-only checks against the real machine. Prints what each part of the crate finds.
//!
//! `cargo run -p hm-system --example probe -- [fda|clones|scanners|monitor|apps|startup|space]`
//!
//! Touches nothing: scanners are read-only by contract, and the safety policy check below only
//! stats paths. The cleaner is never called, not even for a dry run.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use hm_core::fmt::human_size;
use hm_core::model::dedupe_findings;
use hm_core::{fsutil, run_scan, Action, CancelToken, Risk, SafetyPolicy, ScanContext};
use hm_system::{apps, health, monitor::Monitor, scanners, space, startup};

fn main() -> Result<()> {
    let what = std::env::args().nth(1).unwrap_or_else(|| "all".to_string());
    let ctx = ScanContext::for_current_user()?;
    let wants = |section: &str| what == "all" || what == section;

    println!(
        "Full Disk Access: {}",
        fsutil::has_full_disk_access(&ctx.home)
    );
    if wants("clones") {
        clones()?;
    }
    if wants("scanners") {
        run_scanners(&ctx);
    }
    if what == "details" {
        run_details(&ctx);
    }
    if wants("monitor") {
        run_monitor();
    }
    // Takes over a minute, so only on request.
    if what == "load" {
        run_load();
    }
    if wants("apps") {
        run_apps(&ctx)?;
    }
    if wants("startup") {
        run_startup(&ctx)?;
    }
    if wants("space") {
        run_space(&ctx)?;
    }
    Ok(())
}

/// The duplicates scanner relies on how APFS reports clones. Checked in a temporary folder.
fn clones() -> Result<()> {
    println!("\n== Clones (temp files only)");
    let dir = std::env::temp_dir().join(format!("hm-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let original = dir.join("original.bin");
    let clone = dir.join("clone.bin");
    let copy = dir.join("copy.bin");
    std::fs::write(&original, vec![7u8; 5_000_000])?;
    std::fs::copy(&original, &clone)?;
    std::fs::write(&copy, vec![7u8; 5_000_000])?;
    for path in [&original, &clone, &copy] {
        println!(
            "  {:<12} clone_id={:?} private_size={:?}",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            fsutil::clone_id(path),
            fsutil::private_size(path),
        );
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

fn run_scanners(ctx: &ScanContext) {
    println!("\n== Scanners, one at a time");
    let all = scanners();
    let mut raw = Vec::new();
    for scanner in &all {
        let started = Instant::now();
        match scanner.scan(ctx) {
            Ok(findings) => {
                let total: u64 = findings.iter().map(|f| f.size).sum();
                println!(
                    "\n{} ({}): {} hallazgos, {} en {:.2?}",
                    scanner.id(),
                    scanner.name(),
                    findings.len(),
                    human_size(total),
                    started.elapsed()
                );
                let mut sorted = findings.clone();
                sorted.sort_by_key(|f| std::cmp::Reverse(f.size));
                for finding in sorted.iter().take(12) {
                    println!(
                        "  {:>9}  {:?}/{:?} sel={} blocked={:?}  {}",
                        human_size(finding.size),
                        finding.risk,
                        finding.recovery,
                        finding.selected_by_default,
                        finding.blocked_by,
                        finding.title
                    );
                }
                if sorted.len() > 12 {
                    println!("  ... y {} más", sorted.len() - 12);
                }
                raw.extend(findings);
            }
            Err(error) => println!("\n{}: ERROR {error:#}", scanner.id()),
        }
    }

    println!("\n== Rules, across every finding");
    let mut review_selected = 0;
    let mut no_explanation = 0;
    let mut report_with_size = 0;
    for finding in &raw {
        review_selected += usize::from(finding.risk == Risk::Review && finding.selected_by_default);
        no_explanation += usize::from(finding.explanation.is_empty());
        report_with_size += usize::from(finding.action == Action::ReportOnly && finding.size > 0);
    }
    println!("  Review preseleccionados: {review_selected}");
    println!("  Sin explicación: {no_explanation}");
    println!("  Informativos con tamaño: {report_with_size}");

    // Would the policy let these through? Only stats the paths.
    let policy = SafetyPolicy::new(&ctx.home, &ctx.config.excluded_paths);
    let mut checked = 0;
    let mut refused: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for finding in &raw {
        for path in finding.target_paths() {
            checked += 1;
            if let Err(violation) = policy.check_removal(path) {
                refused
                    .entry(finding.scanner_id.clone())
                    .or_default()
                    .push(violation.to_string());
            }
        }
    }
    println!("  Rutas validadas contra SafetyPolicy: {checked}");
    println!(
        "  Rechazadas: {}",
        refused.values().map(Vec::len).sum::<usize>()
    );
    for (scanner, violations) in &refused {
        for violation in violations.iter().take(5) {
            println!("    {scanner}: {violation}");
        }
    }

    println!("\n== Run together, as the app does");
    let before = raw.len();
    let started = Instant::now();
    let together = run_scan(&all, ctx, &|_| {});
    let elapsed = started.elapsed();
    println!(
        "  {} hallazgos en {elapsed:.2?} (por separado {before}, deduplicados {})",
        together.len(),
        dedupe_findings(raw.clone()).len()
    );
    println!(
        "  Recuperable sin decidir nada: {}",
        human_size(health::reclaimable_bytes(&together))
    );
    let selected: u64 = together
        .iter()
        .filter(|f| f.selected_by_default)
        .map(|f| f.size)
        .sum();
    println!("  Preseleccionado: {}", human_size(selected));
}

/// Every path behind the findings where a wrong call would hurt most.
fn run_details(ctx: &ScanContext) {
    for scanner in scanners() {
        if !matches!(
            scanner.id(),
            "apps.leftovers" | "files.duplicates" | "sys.caches"
        ) {
            continue;
        }
        println!("\n== {}", scanner.id());
        let Ok(mut findings) = scanner.scan(ctx) else {
            continue;
        };
        findings.sort_by_key(|f| std::cmp::Reverse(f.size));
        for finding in &findings {
            println!("{}  {}", human_size(finding.size), finding.title);
            if scanner.id() == "sys.caches" && !finding.id.ends_with("pequenas") {
                if finding.id.ends_with("protegidas") {
                    println!("    {}", finding.explanation);
                }
                continue;
            }
            if let Some(subtitle) = &finding.subtitle {
                println!("    {subtitle}");
            }
            let targets = finding.target_paths();
            for path in &targets {
                println!("    - {}", path.display());
            }
        }
    }
}

fn run_monitor() {
    println!("\n== Monitor");
    let started = Instant::now();
    let mut monitor = Monitor::new();
    println!("  Monitor::new: {:.2?}", started.elapsed());
    let started = Instant::now();
    monitor.refresh();
    println!("  Primer refresh (todo frío): {:.2?}", started.elapsed());

    let mut timings = Vec::new();
    let mut snapshot = monitor.refresh();
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(500));
        let started = Instant::now();
        snapshot = monitor.refresh();
        timings.push(started.elapsed());
    }
    println!("  Refresh en caliente: {timings:.1?}");

    let cpu = &snapshot.cpu;
    println!(
        "  CPU: {} {:.1} %, load {:?}",
        cpu.brand, cpu.usage_percent, cpu.load_average
    );
    println!(
        "  Núcleos: {} rendimiento, {} eficiencia, clusters {:?}, {} por núcleo",
        cpu.performance_cores,
        cpu.efficiency_cores,
        cpu.core_clusters,
        cpu.per_core_percent.len()
    );
    let memory = &snapshot.memory;
    println!(
        "  Memoria: total {} usada {} (apps {} wired {} comprimida {})",
        human_size(memory.total),
        human_size(memory.used),
        human_size(memory.app),
        human_size(memory.wired),
        human_size(memory.compressed),
    );
    println!(
        "  Swap: {} de {}, presión {:?}, límite GPU {:?}",
        human_size(memory.swap_used),
        human_size(memory.swap_total),
        memory.pressure,
        memory.gpu_wired_limit.map(human_size)
    );
    for disk in &snapshot.disks {
        println!(
            "  Disco {:?} en {}: {} libres de {} ({}, extraíble {})",
            disk.name,
            disk.mount_point.display(),
            human_size(disk.available),
            human_size(disk.total),
            disk.file_system,
            disk.is_removable
        );
    }
    println!("  Batería: {:?}", snapshot.battery);
    println!("  Temperatura: {:?}", snapshot.thermal);
    println!("  Red: {:?}", snapshot.network);
    println!("  Uptime: {} s", snapshot.uptime_secs);
    println!("  Top CPU:");
    for process in snapshot.top_by_cpu.iter().take(4) {
        println!("    {:>6.1} %  {}", process.cpu_percent, process.name);
    }
    println!("  Top memoria:");
    for process in snapshot.top_by_memory.iter().take(4) {
        println!(
            "    {:>9}  {}",
            human_size(process.memory_bytes),
            process.name
        );
    }

    let report = health::compute(&snapshot, 12_000_000_000);
    println!("\n== Salud (con 12 GB recuperables de ejemplo)");
    println!("  {} / 100: {}", report.score, report.label);
    for factor in &report.factors {
        println!(
            "  -{:<2} {:?} {}: {}",
            factor.penalty, factor.severity, factor.title, factor.detail
        );
    }
}

/// CPU time so far in seconds, for this process or for the children it has waited on.
fn cpu_seconds(who: libc::c_int) -> f64 {
    // SAFETY: all integers, and `getrusage` only writes to `usage`.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `usage` is valid and writable.
    unsafe { libc::getrusage(who, &raw mut usage) };
    let seconds = |t: libc::timeval| t.tv_sec as f64 + f64::from(t.tv_usec) / 1e6;
    seconds(usage.ru_utime) + seconds(usage.ru_stime)
}

/// What leaving the monitor on actually costs. Long enough to cross the 30 s battery refresh twice.
fn run_load() {
    println!("\n== Monitor bajo carga: 65 refrescos, uno por segundo");
    let mut monitor = Monitor::new();
    monitor.refresh();
    let cpu_before = cpu_seconds(libc::RUSAGE_SELF);
    let children_before = cpu_seconds(libc::RUSAGE_CHILDREN);
    let wall = Instant::now();
    let mut timings = Vec::new();
    for _ in 0..65 {
        std::thread::sleep(Duration::from_secs(1));
        let started = Instant::now();
        monitor.refresh();
        timings.push(started.elapsed());
    }
    let elapsed = wall.elapsed().as_secs_f64();
    let own = cpu_seconds(libc::RUSAGE_SELF) - cpu_before;
    let children = cpu_seconds(libc::RUSAGE_CHILDREN) - children_before;
    timings.sort();
    println!(
        "  Mediana {:.1?}, p90 {:.1?}, peor {:.1?}",
        timings[timings.len() / 2],
        timings[timings.len() * 9 / 10],
        timings[timings.len() - 1]
    );
    println!(
        "  CPU: {:.2} % de un núcleo, más {:.2} % en procesos hijos (pmset, ioreg, system_profiler)",
        own / elapsed * 100.0,
        children / elapsed * 100.0
    );
}

fn run_apps(ctx: &ScanContext) -> Result<()> {
    println!("\n== Apps");
    let started = Instant::now();
    let list = apps::list_apps(ctx)?;
    println!("  {} apps en {:.2?}", list.len(), started.elapsed());

    let mut archs: BTreeMap<String, usize> = BTreeMap::new();
    for app in &list {
        *archs.entry(format!("{:?}", app.arch)).or_default() += 1;
    }
    println!("  Arquitecturas: {archs:?}");
    let count =
        |test: &dyn Fn(&hm_system::types::AppInfo) -> bool| list.iter().filter(|a| test(a)).count();
    println!(
        "  En marcha {}, de sistema {}, con fecha de uso {}, con icono {}, sin bundle id {}, con restos {}",
        count(&|a| a.running),
        count(&|a| a.is_system),
        count(&|a| a.last_used.is_some()),
        count(&|a| a.icon_path.is_some()),
        count(&|a| a.bundle_id.is_none()),
        count(&|a| !a.leftovers.is_empty()),
    );
    for app in list.iter().take(6) {
        let leftovers: u64 = app.leftovers.iter().map(|l| l.size).sum();
        println!(
            "  {:>9}  {} {} [{:?}] exe={:?} datos={} running={}",
            human_size(app.size),
            app.name,
            app.version.as_deref().unwrap_or("?"),
            app.arch,
            app.executable,
            human_size(leftovers),
            app.running
        );
    }
    println!(
        "  De sistema: {:?}",
        list.iter()
            .filter(|a| a.is_system)
            .map(|a| &a.name)
            .collect::<Vec<_>>()
    );
    println!(
        "  Apple pero desinstalables: {:?}",
        list.iter()
            .filter(|a| !a.is_system
                && a.bundle_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with("com.apple.")))
            .map(|a| &a.name)
            .collect::<Vec<_>>()
    );
    println!(
        "  Intel: {:?}",
        list.iter()
            .filter(|a| a.arch == hm_system::types::AppArch::Intel)
            .map(|a| &a.name)
            .collect::<Vec<_>>()
    );
    println!(
        "  Arquitectura desconocida: {:?}",
        list.iter()
            .filter(|a| a.arch == hm_system::types::AppArch::Unknown)
            .map(|a| &a.name)
            .collect::<Vec<_>>()
    );

    // Whatever the app says can be uninstalled must pass the policy. Builds findings and stats paths.
    let policy = SafetyPolicy::new(&ctx.home, &ctx.config.excluded_paths);
    let blocked = list
        .iter()
        .filter(|a| a.uninstall_blocked.is_some())
        .count();
    let mut offered = 0;
    let mut refused = Vec::new();
    for app in list.iter().filter(|a| a.uninstall_blocked.is_none()) {
        offered += 1;
        let finding = apps::uninstall_finding(app, true);
        for path in finding.target_paths() {
            if let Err(violation) = policy.check_removal(path) {
                refused.push(violation.to_string());
            }
        }
    }
    println!(
        "  Desinstalables {offered}, bloqueadas {blocked}, rutas rechazadas {}",
        refused.len()
    );
    for violation in refused.iter().take(8) {
        println!("    {violation}");
    }
    let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
    for reason in list.iter().filter_map(|a| a.uninstall_blocked.as_deref()) {
        let kind = if reason.contains("macOS") {
            "sistema"
        } else if reason.contains("fabricante") {
            "carpeta del fabricante"
        } else if reason.contains("administrador") {
            "propiedad de root"
        } else {
            "excluida"
        };
        *reasons.entry(kind).or_default() += 1;
    }
    println!("  Motivos: {reasons:?}");
    Ok(())
}

fn run_startup(ctx: &ScanContext) -> Result<()> {
    println!("\n== Arranque");
    let started = Instant::now();
    let items = startup::list_items(ctx)?;
    println!("  {} elementos en {:.2?}", items.len(), started.elapsed());
    let count = |test: &dyn Fn(&hm_system::types::StartupItem) -> bool| {
        items.iter().filter(|i| test(i)).count()
    };
    println!(
        "  Desactivados {}, huérfanos {}, malformados {}, RunAtLoad {}, KeepAlive {}",
        count(&|i| i.disabled),
        count(&|i| i.orphaned),
        count(&|i| i.malformed),
        count(&|i| i.run_at_load),
        count(&|i| i.keep_alive),
    );
    for item in &items {
        if item.orphaned || item.disabled || item.malformed {
            println!(
                "  {} [{:?}] disabled={} orphaned={} malformed={} program={:?}",
                item.label, item.kind, item.disabled, item.orphaned, item.malformed, item.program
            );
        }
    }
    Ok(())
}

fn print_tree(node: &hm_system::types::DirNode, depth: usize, limit: usize) {
    println!(
        "  {}{:>9}  {}{}",
        "  ".repeat(depth),
        human_size(node.size),
        node.name,
        if node.partial { " (parcial)" } else { "" }
    );
    for child in node.children.iter().take(limit) {
        print_tree(child, depth + 1, limit);
    }
}

fn run_space(ctx: &ScanContext) -> Result<()> {
    println!("\n== Mapa de disco");
    let cancel = CancelToken::new();
    for root in [ctx.home_path("Library/Caches"), ctx.home.clone()] {
        let started = Instant::now();
        let tree = space::scan_tree(&root, 2, 500_000_000, &cancel)?;
        println!(
            "  {}: {} en {} archivos, parcial={}, {:.2?}",
            root.display(),
            human_size(tree.size),
            tree.file_count,
            tree.partial,
            started.elapsed()
        );
        print_tree(&tree, 0, 8);
    }
    // Cancellation has to stop a large walk quickly, or the UI's cancel button is a lie.
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        trigger.cancel();
    });
    let started = Instant::now();
    let tree = space::scan_tree(Path::new(&ctx.home), 2, 500_000_000, &cancel)?;
    let _ = canceller.join();
    println!(
        "  Cancelado a los 50 ms: terminó en {:.2?}, parcial={}",
        started.elapsed(),
        tree.partial
    );
    Ok(())
}

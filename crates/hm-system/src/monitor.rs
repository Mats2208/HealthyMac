//! Live system metrics.

use std::ffi::{c_char, c_int, CString};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use hm_core::exec;
use plist::Value;
use sysinfo::{Disks, Networks, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::types::*;

const BATTERY_TTL: Duration = Duration::from_secs(30);
/// Maximum capacity and condition barely change, and `system_profiler` is the slowest thing here.
const BATTERY_HEALTH_TTL: Duration = Duration::from_secs(600);
const THERMAL_TTL: Duration = Duration::from_secs(5);
const DISK_TTL: Duration = Duration::from_secs(10);
/// Someone may raise the limit with `sysctl` while the app is open.
const GPU_LIMIT_TTL: Duration = Duration::from_secs(30);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const TOP_PROCESSES: usize = 10;
/// macOS gives the GPU about 3/4 of RAM above this, and about 2/3 at or below it.
const GPU_LIMIT_RAM_BOUNDARY: u64 = 36 * 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

/// A value that is expensive to read, kept until it goes stale.
struct Timed<T> {
    value: T,
    read_at: Option<Instant>,
}

impl<T: Default> Default for Timed<T> {
    fn default() -> Self {
        Self {
            value: T::default(),
            read_at: None,
        }
    }
}

impl<T> Timed<T> {
    fn get(&mut self, ttl: Duration, read: impl FnOnce() -> T) -> &T {
        if self.read_at.is_none_or(|at| at.elapsed() >= ttl) {
            self.value = read();
            self.read_at = Some(Instant::now());
        }
        &self.value
    }
}

/// Keeps the state needed to compute rates between refreshes. Create one and call
/// [`Monitor::refresh`] about once a second from a background thread.
///
/// CPU use and network rates need two samples, so they read 0 in the first snapshot. Everything
/// else is right from the start.
pub struct Monitor {
    system: Option<System>,
    networks: Option<Networks>,
    network_sample: Option<NetworkSample>,
    clusters: Vec<CoreCluster>,
    thermal_token: Option<c_int>,
    disks: Timed<Vec<DiskStats>>,
    battery: Timed<Option<BatteryLive>>,
    battery_health: Timed<BatteryHealth>,
    thermal: Timed<ThermalState>,
    gpu_limit: Timed<Option<u64>>,
}

#[derive(Clone, Copy)]
struct NetworkSample {
    at: Instant,
    received: u64,
    transmitted: u64,
}

impl Monitor {
    pub fn new() -> Self {
        let mut system = System::new().ok();
        if let Some(system) = &mut system {
            // Baselines, so the first real refresh already has something to compare with.
            system.refresh_cpu_usage();
            system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_cpu().with_memory(),
            );
        }
        Self {
            system,
            networks: Networks::new_with_refreshed_list().ok(),
            network_sample: None,
            clusters: read_core_clusters(),
            thermal_token: register_thermal_notification(),
            disks: Timed::default(),
            battery: Timed::default(),
            battery_health: Timed::default(),
            thermal: Timed::default(),
            gpu_limit: Timed::default(),
        }
    }

    /// Cheap enough to call once a second: slow sources are cached, and only the rare refresh that
    /// finds one stale pays for reading it.
    pub fn refresh(&mut self) -> SystemSnapshot {
        let network = self.refresh_network();
        let thermal_token = self.thermal_token;
        let memory = self.read_memory();
        SystemSnapshot {
            taken_at: SystemTime::now(),
            cpu: self.read_cpu(),
            memory,
            disks: self.disks.get(DISK_TTL, read_disks).clone(),
            battery: self.read_battery(),
            thermal: *self
                .thermal
                .get(THERMAL_TTL, || read_thermal_state(thermal_token)),
            network,
            uptime_secs: System::uptime().unwrap_or(0),
            top_by_cpu: Vec::new(),
            top_by_memory: Vec::new(),
        }
        .with_top_processes(self.read_processes())
    }

    fn read_cpu(&mut self) -> CpuStats {
        let load = System::load_average().ok();
        let mut stats = CpuStats {
            performance_cores: self.clusters.first().map_or(0, |c| c.logical_cpus),
            efficiency_cores: self.clusters.iter().skip(1).map(|c| c.logical_cpus).sum(),
            core_clusters: self.clusters.clone(),
            load_average: load.map_or([0.0; 3], |load| [load.one, load.five, load.fifteen]),
            ..CpuStats::default()
        };
        if let Some(system) = &mut self.system {
            system.refresh_cpu_usage();
            stats.usage_percent = system.global_cpu_usage();
            stats.per_core_percent = system.cpus().iter().map(|cpu| cpu.usage()).collect();
            stats.brand = system
                .cpus()
                .first()
                .map(|cpu| cpu.brand().to_string())
                .unwrap_or_default();
        }
        stats
    }

    fn read_memory(&mut self) -> MemoryStats {
        let mut stats = MemoryStats::default();
        if let Some(system) = &mut self.system {
            system.refresh_memory();
            stats.total = system.total_memory();
            stats.used = system.used_memory();
            stats.swap_total = system.total_swap();
            stats.swap_used = system.used_swap();
        }
        if stats.total == 0 {
            stats.total = sysctl_u64("hw.memsize").unwrap_or(0);
        }
        // When this fails, `used` keeps sysinfo's figure and the breakdown stays at 0 rather than
        // being made up.
        if let Some(breakdown) = read_vm_breakdown() {
            stats.app = breakdown.app;
            stats.wired = breakdown.wired;
            stats.compressed = breakdown.compressed;
            stats.used = breakdown.app + breakdown.wired + breakdown.compressed;
        }
        stats.pressure = sysctl_u64("kern.memorystatus_vm_pressure_level")
            .map_or(MemoryPressure::Normal, pressure_from_level);
        let total = stats.total;
        stats.gpu_wired_limit = *self.gpu_limit.get(GPU_LIMIT_TTL, || {
            sysctl_u64("iogpu.wired_limit_mb").map(|mb| gpu_limit_bytes(mb, total))
        });
        stats
    }

    fn read_processes(&mut self) -> Vec<ProcessStats> {
        let Some(system) = &mut self.system else {
            return Vec::new();
        };
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        system
            .processes()
            .values()
            .map(|process| {
                let pid = process.pid().as_u32();
                ProcessStats {
                    pid,
                    name: process.name().to_string_lossy().into_owned(),
                    cpu_percent: process.cpu_usage(),
                    // Resident size misses GPU-wired memory, so a local model that holds 30 GB
                    // would look tiny. The footprint is what Activity Monitor shows.
                    memory_bytes: phys_footprint(pid).unwrap_or_else(|| process.memory()),
                }
            })
            .collect()
    }

    fn refresh_network(&mut self) -> NetworkStats {
        let Some(networks) = &mut self.networks else {
            return NetworkStats::default();
        };
        networks.refresh(true);
        let (mut received, mut transmitted) = (0u64, 0u64);
        for (name, data) in networks.iter() {
            if is_physical_interface(&name.to_string_lossy()) {
                received = received.saturating_add(data.total_received());
                transmitted = transmitted.saturating_add(data.total_transmitted());
            }
        }
        let now = Instant::now();
        let stats = match self.network_sample {
            Some(previous) => {
                let elapsed = now.duration_since(previous.at);
                NetworkStats {
                    rx_bytes_per_sec: bytes_per_second(previous.received, received, elapsed),
                    tx_bytes_per_sec: bytes_per_second(previous.transmitted, transmitted, elapsed),
                }
            }
            None => NetworkStats::default(),
        };
        self.network_sample = Some(NetworkSample {
            at: now,
            received,
            transmitted,
        });
        stats
    }

    fn read_battery(&mut self) -> Option<BatteryStats> {
        let live = self.battery.get(BATTERY_TTL, read_battery_live).clone()?;
        // No point asking `system_profiler` about the battery of a Mac that has none.
        let health = self
            .battery_health
            .get(BATTERY_HEALTH_TTL, read_battery_health)
            .clone();
        Some(BatteryStats {
            percent: live.percent,
            charging: live.charging,
            plugged_in: live.plugged_in,
            minutes_remaining: live.minutes_remaining,
            cycle_count: live.cycle_count.or(health.cycle_count),
            health_percent: health.maximum_capacity.or(live.computed_health),
            condition: health.condition,
        })
    }
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemSnapshot {
    fn with_top_processes(mut self, mut processes: Vec<ProcessStats>) -> Self {
        processes.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
        self.top_by_cpu = processes.iter().take(TOP_PROCESSES).cloned().collect();
        processes.sort_by_key(|process| std::cmp::Reverse(process.memory_bytes));
        processes.truncate(TOP_PROCESSES);
        self.top_by_memory = processes;
        self
    }
}

// ---- sysctl -------------------------------------------------------------------------------------

/// Reads a numeric sysctl of any width. `None` if it does not exist on this Mac.
fn sysctl_u64(name: &str) -> Option<u64> {
    let c_name = CString::new(name).ok()?;
    let mut bytes = [0u8; 8];
    let mut len = bytes.len();
    // SAFETY: `c_name` is NUL-terminated, and `bytes` is writable for the `len` bytes passed.
    let result = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            &raw mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0).then(|| u64::from_ne_bytes(bytes))
}

fn sysctl_string(name: &str) -> Option<String> {
    let c_name = CString::new(name).ok()?;
    let mut bytes = [0u8; 128];
    let mut len = bytes.len();
    // SAFETY: as in `sysctl_u64`.
    let result = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            &raw mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return None;
    }
    let text = bytes.get(..len)?;
    let end = text
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(text.len());
    String::from_utf8(text[..end].to_vec()).ok()
}

fn read_core_clusters() -> Vec<CoreCluster> {
    let levels = sysctl_u64("hw.nperflevels").unwrap_or(0);
    let clusters: Vec<CoreCluster> = (0..levels)
        .filter_map(|level| {
            Some(CoreCluster {
                name: sysctl_string(&format!("hw.perflevel{level}.name")).unwrap_or_default(),
                logical_cpus: sysctl_u64(&format!("hw.perflevel{level}.logicalcpu"))
                    .and_then(|count| u32::try_from(count).ok())?,
            })
        })
        .collect();
    if !clusters.is_empty() {
        return clusters;
    }
    // Intel Macs have no performance levels: every core is the same.
    sysctl_u64("hw.logicalcpu")
        .and_then(|count| u32::try_from(count).ok())
        .map(|logical_cpus| {
            vec![CoreCluster {
                name: String::new(),
                logical_cpus,
            }]
        })
        .unwrap_or_default()
}

fn pressure_from_level(level: u64) -> MemoryPressure {
    match level {
        2 => MemoryPressure::Warning,
        4 => MemoryPressure::Critical,
        // 1 is normal. Anything undocumented is not worth alarming anyone over.
        _ => MemoryPressure::Normal,
    }
}

/// A set limit wins. At 0 macOS uses its default, which this approximates: the real figure is only
/// available from Metal, which this crate does not link.
fn gpu_limit_bytes(limit_mb: u64, total_ram: u64) -> u64 {
    if limit_mb > 0 {
        return limit_mb.saturating_mul(MIB);
    }
    // Metal knows the real figure. The fractions are only for when it can't be asked.
    hm_core::gpu::recommended_working_set().unwrap_or_else(|| gpu_limit_fallback(total_ram))
}

fn gpu_limit_fallback(total_ram: u64) -> u64 {
    if total_ram > GPU_LIMIT_RAM_BOUNDARY {
        total_ram / 4 * 3
    } else {
        total_ram / 3 * 2
    }
}

// ---- Memory breakdown ---------------------------------------------------------------------------

struct VmBreakdown {
    app: u64,
    wired: u64,
    compressed: u64,
}

fn read_vm_breakdown() -> Option<VmBreakdown> {
    // SAFETY: `sysconf` has no preconditions.
    let page_size = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    // SAFETY: an all-zero `vm_statistics64` is valid, since it is only integers.
    let mut stats: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // libc points to the `mach2` crate for this call. Not worth a dependency for one host port.
    #[allow(deprecated)]
    // SAFETY: `stats` is writable and `count` says how many integers fit in it.
    let result = unsafe {
        libc::host_statistics64(
            libc::mach_host_self(),
            libc::HOST_VM_INFO64,
            (&raw mut stats).cast(),
            &raw mut count,
        )
    };
    (result == libc::KERN_SUCCESS).then(|| vm_breakdown_from(&stats, page_size))
}

/// The same arithmetic as Activity Monitor: app memory is anonymous memory that cannot simply be
/// dropped, wired cannot be paged out, and "used" is those plus the compressor. File-backed and
/// purgeable pages are cache, and macOS gives those up as soon as anything needs them.
fn vm_breakdown_from(stats: &libc::vm_statistics64, page_size: u64) -> VmBreakdown {
    let pages = |count: u32| u64::from(count) * page_size;
    VmBreakdown {
        app: pages(
            stats
                .internal_page_count
                .saturating_sub(stats.purgeable_count),
        ),
        wired: pages(stats.wire_count),
        compressed: pages(stats.compressor_page_count),
    }
}

/// `None` for processes of other users, which `proc_pid_rusage` refuses.
fn phys_footprint(pid: u32) -> Option<u64> {
    let pid = c_int::try_from(pid).ok()?;
    // SAFETY: an all-zero `rusage_info_v4` is valid, since it is only integers.
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is exactly the struct the V4 flavor writes.
    let result =
        unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, (&raw mut info).cast()) };
    (result == 0).then_some(info.ri_phys_footprint)
}

// ---- Thermal ------------------------------------------------------------------------------------

/// What `NSProcessInfo.thermalState` is built on, from `<libkern/OSThermalNotification.h>`.
const THERMAL_NOTIFICATION: &str = "com.apple.system.thermalpressurelevel";

// `libc` does not export the notify API. It is part of libSystem, so there is nothing to link.
extern "C" {
    fn notify_register_check(name: *const c_char, out_token: *mut c_int) -> u32;
    fn notify_get_state(token: c_int, state: *mut u64) -> u32;
}

fn register_thermal_notification() -> Option<c_int> {
    let name = CString::new(THERMAL_NOTIFICATION).ok()?;
    let mut token = 0;
    // SAFETY: `name` is NUL-terminated and `token` is writable.
    let status = unsafe { notify_register_check(name.as_ptr(), &raw mut token) };
    (status == 0).then_some(token)
}

fn read_thermal_state(token: Option<c_int>) -> ThermalState {
    let from_notification = token.and_then(|token| {
        let mut state = 0u64;
        // SAFETY: `state` is writable, and a bad token just returns an error status.
        let status = unsafe { notify_get_state(token, &raw mut state) };
        (status == 0).then(|| thermal_from_pressure_level(state))
    });
    from_notification.unwrap_or_else(read_thermal_from_pmset)
}

fn thermal_from_pressure_level(level: u64) -> ThermalState {
    match level {
        0 => ThermalState::Nominal,
        1 => ThermalState::Fair,
        2 => ThermalState::Serious,
        3 | 4 => ThermalState::Critical,
        _ => ThermalState::Unknown,
    }
}

fn read_thermal_from_pmset() -> ThermalState {
    exec::run(
        Path::new("/usr/bin/pmset"),
        &["-g", "therm"],
        COMMAND_TIMEOUT,
    )
    .ok()
    .filter(|output| output.success())
    .map_or(ThermalState::Unknown, |output| {
        parse_pmset_therm(&output.stdout)
    })
}

fn parse_pmset_therm(output: &str) -> ThermalState {
    // Intel Macs report how far the CPU is being held back.
    let speed_limit = output.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "CPU_Speed_Limit")
            .then(|| value.trim().parse::<u32>().ok())
            .flatten()
    });
    match speed_limit {
        Some(100..) => ThermalState::Nominal,
        Some(70..=99) => ThermalState::Fair,
        Some(40..=69) => ThermalState::Serious,
        Some(_) => ThermalState::Critical,
        None if output.contains("No thermal warning level has been recorded") => {
            ThermalState::Nominal
        }
        None => ThermalState::Unknown,
    }
}

// ---- Battery ------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BatteryLive {
    percent: u8,
    charging: bool,
    plugged_in: bool,
    minutes_remaining: Option<u32>,
    cycle_count: Option<u32>,
    /// Only a fallback: on the Mac this was written on it says 90 % while macOS reports 92 %.
    computed_health: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BatteryHealth {
    maximum_capacity: Option<u8>,
    condition: Option<String>,
    cycle_count: Option<u32>,
}

fn read_battery_live() -> Option<BatteryLive> {
    let pmset = exec::run(
        Path::new("/usr/bin/pmset"),
        &["-g", "batt"],
        COMMAND_TIMEOUT,
    )
    .ok()?;
    let mut live = parse_pmset_batt(&pmset.stdout)?;
    let ioreg = exec::run(
        Path::new("/usr/sbin/ioreg"),
        &["-rn", "AppleSmartBattery", "-a"],
        COMMAND_TIMEOUT,
    );
    if let Ok(ioreg) = ioreg {
        let (cycles, health) = parse_ioreg_battery(ioreg.stdout.as_bytes());
        live.cycle_count = cycles;
        live.computed_health = health;
    }
    Some(live)
}

/// `None` means this Mac has no battery.
fn parse_pmset_batt(output: &str) -> Option<BatteryLive> {
    let line = output
        .lines()
        .find(|line| line.contains("InternalBattery"))?;
    let (before, after) = line.split_once('%')?;
    let digits = before.rsplit(|c: char| !c.is_ascii_digit()).next()?;
    let percent = digits.parse::<u8>().ok()?;
    let status = after.to_ascii_lowercase();
    // Order matters: "discharging" and "not charging" both contain "charging".
    let charging = !status.contains("discharging")
        && !status.contains("not charging")
        && !status.contains("; charged")
        && status.contains("charging");
    Some(BatteryLive {
        percent,
        charging,
        plugged_in: output.contains("'AC Power'"),
        minutes_remaining: parse_time_remaining(&status),
        ..BatteryLive::default()
    })
}

/// `2:05 remaining` to 125. macOS reports `0:00` and `(no estimate)` while it works it out, and
/// neither means "no time left".
fn parse_time_remaining(status: &str) -> Option<u32> {
    let token = status.split_whitespace().find(|token| {
        token.contains(':') && token.chars().next().is_some_and(|c| c.is_ascii_digit())
    })?;
    let (hours, minutes) = token.split_once(':')?;
    let minutes = hours
        .parse::<u32>()
        .ok()?
        .checked_mul(60)?
        .checked_add(minutes.parse::<u32>().ok()?)?;
    (minutes > 0).then_some(minutes)
}

fn parse_ioreg_battery(output: &[u8]) -> (Option<u32>, Option<u8>) {
    let Ok(value) = Value::from_reader(std::io::Cursor::new(output)) else {
        return (None, None);
    };
    let Some(dict) = value
        .as_array()
        .and_then(|array| array.first())
        .and_then(Value::as_dictionary)
    else {
        return (None, None);
    };
    let nested = dict.get("BatteryData").and_then(Value::as_dictionary);
    let number = |key: &str| {
        dict.get(key)
            .or_else(|| nested.and_then(|nested| nested.get(key)))
            .and_then(Value::as_unsigned_integer)
    };
    let cycles = number("CycleCount").and_then(|count| u32::try_from(count).ok());
    let health = number("NominalChargeCapacity")
        .or_else(|| number("AppleRawMaxCapacity"))
        .zip(number("DesignCapacity").filter(|design| *design > 0))
        .map(|(full, design)| (full.saturating_mul(100) / design).min(100) as u8);
    (cycles, health)
}

fn read_battery_health() -> BatteryHealth {
    exec::run(
        Path::new("/usr/sbin/system_profiler"),
        &["SPPowerDataType"],
        COMMAND_TIMEOUT,
    )
    .ok()
    .filter(|output| output.success())
    .map(|output| parse_system_profiler_power(&output.stdout))
    .unwrap_or_default()
}

/// The figures macOS itself shows in Settings > Battery.
fn parse_system_profiler_power(output: &str) -> BatteryHealth {
    let field = |label: &str| {
        output.lines().find_map(|line| {
            line.trim()
                .strip_prefix(label)?
                .strip_prefix(':')
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
    };
    BatteryHealth {
        maximum_capacity: field("Maximum Capacity")
            .and_then(|value| value.trim_end_matches('%').parse().ok()),
        condition: field("Condition"),
        cycle_count: field("Cycle Count").and_then(|value| value.parse().ok()),
    }
}

// ---- Disks and network --------------------------------------------------------------------------

fn read_disks() -> Vec<DiskStats> {
    let Ok(disks) = Disks::new_with_refreshed_list() else {
        return Vec::new();
    };
    disks
        .iter()
        .filter(|disk| is_user_volume(disk.mount_point()))
        .map(|disk| DiskStats {
            name: disk.name().to_string_lossy().into_owned(),
            mount_point: disk.mount_point().to_path_buf(),
            total: disk.total_space(),
            available: disk.available_space(),
            is_removable: disk.is_removable(),
            file_system: disk.file_system().to_string_lossy().into_owned(),
        })
        .collect()
}

/// `/` plus whatever the user mounted. The rest of `/System/Volumes` is plumbing that shares `/`'s
/// container, and listing it would just repeat the same free space several times.
fn is_user_volume(mount_point: &Path) -> bool {
    mount_point == Path::new("/") || mount_point.starts_with("/Volumes")
}

/// Counts only physical links. Traffic through a VPN (`utun`) or a VM bridge also crosses `en0`, so
/// including those would count it twice.
fn is_physical_interface(name: &str) -> bool {
    name.starts_with("en") || name.starts_with("pdp_ip")
}

/// A counter that went backwards, as when an interface goes away, reads as no traffic.
fn bytes_per_second(previous: u64, current: u64, elapsed: Duration) -> u64 {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return 0;
    }
    (current.saturating_sub(previous) as f64 / seconds) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_can_move_to_a_background_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<Monitor>();
    }

    #[test]
    fn timed_value_is_kept_until_it_goes_stale() {
        let mut value: Timed<u32> = Timed::default();
        assert_eq!(*value.get(Duration::from_secs(60), || 1), 1);
        assert_eq!(*value.get(Duration::from_secs(60), || 2), 1);
        assert_eq!(*value.get(Duration::ZERO, || 3), 3);
    }

    #[test]
    fn network_rate_comes_from_the_delta() {
        let second = Duration::from_secs(1);
        assert_eq!(bytes_per_second(1_000, 3_000, second), 2_000);
        assert_eq!(bytes_per_second(1_000, 3_000, second * 2), 1_000);
        assert_eq!(bytes_per_second(5_000, 1_000, second), 0, "counter reset");
        assert_eq!(bytes_per_second(0, 1_000, Duration::ZERO), 0);
    }

    #[test]
    fn only_physical_links_count_as_traffic() {
        for name in ["en0", "en6", "pdp_ip0"] {
            assert!(is_physical_interface(name), "{name}");
        }
        for name in [
            "lo0",
            "utun3",
            "awdl0",
            "llw0",
            "bridge100",
            "anpi0",
            "vmenet0",
        ] {
            assert!(!is_physical_interface(name), "{name}");
        }
    }

    #[test]
    fn plumbing_volumes_are_hidden() {
        assert!(is_user_volume(Path::new("/")));
        assert!(is_user_volume(Path::new("/Volumes/Backup")));
        assert!(!is_user_volume(Path::new("/System/Volumes/Data")));
        assert!(!is_user_volume(Path::new("/System/Volumes/Preboot")));
    }

    #[test]
    fn pressure_levels_map_to_the_documented_values() {
        assert_eq!(pressure_from_level(1), MemoryPressure::Normal);
        assert_eq!(pressure_from_level(2), MemoryPressure::Warning);
        assert_eq!(pressure_from_level(4), MemoryPressure::Critical);
        assert_eq!(pressure_from_level(99), MemoryPressure::Normal);
    }

    #[test]
    fn gpu_limit_prefers_the_sysctl_then_the_macos_default() {
        const GIB: u64 = 1024 * 1024 * 1024;
        assert_eq!(gpu_limit_bytes(40_960, 48 * GIB), 40 * GIB);
        assert_eq!(
            gpu_limit_bytes(0, 48 * GIB),
            hm_core::gpu::recommended_working_set().unwrap_or(36 * GIB)
        );
        assert_eq!(gpu_limit_fallback(48 * GIB), 36 * GIB);
        assert_eq!(gpu_limit_fallback(18 * GIB), 12 * GIB);
    }

    #[test]
    fn memory_breakdown_matches_activity_monitor_arithmetic() {
        // SAFETY: all integers.
        let mut stats: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
        stats.internal_page_count = 100;
        stats.purgeable_count = 10;
        stats.wire_count = 20;
        stats.compressor_page_count = 30;
        let breakdown = vm_breakdown_from(&stats, 16_384);
        assert_eq!(breakdown.app, 90 * 16_384);
        assert_eq!(breakdown.wired, 20 * 16_384);
        assert_eq!(breakdown.compressed, 30 * 16_384);
    }

    #[test]
    fn thermal_levels_map_to_process_info_states() {
        assert_eq!(thermal_from_pressure_level(0), ThermalState::Nominal);
        assert_eq!(thermal_from_pressure_level(1), ThermalState::Fair);
        assert_eq!(thermal_from_pressure_level(2), ThermalState::Serious);
        assert_eq!(thermal_from_pressure_level(4), ThermalState::Critical);
        assert_eq!(thermal_from_pressure_level(77), ThermalState::Unknown);
    }

    #[test]
    fn parses_pmset_therm_on_both_architectures() {
        let apple_silicon = "Note: No thermal warning level has been recorded\n\
            Note: No performance warning level has been recorded\n";
        assert_eq!(parse_pmset_therm(apple_silicon), ThermalState::Nominal);
        let intel =
            "CPU_Scheduler_Limit \t= 100\nCPU_Available_CPUs \t= 8\nCPU_Speed_Limit \t= 55\n";
        assert_eq!(parse_pmset_therm(intel), ThermalState::Serious);
        assert_eq!(parse_pmset_therm(""), ThermalState::Unknown);
    }

    #[test]
    fn parses_pmset_batt_states() {
        let discharging = "Now drawing from 'Battery Power'\n\
            -InternalBattery-0 (id=24510563)\t50%; discharging; 2:05 remaining present: true\n";
        let live = parse_pmset_batt(discharging).expect("battery");
        assert_eq!(live.percent, 50);
        assert!(!live.charging && !live.plugged_in);
        assert_eq!(live.minutes_remaining, Some(125));

        let charging = "Now drawing from 'AC Power'\n\
            -InternalBattery-0 (id=1)\t80%; charging; 0:40 remaining present: true\n";
        let live = parse_pmset_batt(charging).expect("battery");
        assert!(live.charging && live.plugged_in);
        assert_eq!(live.minutes_remaining, Some(40));

        let full = "Now drawing from 'AC Power'\n\
            -InternalBattery-0 (id=1)\t100%; charged; 0:00 remaining present: true\n";
        let live = parse_pmset_batt(full).expect("battery");
        assert!(!live.charging && live.plugged_in);
        assert_eq!(live.percent, 100);
        assert_eq!(live.minutes_remaining, None);

        let held = "Now drawing from 'AC Power'\n\
            -InternalBattery-0 (id=1)\t80%; AC attached; not charging present: true\n";
        let live = parse_pmset_batt(held).expect("battery");
        assert!(!live.charging && live.plugged_in);

        let estimating = "Now drawing from 'Battery Power'\n\
            -InternalBattery-0 (id=1)\t64%; discharging; (no estimate) present: true\n";
        assert_eq!(
            parse_pmset_batt(estimating)
                .expect("battery")
                .minutes_remaining,
            None
        );
    }

    #[test]
    fn odd_battery_lines_do_not_panic() {
        // Multibyte text right before the percentage, and a time too large to represent.
        let accented = "-InternalBattery-0 é50%; discharging; 99999999:00 remaining";
        let live = parse_pmset_batt(accented).expect("battery");
        assert_eq!(live.percent, 50);
        assert_eq!(live.minutes_remaining, None);
        assert_eq!(parse_pmset_batt("-InternalBattery-0 no percent here"), None);
        assert_eq!(parse_pmset_batt("-InternalBattery-0 300%; charging"), None);
    }

    #[test]
    fn a_desktop_mac_has_no_battery() {
        assert_eq!(parse_pmset_batt("Now drawing from 'AC Power'\n"), None);
        assert_eq!(parse_pmset_batt(""), None);
    }

    #[test]
    fn parses_ioreg_with_capacities_nested_or_top_level() {
        let nested = br#"<plist version="1.0"><array><dict>
            <key>CycleCount</key><integer>151</integer>
            <key>BatteryData</key><dict>
              <key>NominalChargeCapacity</key><integer>5618</integer>
              <key>DesignCapacity</key><integer>6249</integer>
            </dict></dict></array></plist>"#;
        assert_eq!(parse_ioreg_battery(nested), (Some(151), Some(89)));

        let top = br#"<plist version="1.0"><array><dict>
            <key>AppleRawMaxCapacity</key><integer>4000</integer>
            <key>DesignCapacity</key><integer>5000</integer>
            </dict></array></plist>"#;
        assert_eq!(parse_ioreg_battery(top), (None, Some(80)));
    }

    #[test]
    fn broken_ioreg_output_is_not_fatal() {
        assert_eq!(parse_ioreg_battery(b"not a plist"), (None, None));
        assert_eq!(parse_ioreg_battery(b""), (None, None));
        let empty = br#"<plist version="1.0"><array/></plist>"#;
        assert_eq!(parse_ioreg_battery(empty), (None, None));
        let zero_design = br#"<plist version="1.0"><array><dict>
            <key>NominalChargeCapacity</key><integer>10</integer>
            <key>DesignCapacity</key><integer>0</integer>
            </dict></array></plist>"#;
        assert_eq!(parse_ioreg_battery(zero_design), (None, None));
    }

    #[test]
    fn parses_system_profiler_power() {
        let output = "Power:\n\n    Battery Information:\n\n      Health Information:\n\
            \n          Cycle Count: 151\n          Condition: Normal\n          Maximum Capacity: 92%\n";
        let health = parse_system_profiler_power(output);
        assert_eq!(health.maximum_capacity, Some(92));
        assert_eq!(health.condition.as_deref(), Some("Normal"));
        assert_eq!(health.cycle_count, Some(151));
        assert_eq!(parse_system_profiler_power("").maximum_capacity, None);
    }

    #[test]
    fn top_lists_are_sorted_and_capped() {
        let processes: Vec<ProcessStats> = (0..30)
            .map(|index| ProcessStats {
                pid: index,
                name: format!("p{index}"),
                cpu_percent: index as f32,
                memory_bytes: u64::from(30 - index),
            })
            .collect();
        let snapshot = SystemSnapshot {
            taken_at: SystemTime::now(),
            cpu: CpuStats::default(),
            memory: MemoryStats::default(),
            disks: Vec::new(),
            battery: None,
            thermal: ThermalState::Unknown,
            network: NetworkStats::default(),
            uptime_secs: 0,
            top_by_cpu: Vec::new(),
            top_by_memory: Vec::new(),
        }
        .with_top_processes(processes);
        assert_eq!(snapshot.top_by_cpu.len(), TOP_PROCESSES);
        assert_eq!(snapshot.top_by_cpu[0].pid, 29);
        assert_eq!(snapshot.top_by_memory.len(), TOP_PROCESSES);
        assert_eq!(snapshot.top_by_memory[0].pid, 0);
    }

    #[test]
    fn real_refresh_reads_something_plausible_and_does_not_panic() {
        let mut monitor = Monitor::new();
        let first = monitor.refresh();
        assert!(first.memory.total > 0);
        assert!(first.memory.used <= first.memory.total * 2);
        assert!(first.uptime_secs > 0);
        assert!(!first.cpu.core_clusters.is_empty());
        let second = monitor.refresh();
        assert!(!second.top_by_memory.is_empty());
    }

    #[test]
    fn sysctl_helpers_handle_missing_keys() {
        assert!(sysctl_u64("hw.memsize").is_some());
        assert_eq!(sysctl_u64("definitely.not.a.sysctl"), None);
        assert_eq!(sysctl_string("definitely.not.a.sysctl"), None);
    }
}

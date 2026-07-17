//! Machine stats for the status bar: CPU%, memory, and network/disk throughput.
//!
//! Sampling is delegated to `fdtop`'s `prexp-core` (native on macOS, procfs on
//! Linux) — tty uses only its system-level readings, not the process explorer.
//! Aggregate CPU% is a delta of tick counts between two samples (top-style) and
//! the network/disk rates are byte-counter deltas over the elapsed interval, so
//! this holds the previous totals; memory is an instantaneous read. Network and
//! disk have samplers on macOS only for now (the Linux side is a follow-up, so
//! those reads error there and are dropped). See
//! `docs/ideas/status-bar-metrics.md`.

use prexp_core::backend::NativeSource;
use prexp_core::source::ProcessSource;
use prexp_core::system::{CpuTicks, DiskCounters, NetworkCounters};

/// One reading for the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MachineStats {
    /// Aggregate CPU utilization over the last interval, 0..=100.
    pub cpu_percent: f32,
    /// Memory in use, bytes.
    pub mem_used: u64,
    /// Total physical memory, bytes.
    pub mem_total: u64,
    /// Swap in use / total, bytes (`0` when there is no swap).
    pub swap_used: u64,
    pub swap_total: u64,
    /// Network receive / transmit throughput over the last interval, bytes/sec.
    pub net_rx_bps: f32,
    pub net_tx_bps: f32,
    /// Disk read / write throughput over the last interval, bytes/sec.
    pub disk_r_bps: f32,
    pub disk_w_bps: f32,
}

impl MachineStats {
    /// Memory in use as a 0..=100 percentage (for the sparkline color grade).
    pub fn mem_percent(&self) -> f32 {
        mem_percent(self.mem_used, self.mem_total)
    }
}

/// How many samples of history each metric keeps for its sparkline (~2 minutes
/// at the 2s sample cadence).
pub const HISTORY_LEN: usize = 60;

/// The sampler carried in `Tty`. Plain data (no FFI handle — a `NativeSource`
/// is constructed per sample, which is cheap), so the headless fixtures build
/// it with `Default`.
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    /// Previous `(busy, idle)` tick totals summed across cores; `None` until
    /// the first sample establishes the baseline.
    prev_busy_idle: Option<(u64, u64)>,
    /// Previous cumulative network/disk counters, and the instant they were
    /// read, for turning byte deltas into per-second rates. `None` until a
    /// first successful read establishes the baseline (network/disk are
    /// optional: a platform without a sampler just leaves these `None`).
    prev_net: Option<NetworkCounters>,
    prev_disk: Option<DiskCounters>,
    prev_instant: Option<std::time::Instant>,
    /// The most recent reading, once at least one sample has landed.
    pub latest: Option<MachineStats>,
    /// Recent aggregate CPU% (oldest first), for the sparkline. The baseline
    /// sample contributes no point (its CPU% is a placeholder 0).
    pub cpu_history: std::collections::VecDeque<f32>,
    /// Recent memory-use percentage (oldest first), for the sparkline.
    pub mem_history: std::collections::VecDeque<f32>,
    /// Recent throughput rates (bytes/sec, oldest first), for the sparklines.
    /// A point is pushed only once a baseline exists to diff against.
    pub net_rx_history: std::collections::VecDeque<f32>,
    pub net_tx_history: std::collections::VecDeque<f32>,
    pub disk_r_history: std::collections::VecDeque<f32>,
    pub disk_w_history: std::collections::VecDeque<f32>,
    /// Previous per-core `(busy, idle)` tick totals, aligned with `cpu_ticks`;
    /// `None` until the first sample. Drives the per-core CPU drill-in.
    prev_core: Option<Vec<(u64, u64)>>,
    /// Recent per-core CPU% (one bounded history per logical core, oldest
    /// first), for the per-core sparkline grid. Empty until a baseline lands.
    pub core_history: Vec<std::collections::VecDeque<f32>>,
    /// Each logical core's cluster type (P/E), read once from the platform and
    /// cached (the topology is static). `None` where unavailable, so the grid
    /// falls back to a single ungrouped section.
    pub perf_levels: Option<Vec<prexp_core::system::CpuKind>>,
    /// The system boot time (Unix-epoch seconds), read once from `prexp-core`;
    /// `None` until read (or where the platform can't report it). Drives the
    /// system-uptime cell.
    boot_time_secs: Option<u64>,
    /// When this terminal's sampler first ran (Unix-epoch seconds), set on the
    /// first `sample()`. Drives the session-uptime cell.
    session_start_secs: Option<u64>,
    /// System uptime in whole seconds (now − boot), refreshed each sample.
    /// `None` where the boot time is unavailable.
    pub system_uptime_secs: Option<u64>,
    /// This terminal session's uptime in whole seconds (now − first sample),
    /// refreshed each sample.
    pub session_uptime_secs: Option<u64>,
    /// The system load average (1, 5, 15-minute), refreshed each sample; `None`
    /// where the platform can't report it. Drives the load cell.
    pub load_avg: Option<[f32; 3]>,
    /// Recent 1-minute load (oldest first), for the load cell's sparkline.
    pub load1_history: std::collections::VecDeque<f32>,
    /// The primary battery's state, refreshed each sample; `None` on a machine
    /// with no battery (or where unreadable). Drives the battery cell.
    pub battery: Option<prexp_core::system::BatteryInfo>,
    /// Recent battery charge percentage (oldest first), for its sparkline.
    pub battery_history: std::collections::VecDeque<f32>,
    /// Previous cumulative per-process CPU time (pid → ns), and the instant read,
    /// for folding per-process CPU% from the delta. Only populated while a
    /// Processes cell is shown (see [`Self::sample_processes`]).
    prev_proc_cpu: std::collections::HashMap<i32, u64>,
    prev_proc_instant: Option<std::time::Instant>,
    /// The current per-process list (unsorted; the view sorts per the active
    /// column). Empty until sampled.
    pub processes: Vec<ProcInfo>,
    /// The live detail for the process whose drill-in is open, or `None`. Its CPU
    /// history is fresh per open (see [`ProcDetail`]); sampled only while open.
    pub proc_detail: Option<ProcDetail>,
}

/// One process's live resource use, for the Processes widget.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcInfo {
    pub pid: i32,
    pub name: String,
    /// CPU% over the last interval (can exceed 100 for a multi-threaded process,
    /// like `top`).
    pub cpu_percent: f32,
    /// Physical footprint (private memory) in bytes.
    pub memory_bytes: u64,
}

/// The live detail for a single process, driving the Processes drill-in's
/// per-process view. Populated only while one process's detail is open; the CPU
/// history starts fresh each time a process is opened (we deliberately do not
/// retain history for every process, only the one being looked at). Refreshed
/// each sample via `prexp-core`'s `snapshot_pid` (which enumerates fds).
#[derive(Debug, Clone)]
pub struct ProcDetail {
    pub pid: i32,
    pub name: String,
    /// False when the OS denied fd access (pid/name/cpu are still valid).
    pub accessible: bool,
    pub thread_count: i32,
    pub memory_bytes: u64,
    /// CPU% since this process's detail was opened (oldest first), for the chart.
    pub cpu_history: std::collections::VecDeque<f32>,
    /// The process's open file descriptors, refreshed each sample.
    pub resources: Vec<prexp_core::models::OpenResource>,
    prev_cpu_ns: u64,
    prev_instant: std::time::Instant,
}

impl Metrics {
    /// Take one sample: read CPU ticks + memory from `prexp-core`, fold the
    /// aggregate CPU% from the delta against the previous sample, and store the
    /// result in [`Self::latest`]. The first call sets the baseline and reports
    /// `cpu_percent = 0` (there is no prior interval to diff yet). A failed read
    /// is warned and dropped — a stats hiccup must never disturb the terminal.
    pub fn sample(&mut self) {
        let source = NativeSource::new();
        let mem = match source.memory_info() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("machine stats: memory read failed, skipping: {e}");
                return;
            }
        };
        let ticks = match source.cpu_ticks() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("machine stats: cpu read failed, skipping: {e}");
                return;
            }
        };

        // Elapsed since the previous sample, for turning byte deltas into rates.
        let now = std::time::Instant::now();
        let elapsed = self
            .prev_instant
            .map(|p| now.duration_since(p).as_secs_f32())
            .filter(|s| *s > 0.0);
        self.prev_instant = Some(now);

        let (busy, idle) = fold_ticks(&ticks);
        let had_baseline = self.prev_busy_idle.is_some();
        let cpu_percent = match self.prev_busy_idle {
            Some((pb, pi)) => {
                cpu_percent_from_delta(busy.saturating_sub(pb), idle.saturating_sub(pi))
            }
            None => 0.0,
        };
        self.prev_busy_idle = Some((busy, idle));

        // Per-core CPU%: the same busy/idle delta kept per logical core for the
        // drill-in grid. Perf levels (P/E) are static, so read once and cache.
        if self.perf_levels.is_none() {
            self.perf_levels = source.cpu_perf_levels().ok();
        }
        let cores: Vec<(u64, u64)> = ticks.iter().map(core_busy_idle).collect();
        if let Some(prev) = &self.prev_core {
            if prev.len() == cores.len() {
                if self.core_history.len() != cores.len() {
                    self.core_history = vec![std::collections::VecDeque::new(); cores.len()];
                }
                for (i, (&(cbusy, cidle), &(pbusy, pidle))) in
                    cores.iter().zip(prev.iter()).enumerate()
                {
                    let pct = cpu_percent_from_delta(
                        cbusy.saturating_sub(pbusy),
                        cidle.saturating_sub(pidle),
                    );
                    push_capped(&mut self.core_history[i], pct);
                }
            }
        }
        self.prev_core = Some(cores);

        let mem_percent = mem_percent(mem.used, mem.total);

        // Network / disk are optional: an unsupported platform returns an error,
        // which we drop (leaving the rate at 0 and pushing no history point).
        let net = source.network_counters().ok();
        let (net_rx_bps, net_tx_bps, net_ready) = match (net, self.prev_net, elapsed) {
            (Some(c), Some(p), Some(el)) => (
                rate(c.rx_bytes, p.rx_bytes, el),
                rate(c.tx_bytes, p.tx_bytes, el),
                true,
            ),
            _ => (0.0, 0.0, false),
        };
        if net.is_some() {
            self.prev_net = net;
        }

        let disk = source.disk_counters().ok();
        let (disk_r_bps, disk_w_bps, disk_ready) = match (disk, self.prev_disk, elapsed) {
            (Some(c), Some(p), Some(el)) => (
                rate(c.read_bytes, p.read_bytes, el),
                rate(c.write_bytes, p.write_bytes, el),
                true,
            ),
            _ => (0.0, 0.0, false),
        };
        if disk.is_some() {
            self.prev_disk = disk;
        }

        self.latest = Some(MachineStats {
            cpu_percent,
            mem_used: mem.used,
            mem_total: mem.total,
            swap_used: mem.swap_used,
            swap_total: mem.swap_total,
            net_rx_bps,
            net_tx_bps,
            disk_r_bps,
            disk_w_bps,
        });

        // Memory is real from the first sample; the rate/CPU% series only get a
        // point once a baseline exists to diff against (the first would be a
        // placeholder 0 that would dent the sparkline).
        push_capped(&mut self.mem_history, mem_percent);
        if had_baseline {
            push_capped(&mut self.cpu_history, cpu_percent);
        }
        if net_ready {
            push_capped(&mut self.net_rx_history, net_rx_bps);
            push_capped(&mut self.net_tx_history, net_tx_bps);
        }
        if disk_ready {
            push_capped(&mut self.disk_r_history, disk_r_bps);
            push_capped(&mut self.disk_w_history, disk_w_bps);
        }

        // Uptimes: wall-clock seconds since boot (system) and since the first
        // sample (this terminal session). Read the boot time once (it only moves
        // across reboots); a platform that can't report it leaves system uptime
        // `None`. Both are recomputed here so the cells refresh each tick.
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if self.boot_time_secs.is_none() {
            self.boot_time_secs = source.system_boot_time_secs().ok();
        }
        self.system_uptime_secs = self
            .boot_time_secs
            .map(|boot| now_secs.saturating_sub(boot));
        let start = *self.session_start_secs.get_or_insert(now_secs);
        self.session_uptime_secs = Some(now_secs.saturating_sub(start));

        // Load average (1/5/15-minute). The 1-minute value also feeds a sparkline.
        if let Ok(load) = source.system_load_average() {
            let load = [load[0] as f32, load[1] as f32, load[2] as f32];
            self.load_avg = Some(load);
            push_capped(&mut self.load1_history, load[0]);
        }

        // Battery (laptops only). The charge % feeds a 0..100 gauge sparkline.
        if let Ok(bat) = source.system_battery() {
            self.battery = Some(bat);
            push_capped(&mut self.battery_history, bat.percent as f32);
        }
    }

    /// Sample the whole process table for the Processes widget: fold each
    /// process's CPU% from its cumulative-CPU-time delta and its memory% from the
    /// total. Uses `prexp-core`'s light `process_summaries` (no fd enumeration).
    /// Call only while a Processes cell is shown — it walks every pid. Reads
    /// `mem_total` from the last [`Self::sample`], so call it after.
    pub fn sample_processes(&mut self) {
        let source = NativeSource::new();
        let summaries = match source.process_summaries() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("processes: read failed, skipping: {e}");
                return;
            }
        };
        let now = std::time::Instant::now();
        let elapsed_ns = self
            .prev_proc_instant
            .map(|p| now.duration_since(p).as_nanos())
            .filter(|n| *n > 0);

        let mut next_prev = std::collections::HashMap::with_capacity(summaries.len());
        let mut procs = Vec::with_capacity(summaries.len());
        for s in &summaries {
            let cpu_percent = match (self.prev_proc_cpu.get(&s.pid), elapsed_ns) {
                (Some(&prev), Some(el)) => proc_cpu_percent(prev, s.cpu_time_ns, el),
                _ => 0.0,
            };
            next_prev.insert(s.pid, s.cpu_time_ns);
            procs.push(ProcInfo {
                pid: s.pid,
                name: s.name.clone(),
                cpu_percent,
                memory_bytes: s.memory_phys,
            });
        }
        self.prev_proc_cpu = next_prev;
        self.prev_proc_instant = Some(now);
        self.processes = procs;
    }

    /// Refresh the open process's detail (the Processes drill-in's per-process
    /// view). Reads one process fully via `snapshot_pid` (which enumerates fds),
    /// folds a CPU% from the cumulative-CPU-time delta, and appends it to a
    /// history that is *fresh for this pid* — opening a different process starts a
    /// new history rather than retaining every process's series. If the pid is
    /// gone (it exited), the detail is dropped. Call only while a detail is open.
    pub fn sample_proc_detail(&mut self, pid: i32) {
        let source = NativeSource::new();
        let snap = match source.snapshot_pid(pid) {
            Ok(s) => s,
            Err(_) => {
                // The process likely exited; drop the detail so the view returns
                // to the list.
                self.proc_detail = None;
                return;
            }
        };
        let now = std::time::Instant::now();
        let cpu_ns = snap.activity.cpu_time_ns;
        match self.proc_detail.as_mut() {
            // Same process still open: fold a CPU% from the delta and append it.
            Some(d) if d.pid == pid => {
                let elapsed = now.duration_since(d.prev_instant).as_nanos();
                if elapsed > 0 {
                    let cpu = proc_cpu_percent(d.prev_cpu_ns, cpu_ns, elapsed);
                    d.cpu_history.push_back(cpu);
                    while d.cpu_history.len() > PROC_DETAIL_HISTORY {
                        d.cpu_history.pop_front();
                    }
                }
                d.prev_cpu_ns = cpu_ns;
                d.prev_instant = now;
                d.name = snap.name;
                d.accessible = snap.accessible;
                d.thread_count = snap.activity.thread_count;
                d.memory_bytes = snap.memory.phys;
                d.resources = snap.resources;
            }
            // Newly opened (or switched to a different pid): start a fresh history.
            _ => {
                self.proc_detail = Some(ProcDetail {
                    pid,
                    name: snap.name,
                    accessible: snap.accessible,
                    thread_count: snap.activity.thread_count,
                    memory_bytes: snap.memory.phys,
                    cpu_history: std::collections::VecDeque::new(),
                    resources: snap.resources,
                    prev_cpu_ns: cpu_ns,
                    prev_instant: now,
                });
            }
        }
    }
}

/// How many CPU% samples the process-detail chart retains (a few minutes at the
/// usual sample interval).
const PROC_DETAIL_HISTORY: usize = 180;

/// Resolve a process's executable path by pid, or `None` if it can't be read
/// (the process exited, or the platform doesn't support it). Used lazily by the
/// process context menu's "Copy path" so the whole list isn't paying for it.
pub(crate) fn process_path(pid: i32) -> Option<String> {
    NativeSource::new().process_path(pid).ok()
}

#[cfg(test)]
impl ProcDetail {
    /// Build a detail with fixed contents, for snapshot/behavior fixtures (the
    /// live path reads the OS via `snapshot_pid`).
    pub(crate) fn for_test(
        pid: i32,
        name: &str,
        thread_count: i32,
        memory_bytes: u64,
        cpu_history: impl IntoIterator<Item = f32>,
        resources: Vec<prexp_core::models::OpenResource>,
    ) -> Self {
        Self {
            pid,
            name: name.to_string(),
            accessible: true,
            thread_count,
            memory_bytes,
            cpu_history: cpu_history.into_iter().collect(),
            resources,
            prev_cpu_ns: 0,
            prev_instant: std::time::Instant::now(),
        }
    }
}

/// Per-second rate from a monotonic counter delta over `elapsed` seconds.
/// Saturating so a counter reset (or wrap) reads as 0 rather than a huge spike.
fn rate(cur: u64, prev: u64, elapsed: f32) -> f32 {
    if elapsed <= 0.0 {
        return 0.0;
    }
    cur.saturating_sub(prev) as f32 / elapsed
}

/// A process's CPU% over an interval: the cumulative-CPU-time delta (`cur_ns` −
/// `prev_ns`, in nanoseconds) as a fraction of the wall-clock `elapsed_ns`. Can
/// exceed 100 for a multi-threaded process, like `top`. Saturating (a counter
/// reset reads 0, not a huge spike), and 0 on a non-positive interval so a first
/// sample or a clock hiccup reads calm.
fn proc_cpu_percent(prev_ns: u64, cur_ns: u64, elapsed_ns: u128) -> f32 {
    if elapsed_ns == 0 {
        return 0.0;
    }
    (cur_ns.saturating_sub(prev_ns) as f64 / elapsed_ns as f64 * 100.0) as f32
}

/// Push `v` onto `hist`, dropping the oldest past [`HISTORY_LEN`].
fn push_capped(hist: &mut std::collections::VecDeque<f32>, v: f32) {
    hist.push_back(v);
    while hist.len() > HISTORY_LEN {
        hist.pop_front();
    }
}

/// Memory-in-use as a 0..=100 percentage, guarding a zero total.
fn mem_percent(used: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.0;
    }
    ((used as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32
}

/// One core's `(busy, idle)` from its tick counts: busy = user + system + nice.
/// Wrapping adds, since these are monotonic counters we only ever diff.
fn core_busy_idle(c: &CpuTicks) -> (u64, u64) {
    let busy = c.user.wrapping_add(c.system).wrapping_add(c.nice);
    (busy, c.idle)
}

/// Sum `(busy, idle)` tick totals across all cores. Busy is user + system +
/// nice; idle is idle. Wrapping adds, since these are monotonically increasing
/// counters we only ever diff.
fn fold_ticks(ticks: &[CpuTicks]) -> (u64, u64) {
    let mut busy = 0u64;
    let mut idle = 0u64;
    for c in ticks {
        busy = busy
            .wrapping_add(c.user)
            .wrapping_add(c.system)
            .wrapping_add(c.nice);
        idle = idle.wrapping_add(c.idle);
    }
    (busy, idle)
}

/// Aggregate CPU% over one interval from `busy`/`idle` tick deltas. A zero (or
/// empty) interval reports 0 rather than dividing by zero; the result is
/// clamped to 0..=100.
fn cpu_percent_from_delta(busy_delta: u64, idle_delta: u64) -> f32 {
    let total = busy_delta + idle_delta;
    if total == 0 {
        return 0.0;
    }
    ((busy_delta as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32
}

/// The CPU cell's label, e.g. `CPU 34%`.
pub fn cpu_label(stats: &MachineStats) -> String {
    format!("CPU {}%", stats.cpu_percent.round() as u32)
}

/// The memory cell's label, e.g. `MEM 12.1G/16.0G`.
pub fn mem_label(stats: &MachineStats) -> String {
    format!(
        "MEM {}/{}",
        format_bytes(stats.mem_used),
        format_bytes(stats.mem_total),
    )
}

/// The swap line for the memory drill-in, e.g. `Swap 1.2G/8.0G` — or `No swap`
/// when the machine has none.
pub fn swap_label(stats: &MachineStats) -> String {
    if stats.swap_total == 0 {
        "No swap".to_string()
    } else {
        format!(
            "Swap {}/{}",
            format_bytes(stats.swap_used),
            format_bytes(stats.swap_total),
        )
    }
}

/// The network cells' labels, e.g. `Net ↓ 1.2M/s` / `Net ↑ 40K/s`.
pub fn net_rx_label(stats: &MachineStats) -> String {
    format!("Net ↓ {}", format_rate(stats.net_rx_bps))
}
pub fn net_tx_label(stats: &MachineStats) -> String {
    format!("Net ↑ {}", format_rate(stats.net_tx_bps))
}

/// The combined network I/O label, e.g. `Net ↓ 1.2M/s ↑ 40K/s` — both
/// directions beside the one two-series sparkline that shows them.
pub fn net_io_label(stats: &MachineStats) -> String {
    format!(
        "Net ↓ {} ↑ {}",
        format_rate(stats.net_rx_bps),
        format_rate(stats.net_tx_bps),
    )
}

/// The disk cells' labels, e.g. `Disk R 5.0M/s` / `Disk W 0B/s`.
pub fn disk_r_label(stats: &MachineStats) -> String {
    format!("Disk R {}", format_rate(stats.disk_r_bps))
}
pub fn disk_w_label(stats: &MachineStats) -> String {
    format!("Disk W {}", format_rate(stats.disk_w_bps))
}

/// The combined disk I/O label, e.g. `Disk R 5.0M/s W 1.0M/s` — both directions
/// beside the one two-series sparkline that shows them.
pub fn disk_io_label(stats: &MachineStats) -> String {
    format!(
        "Disk R {} W {}",
        format_rate(stats.disk_r_bps),
        format_rate(stats.disk_w_bps),
    )
}

/// The load cell's label from the 1-minute load, e.g. `load 1.23`.
pub fn load_label(load1: f32) -> String {
    format!("load {load1:.2}")
}

/// The battery cell's label, e.g. `bat 82%` (a trailing `↑` while charging).
pub fn battery_label(bat: &prexp_core::system::BatteryInfo) -> String {
    let arrow = if bat.charging { " ↑" } else { "" };
    format!("bat {}%{arrow}", bat.percent.round() as i32)
}

/// The battery drill-in's state line: charging + time-to-full, discharging +
/// time-remaining, or on-AC when neither estimate applies.
pub fn battery_detail(bat: &prexp_core::system::BatteryInfo) -> String {
    if bat.charging {
        if bat.time_to_full_min > 0 {
            format!("Charging — {} to full", hm_from_mins(bat.time_to_full_min))
        } else {
            "Charging".to_string()
        }
    } else if bat.time_to_empty_min > 0 {
        format!("{} remaining", hm_from_mins(bat.time_to_empty_min))
    } else {
        "On AC power".to_string()
    }
}

/// A minute count as `3h 20m` / `45m`.
fn hm_from_mins(mins: i32) -> String {
    let (h, m) = (mins / 60, mins % 60);
    if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// The full load triple for the drill-in, e.g. `1.23 · 0.95 · 0.80  (1 / 5 / 15 min)`.
pub fn load_triple(load: [f32; 3]) -> String {
    format!(
        "{:.2} · {:.2} · {:.2}  (1 / 5 / 15 min)",
        load[0], load[1], load[2]
    )
}

/// Break a duration in seconds into `(days, hours, minutes, seconds)`.
fn dhms(secs: u64) -> (u64, u64, u64, u64) {
    (
        secs / 86_400,
        (secs % 86_400) / 3600,
        (secs % 3600) / 60,
        secs % 60,
    )
}

/// Compact uptime for a status-bar cell: the two most-significant non-zero
/// units, prefixed `up`. e.g. `up 3d 4h`, `up 4h 12m`, `up 12m`, `up 45s`.
pub fn uptime_abbrev(secs: u64) -> String {
    let (d, h, m, s) = dhms(secs);
    let body = if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    };
    format!("up {body}")
}

/// The full uptime breakdown for the drill-in popover: every non-zero unit
/// spelled out, e.g. `3 days, 4 hours, 12 minutes`. Seconds appear only under a
/// minute (otherwise they just churn). Zero reads as `less than a minute`.
pub fn uptime_full(secs: u64) -> String {
    let (d, h, m, s) = dhms(secs);
    let unit = |n: u64, name: &str| {
        (n > 0).then(|| format!("{n} {name}{}", if n == 1 { "" } else { "s" }))
    };
    let parts: Vec<String> = [unit(d, "day"), unit(h, "hour"), unit(m, "minute")]
        .into_iter()
        .flatten()
        .collect();
    if parts.is_empty() {
        // Under a minute: show seconds so it isn't blank.
        return if s > 0 {
            format!("{s} second{}", if s == 1 { "" } else { "s" })
        } else {
            "less than a minute".to_string()
        };
    }
    parts.join(", ")
}

/// The clock cell's display options (see `Settings::clock_format`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClockFormat {
    /// 24-hour time vs 12-hour with AM/PM.
    pub hour24: bool,
    /// Include seconds.
    pub seconds: bool,
    /// Prefix the weekday + month/day.
    pub date: bool,
}

/// Format a time for the clock cell per `fmt`: e.g. `2:31 PM`, `14:31:05`,
/// `Fri Jul 17  2:31 PM`. Pure (takes the time in), so it's unit-testable without
/// a clock or a timezone.
pub fn format_clock(dt: chrono::NaiveDateTime, fmt: ClockFormat) -> String {
    use chrono::Timelike;
    // 12-hour hour with no leading zero (chrono's `%-I` isn't portable on every
    // libc, so compute it directly).
    let mut out = String::new();
    if fmt.date {
        out.push_str(&dt.format("%a %b %-d  ").to_string());
    }
    if fmt.hour24 {
        out.push_str(
            &dt.format(if fmt.seconds { "%H:%M:%S" } else { "%H:%M" })
                .to_string(),
        );
    } else {
        let h24 = dt.hour();
        let h12 = match h24 % 12 {
            0 => 12,
            h => h,
        };
        let ampm = if h24 < 12 { "AM" } else { "PM" };
        if fmt.seconds {
            out.push_str(&format!(
                "{h12}:{:02}:{:02} {ampm}",
                dt.minute(),
                dt.second()
            ));
        } else {
            out.push_str(&format!("{h12}:{:02} {ampm}", dt.minute()));
        }
    }
    out
}

/// Compact throughput for the status bar: `0B/s`, `40K/s`, `1.2M/s`. Whole
/// bytes/KiB, one decimal for MiB/GiB where it reads cleanly.
pub fn format_rate(bps: f32) -> String {
    const K: f64 = 1024.0;
    let b = bps.max(0.0) as f64;
    if b < K {
        format!("{}B/s", b.round() as u64)
    } else if b < K * K {
        format!("{}K/s", (b / K).round() as u64)
    } else if b < K * K * K {
        format!("{:.1}M/s", b / (K * K))
    } else {
        format!("{:.1}G/s", b / (K * K * K))
    }
}

/// Compact binary byte size for the status bar: `512M`, `12.1G`. One decimal
/// place only where it reads cleanly (GiB/TiB), whole numbers below.
pub(crate) fn format_bytes(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b < K * K {
        format!("{}K", (b / K).round() as u64)
    } else if b < K * K * K {
        format!("{}M", (b / (K * K)).round() as u64)
    } else {
        format!("{:.1}G", b / (K * K * K))
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;

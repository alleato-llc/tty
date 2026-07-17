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
fn format_bytes(bytes: u64) -> String {
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

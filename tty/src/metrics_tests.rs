//! Pure-helper tests for machine-stats math. The FFI sampling itself
//! (`Metrics::sample`) is manual-verification territory, like the keychain.

use super::*;

fn ticks(user: u64, system: u64, idle: u64, nice: u64) -> CpuTicks {
    CpuTicks {
        user,
        system,
        idle,
        nice,
    }
}

#[test]
fn fold_ticks_sums_busy_and_idle_across_cores() {
    // Two cores: busy = user+system+nice, idle = idle, summed.
    let cores = [ticks(10, 5, 100, 2), ticks(20, 3, 200, 4)];
    let (busy, idle) = fold_ticks(&cores);
    assert_eq!(busy, 10 + 5 + 2 + 20 + 3 + 4);
    assert_eq!(idle, 100 + 200);
}

#[test]
fn cpu_percent_is_busy_over_total() {
    // 30 busy of 100 total → 30%.
    assert_eq!(cpu_percent_from_delta(30, 70), 30.0);
    // Fully busy / fully idle boundaries.
    assert_eq!(cpu_percent_from_delta(100, 0), 100.0);
    assert_eq!(cpu_percent_from_delta(0, 100), 0.0);
}

#[test]
fn cpu_percent_guards_a_zero_interval() {
    // No ticks elapsed (two samples same instant) must not divide by zero.
    assert_eq!(cpu_percent_from_delta(0, 0), 0.0);
}

#[test]
fn format_bytes_scales_units() {
    assert_eq!(format_bytes(512 * 1024), "512K");
    assert_eq!(format_bytes(200 * 1024 * 1024), "200M");
    // 16 GiB reads with one decimal.
    assert_eq!(format_bytes(16 * 1024 * 1024 * 1024), "16.0G");
}

#[test]
fn mem_percent_guards_zero_total() {
    assert_eq!(mem_percent(0, 0), 0.0);
    assert_eq!(mem_percent(8, 16), 50.0);
    // Clamped even if used somehow exceeds total.
    assert_eq!(mem_percent(20, 16), 100.0);
}

#[test]
fn format_rate_scales_units() {
    assert_eq!(format_rate(0.0), "0B/s");
    assert_eq!(format_rate(512.0), "512B/s");
    assert_eq!(format_rate(40.0 * 1024.0), "40K/s");
    assert_eq!(format_rate(1.5 * 1024.0 * 1024.0), "1.5M/s");
    assert_eq!(format_rate(2.0 * 1024.0 * 1024.0 * 1024.0), "2.0G/s");
    // A negative (impossible) rate floors at zero rather than printing junk.
    assert_eq!(format_rate(-5.0), "0B/s");
}

#[test]
fn rate_from_counter_delta() {
    // 1000 bytes over 2s → 500 B/s.
    assert_eq!(rate(1000, 0, 2.0), 500.0);
    // A counter that went backwards (reset/wrap) reads as 0, not a huge spike.
    assert_eq!(rate(100, 200, 2.0), 0.0);
    // A zero interval must not divide by zero.
    assert_eq!(rate(1000, 0, 0.0), 0.0);
}

#[test]
fn proc_cpu_percent_from_time_delta() {
    // Half a second of CPU time over a one-second interval → 50%.
    assert_eq!(proc_cpu_percent(0, 500_000_000, 1_000_000_000), 50.0);
    // A full second of CPU per wall-second → 100% (one saturated thread).
    assert_eq!(proc_cpu_percent(0, 1_000_000_000, 1_000_000_000), 100.0);
    // Multi-threaded: two cores' worth of CPU time reads over 100, like `top`.
    assert_eq!(proc_cpu_percent(0, 2_000_000_000, 1_000_000_000), 200.0);
    // Only the delta counts (prev is subtracted).
    assert_eq!(
        proc_cpu_percent(1_000_000_000, 1_250_000_000, 1_000_000_000),
        25.0
    );
    // A counter reset (cur < prev) saturates to 0, not a huge spike.
    assert_eq!(proc_cpu_percent(500_000_000, 0, 1_000_000_000), 0.0);
    // A non-positive interval reads calm (first sample / clock hiccup).
    assert_eq!(proc_cpu_percent(0, 500_000_000, 0), 0.0);
}

#[test]
fn push_capped_keeps_newest_and_bounds_length() {
    let mut hist = std::collections::VecDeque::new();
    for i in 0..(HISTORY_LEN + 10) {
        push_capped(&mut hist, i as f32);
    }
    assert_eq!(hist.len(), HISTORY_LEN, "length is bounded");
    assert_eq!(
        *hist.back().unwrap(),
        (HISTORY_LEN + 9) as f32,
        "newest is kept"
    );
    assert_eq!(
        *hist.front().unwrap(),
        10.0,
        "the oldest beyond the cap rolled off"
    );
}

/// Live end-to-end check of the real sampler (ignored by default — it reads the
/// actual machine via prexp-core FFI). Run with:
/// `cargo test -p tty --bin tty -- --ignored --nocapture live_sample`.
#[test]
#[ignore = "reads live machine stats via FFI; run manually"]
fn live_sample_reports_plausible_numbers() {
    let mut m = Metrics::default();
    m.sample(); // establishes the CPU% baseline
    std::thread::sleep(std::time::Duration::from_millis(500));
    m.sample(); // a real interval to diff against
    let s = m.latest.expect("a sample landed");
    println!(
        "live machine stats → {} · {} · {} · {} · {} · {}",
        cpu_label(&s),
        mem_label(&s),
        net_rx_label(&s),
        net_tx_label(&s),
        disk_r_label(&s),
        disk_w_label(&s),
    );
    assert!(s.mem_total > 0, "total memory must be > 0");
    assert!(s.mem_used <= s.mem_total, "used must not exceed total");
    assert!(
        (0.0..=100.0).contains(&s.cpu_percent),
        "cpu% out of range: {}",
        s.cpu_percent
    );
    // Rates are non-negative (0 is fine on an idle machine).
    for (name, v) in [
        ("net_rx", s.net_rx_bps),
        ("net_tx", s.net_tx_bps),
        ("disk_r", s.disk_r_bps),
        ("disk_w", s.disk_w_bps),
    ] {
        assert!(v >= 0.0, "{name} rate negative: {v}");
    }
}

#[test]
fn labels_read_cleanly() {
    let stats = MachineStats {
        cpu_percent: 33.6,
        mem_used: 12_400_000_000,
        mem_total: 16 * 1024 * 1024 * 1024,
        swap_used: 1_288_490_188,
        swap_total: 8 * 1024 * 1024 * 1024,
        net_rx_bps: 1.5 * 1024.0 * 1024.0,
        net_tx_bps: 40.0 * 1024.0,
        disk_r_bps: 0.0,
        disk_w_bps: 5.0 * 1024.0 * 1024.0,
    };
    assert_eq!(cpu_label(&stats), "CPU 34%");
    let mem = mem_label(&stats);
    assert!(mem.starts_with("MEM "), "got: {mem}");
    // Swap reads as a size pair; zero total reads as "No swap".
    assert!(
        swap_label(&stats).starts_with("Swap "),
        "got: {}",
        swap_label(&stats)
    );
    let no_swap = MachineStats {
        swap_total: 0,
        ..stats
    };
    assert_eq!(swap_label(&no_swap), "No swap");
    assert!(mem.ends_with("/16.0G"), "got: {mem}");
    assert_eq!(net_rx_label(&stats), "Net ↓ 1.5M/s");
    assert_eq!(net_tx_label(&stats), "Net ↑ 40K/s");
    assert_eq!(disk_r_label(&stats), "Disk R 0B/s");
    assert_eq!(disk_w_label(&stats), "Disk W 5.0M/s");
    // The combined cells show both directions beside their one sparkline.
    assert_eq!(net_io_label(&stats), "Net ↓ 1.5M/s ↑ 40K/s");
    assert_eq!(disk_io_label(&stats), "Disk R 0B/s W 5.0M/s");
}

#[test]
fn uptime_formats_abbreviated_and_full() {
    // Abbreviated: the two most-significant non-zero units, prefixed "up".
    assert_eq!(uptime_abbrev(0), "up 0s");
    assert_eq!(uptime_abbrev(45), "up 45s");
    assert_eq!(uptime_abbrev(12 * 60 + 30), "up 12m");
    assert_eq!(uptime_abbrev(4 * 3600 + 12 * 60), "up 4h 12m");
    assert_eq!(uptime_abbrev(3 * 86_400 + 4 * 3600 + 12 * 60), "up 3d 4h");

    // Full: every non-zero unit spelled out and pluralized.
    assert_eq!(uptime_full(0), "less than a minute");
    assert_eq!(uptime_full(1), "1 second");
    assert_eq!(uptime_full(90), "1 minute");
    assert_eq!(uptime_full(3600 + 60), "1 hour, 1 minute");
    assert_eq!(
        uptime_full(3 * 86_400 + 4 * 3600 + 12 * 60),
        "3 days, 4 hours, 12 minutes"
    );
}

#[test]
fn clock_formats_per_options() {
    use chrono::NaiveDate;
    let dt = NaiveDate::from_ymd_opt(2026, 7, 17)
        .unwrap()
        .and_hms_opt(14, 31, 5)
        .unwrap();
    let fmt = |hour24, seconds, date| {
        format_clock(
            dt,
            ClockFormat {
                hour24,
                seconds,
                date,
            },
        )
    };
    assert_eq!(fmt(false, false, false), "2:31 PM");
    assert_eq!(fmt(false, true, false), "2:31:05 PM");
    assert_eq!(fmt(true, false, false), "14:31");
    assert_eq!(fmt(true, true, false), "14:31:05");
    // Midnight / noon read as 12, not 0, in 12-hour.
    let midnight = NaiveDate::from_ymd_opt(2026, 7, 17)
        .unwrap()
        .and_hms_opt(0, 5, 0)
        .unwrap();
    assert_eq!(format_clock(midnight, ClockFormat::default()), "12:05 AM");
    // With the date prefix, the weekday + month/day lead the time.
    let dated = fmt(true, false, true);
    assert!(
        dated.contains("Jul 17") && dated.ends_with("14:31"),
        "got: {dated}"
    );
}

#[test]
fn battery_labels_and_detail() {
    use prexp_core::system::BatteryInfo;
    let discharging = BatteryInfo {
        percent: 82.0,
        charging: false,
        time_to_empty_min: 200,
        time_to_full_min: -1,
    };
    assert_eq!(battery_label(&discharging), "bat 82%");
    assert_eq!(battery_detail(&discharging), "3h 20m remaining");
    let charging = BatteryInfo {
        percent: 45.0,
        charging: true,
        time_to_empty_min: -1,
        time_to_full_min: 45,
    };
    assert_eq!(battery_label(&charging), "bat 45% ↑");
    assert_eq!(battery_detail(&charging), "Charging — 45m to full");
    let full = BatteryInfo {
        percent: 100.0,
        charging: false,
        time_to_empty_min: -1,
        time_to_full_min: -1,
    };
    assert_eq!(battery_detail(&full), "On AC power");
}

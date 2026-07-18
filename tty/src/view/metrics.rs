//! The metrics UI: the bottom status bar (shell name + machine-stat cells,
//! width-shedding, scroll paging) and the metric drill-ins (the popover cards,
//! the graduated panes, and every body — CPU/memory/rate charts, the per-core
//! grids, uptime/clock/load/battery, and the Processes table + per-process
//! detail). Split out of `view.rs`.

use iced::widget::{column, container, row, text};
use iced::{Border, Element, Length};

use rime::theme;
use rime::widgets::{line_chart, sparkline, LineChart, Series, Sparkline};

use crate::message::Message;
use crate::state::Tty;

use super::procs::procs_body;
use super::util::*;

/// The renderable data for one metric cell, resolved from the current sample:
/// a label, one or more history series (each with its color) that share the
/// sparkline, and the sparkline's max (100 for percentages, auto-scaled for
/// rates). Most metrics have a single series; disk I/O overlays read + write.
pub(super) struct MetricRender {
    pub(super) label: String,
    pub(super) series: Vec<(std::collections::VecDeque<f32>, iced::Color)>,
    pub(super) max: f32,
    /// Set to a caution/alarm color when a graded cell is past its threshold, so
    /// `metric_cell` recolors the label (not just the sparkline). `None` = calm.
    pub(super) alert: Option<iced::Color>,
}

/// Resolve one configured metric against the latest sample, or `None` when
/// there is no reading yet (so the cell is simply skipped until data lands).
/// Percentage metrics (CPU/memory) grade their color by load and scale to 100;
/// rate metrics (network/disk) use a neutral accent and auto-scale to their
/// own recent peak. Disk I/O returns two series (read + write) on one scale.
pub(super) fn metric_render(
    cfg: crate::settings::ResolvedMetric,
    state: &Tty,
) -> Option<MetricRender> {
    use crate::metrics as mx;
    use crate::settings::MetricKind as K;

    // The clock is the wall time, independent of the sampler — render it without
    // waiting for a machine-stats reading.
    if cfg.kind == K::Clock {
        return Some(MetricRender {
            label: clock_label(state),
            series: vec![],
            max: 1.0,
            alert: None,
        });
    }
    // Env is a launcher cell, not a sampled metric — a static text label, no sampler
    // needed. Its click opens the Env popover (see the `PointerReleased` handler).
    if cfg.kind == K::Env {
        return Some(MetricRender {
            label: "env".to_string(),
            series: vec![],
            max: 1.0,
            alert: None,
        });
    }

    let stats = state.metrics.latest.as_ref()?;
    let m = &state.metrics;
    let t = theme::tokens();
    let accent = t.accent;

    // Single-series metrics: (label, history, max, color).
    let single =
        |label: String, hist: &std::collections::VecDeque<f32>, max: f32, color| MetricRender {
            label,
            series: vec![(hist.clone(), color)],
            max,
            alert: None,
        };

    // For a graded kind (CPU/mem/battery), grade the current value against this
    // cell's configured thresholds → the series color + an over-threshold alert.
    let graded: Option<(iced::Color, Option<iced::Color>)> =
        cfg.kind.default_thresholds().map(|(_, _, inverted)| {
            let value = match cfg.kind {
                K::Battery => m.battery.map_or(0.0, |b| b.percent as f32),
                K::Mem => stats.mem_percent(),
                _ => stats.cpu_percent,
            };
            let g = grade(value, cfg.warn as f32, cfg.alarm as f32, inverted);
            (grade_color(g), (g != Grade::Calm).then(|| grade_color(g)))
        });
    let graded_color = graded.map_or(accent, |(c, _)| c);

    let mut render = match cfg.kind {
        // All three CPU drill-ins share the aggregate cell; only their popover
        // body differs.
        K::Cpu | K::CpuCores | K::CpuAll => {
            single(mx::cpu_label(stats), &m.cpu_history, 100.0, graded_color)
        }
        K::Mem => single(mx::mem_label(stats), &m.mem_history, 100.0, graded_color),
        K::NetRx => single(
            mx::net_rx_label(stats),
            &m.net_rx_history,
            hist_max(&m.net_rx_history),
            accent,
        ),
        K::NetTx => single(
            mx::net_tx_label(stats),
            &m.net_tx_history,
            hist_max(&m.net_tx_history),
            accent,
        ),
        K::DiskR => single(
            mx::disk_r_label(stats),
            &m.disk_r_history,
            hist_max(&m.disk_r_history),
            accent,
        ),
        K::DiskW => single(
            mx::disk_w_label(stats),
            &m.disk_w_history,
            hist_max(&m.disk_w_history),
            accent,
        ),
        // rx + tx overlaid on one sparkline, on a shared scale (rx accent, tx
        // `warn`), mirroring disk I/O below.
        K::NetIo => MetricRender {
            label: mx::net_io_label(stats),
            series: vec![
                (m.net_rx_history.clone(), accent),
                (m.net_tx_history.clone(), t.warn),
            ],
            max: hist_max(&m.net_rx_history).max(hist_max(&m.net_tx_history)),
            alert: None,
        },
        // Read + write overlaid on one sparkline, on a shared scale so their
        // relative magnitude reads true. Read is the accent; write is `warn`
        // (a second hue, not an alarm) so the two lines stay distinguishable.
        K::DiskIo => MetricRender {
            label: mx::disk_io_label(stats),
            series: vec![
                (m.disk_r_history.clone(), accent),
                (m.disk_w_history.clone(), t.warn),
            ],
            max: hist_max(&m.disk_r_history).max(hist_max(&m.disk_w_history)),
            alert: None,
        },
        // Uptimes are text, not sparklines: no series (so `metric_cell` renders
        // the label alone). Skip the cell until a reading exists.
        K::Uptime => MetricRender {
            label: mx::uptime_abbrev(m.system_uptime_secs?),
            series: vec![],
            max: 1.0,
            alert: None,
        },
        K::Session => MetricRender {
            label: mx::uptime_abbrev(m.session_uptime_secs?),
            series: vec![],
            max: 1.0,
            alert: None,
        },
        // Load: a sparkline of the 1-minute load, auto-scaled to its recent peak
        // like the rate cells. Skip until a reading exists.
        K::Load => {
            let load = m.load_avg?;
            MetricRender {
                label: mx::load_label(load[0]),
                series: vec![(m.load1_history.clone(), accent)],
                max: hist_max(&m.load1_history),
                alert: None,
            }
        }
        // Battery: a fixed 0..100% gauge, colored by charge against its threshold
        // (low = alarm). Skip on a machine with no battery.
        K::Battery => {
            let b = m.battery?;
            MetricRender {
                label: mx::battery_label(&b),
                series: vec![(m.battery_history.clone(), graded_color)],
                max: 100.0,
                alert: None,
            }
        }
        // Processes: a text cell showing the busiest process (by CPU%); the
        // drill-in is the scrollable, sortable table. Skip until sampled.
        K::Procs => {
            let top = m.processes.iter().max_by(|a, b| {
                a.cpu_percent
                    .partial_cmp(&b.cpu_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })?;
            MetricRender {
                label: format!(
                    "↑ {} {}%",
                    truncate_name(&top.name, 16),
                    top.cpu_percent.round() as i32
                ),
                series: vec![],
                max: 1.0,
                alert: None,
            }
        }
        K::Clock => unreachable!("clock is handled before the stats read"),
        K::Env => unreachable!("env is handled before the stats read"),
    };
    // A graded cell past its warn/alarm threshold carries the alert color so the
    // label recolors, not just the sparkline.
    render.alert = graded.and_then(|(_, alert)| alert);
    Some(render)
}

/// The clock cell's label: the current local time, formatted per the user's clock
/// options. Reads the live clock, so it is never pixel-snapshotted (the pure
/// formatter `metrics::format_clock` is unit-tested instead).
fn clock_label(state: &Tty) -> String {
    crate::metrics::format_clock(
        chrono::Local::now().naive_local(),
        state.settings.clock_format(),
    )
}

/// The sparkline scale for a rate series: its recent peak, floored at 1 so an
/// all-zero history doesn't divide by zero (and reads as a flat baseline).
fn hist_max(history: &std::collections::VecDeque<f32>) -> f32 {
    history.iter().copied().fold(1.0, f32::max)
}

/// The rendered view for a metric `kind` — the chart / table / readout that fills
/// a drill-in popover *or* a graduated metric pane. Pure content: no card frame,
/// resize edges, or controls (those are the popover's / pane's own chrome).
/// `card_w` / `chart_h` size the chart to its container.
pub(super) fn metric_body<'a>(
    state: &'a Tty,
    kind: crate::settings::MetricKind,
    expanded: bool,
    card_w: f32,
    chart_h: f32,
) -> Element<'a, Message> {
    use crate::settings::{MetricKind as K, MetricStyle, ResolvedMetric};
    let t = theme::tokens();
    // Render off the configured cell (so a graded view picks up the user's
    // thresholds); fall back to defaults if somehow not in the list.
    let resolved = state
        .settings
        .status_bar_metrics()
        .into_iter()
        .find(|m| m.kind == kind)
        .unwrap_or_else(|| {
            let (warn, alarm) = kind
                .default_thresholds()
                .map_or((0.0, 0.0), |(w, a, _)| (w, a));
            ResolvedMetric {
                kind,
                style: MetricStyle::Sparkline,
                warn,
                alarm,
            }
        });
    let render = metric_render(resolved, state);

    // The line chart needs at least a couple of points to draw a segment; below
    // that (an empty or single-point history) it shows a plain note.
    let filled = render.as_ref().map_or(0, |r| {
        r.series.iter().map(|(v, _)| v.len()).max().unwrap_or(0)
    });

    // CPU has three drill-ins: total (the aggregate line chart), per-core (the
    // cluster grid), and "all" (both). Per-core / all fall back to the aggregate
    // where the platform reports no per-core history.
    let has_cores = kind.is_cpu() && has_per_core_cpu(state);

    if kind == K::Procs {
        procs_body(state, card_w, chart_h)
    } else if kind == K::Clock {
        clock_body(state)
    } else if kind.is_uptime() {
        uptime_body(state, kind)
    } else if kind == K::CpuAll && has_cores {
        combined_cpu_body(state, expanded, card_w, chart_h)
    } else if kind == K::CpuCores && has_cores {
        core_grid_body(state, expanded, card_w)
    } else if let (true, Some(r)) = (filled >= 2, render) {
        // Each history series becomes a polyline over its sample index, on the
        // same shared 0..max scale the sparkline used.
        let series: Vec<Series> = r
            .series
            .iter()
            .map(|(values, color)| Series {
                points: values
                    .iter()
                    .enumerate()
                    .map(|(i, &v)| (i as f64, v as f64))
                    .collect(),
                color: *color,
            })
            .collect();

        // The y axis: CPU/memory are bounded 0..100 gauges (a fixed scale, so the
        // line reads as a fraction of full capacity); load auto-scales to its peak
        // with a plain numeric label; the rate metrics auto-scale and label the
        // axis with the peak throughput.
        let bounded = kind.is_cpu() || kind == K::Mem || kind == K::Battery;
        let peak = || {
            r.series
                .iter()
                .flat_map(|(v, _)| v.iter().copied())
                .fold(1.0_f32, f32::max)
        };
        let y_max: Option<f64> = bounded.then_some(100.0);
        let y_max_label: Option<String> = Some(if bounded {
            "100%".to_string()
        } else if kind == K::Load {
            format!("{:.1}", peak())
        } else {
            crate::metrics::format_rate(peak())
        });
        let hover: fn(f64) -> String = if bounded {
            hover_percent
        } else if kind == K::Load {
            hover_load
        } else {
            hover_rate
        };
        let chart = line_chart(
            LineChart {
                title: kind.to_string(),
                series,
                y_max,
                y_max_label,
                hover_format: Some(hover),
            },
            chart_h,
        );

        let mut card = column![chart, text(r.label).size(13).color(t.ink)].spacing(8);
        // A small legend names the two overlaid lines for the combined metrics
        // (single-series metrics carry their identity in the title/label already);
        // load shows its full 1/5/15-minute triple.
        match kind {
            K::NetIo => card = card.push(legend_row(&[("Down", t.accent), ("Up", t.warn)])),
            K::DiskIo => card = card.push(legend_row(&[("Read", t.accent), ("Write", t.warn)])),
            K::Load => {
                let triple =
                    crate::metrics::load_triple(state.metrics.load_avg.unwrap_or_default());
                card = card.push(text(triple).size(12).color(t.muted));
            }
            K::Battery => {
                if let Some(b) = state.metrics.battery {
                    card = card.push(
                        text(crate::metrics::battery_detail(&b))
                            .size(12)
                            .color(t.muted),
                    );
                }
            }
            // Memory drills in with a swap line beneath the RAM readout (the bar
            // cell stays RAM-only).
            K::Mem => {
                if let Some(stats) = state.metrics.latest.as_ref() {
                    card = card.push(
                        text(crate::metrics::swap_label(stats))
                            .size(12)
                            .color(t.muted),
                    );
                }
            }
            _ => {}
        }
        card.into()
    } else {
        column![
            text(kind.to_string()).size(14).color(t.ink),
            text("No history yet — samples chart here as they're collected.")
                .size(12)
                .color(t.muted),
        ]
        .spacing(8)
        .into()
    }
}

/// Whether the per-core CPU grid has enough history to draw (a baseline plus at
/// least one interval for any core).
fn has_per_core_cpu(state: &Tty) -> bool {
    state.metrics.core_history.iter().any(|h| h.len() >= 2)
}

/// Group logical core indices `0..n` by cluster type for the per-core grid:
/// Performance, then Efficiency, then any Unknown — or one ungrouped "CPU"
/// section when the platform reports no perf levels. Each group carries its
/// [`CpuKind`] so cells can color by cluster.
fn core_groups(
    levels: Option<&[prexp_core::system::CpuKind]>,
    n: usize,
) -> Vec<(&'static str, prexp_core::system::CpuKind, Vec<usize>)> {
    use prexp_core::system::CpuKind;
    let Some(ls) = levels else {
        return vec![("CPU", CpuKind::Unknown, (0..n).collect())];
    };
    let pick =
        |k: CpuKind| -> Vec<usize> { (0..n).filter(|&i| ls.get(i).copied() == Some(k)).collect() };
    let unknown: Vec<usize> = (0..n)
        .filter(|&i| {
            !matches!(
                ls.get(i),
                Some(CpuKind::Performance) | Some(CpuKind::Efficiency)
            )
        })
        .collect();
    let mut groups = Vec::new();
    let p = pick(CpuKind::Performance);
    if !p.is_empty() {
        groups.push(("Performance", CpuKind::Performance, p));
    }
    let e = pick(CpuKind::Efficiency);
    if !e.is_empty() {
        groups.push(("Efficiency", CpuKind::Efficiency, e));
    }
    if !unknown.is_empty() {
        groups.push(("CPU", CpuKind::Unknown, unknown));
    }
    if groups.is_empty() {
        groups.push(("CPU", CpuKind::Unknown, (0..n).collect()));
    }
    groups
}

/// A core cell's sparkline color: Performance (and ungrouped) cores grade by
/// load (calm → alarm); Efficiency cores use the accent hue so the two clusters
/// read apart at a glance (their load still shows in the sparkline height + %).
/// The uptime drill-in's body: the full duration breakdown (the "full view" to
/// the cell's abbreviated one), under the metric name and over a note saying what
/// it counts from.
fn uptime_body(state: &Tty, kind: crate::settings::MetricKind) -> Element<'_, Message> {
    use crate::settings::MetricKind as K;
    let t = theme::tokens();
    let secs = if kind == K::Uptime {
        state.metrics.system_uptime_secs
    } else {
        state.metrics.session_uptime_secs
    };
    let (full, note) = match secs {
        Some(s) => (
            crate::metrics::uptime_full(s),
            if kind == K::Uptime {
                "since the system booted"
            } else {
                "since this terminal opened"
            },
        ),
        None => ("Unavailable".to_string(), "no reading yet"),
    };
    column![
        text(kind.to_string()).size(14).color(t.ink),
        text(full).size(20).color(t.ink),
        text(note).size(12).color(t.muted),
    ]
    .spacing(8)
    .into()
}

/// The clock drill-in's body: the current time (honoring the 12/24-hour and
/// seconds options) over the full weekday + date. Live, so never snapshotted.
fn clock_body(state: &Tty) -> Element<'_, Message> {
    let t = theme::tokens();
    let now = chrono::Local::now();
    let fmt = state.settings.clock_format();
    // The drill-in always shows seconds and the date (the "full" view), but keeps
    // the 12/24-hour choice.
    let time = crate::metrics::format_clock(
        now.naive_local(),
        crate::metrics::ClockFormat {
            hour24: fmt.hour24,
            seconds: true,
            date: false,
        },
    );
    column![
        text("Clock").size(14).color(t.ink),
        text(time).size(20).color(t.ink),
        text(now.format("%A, %B %-d, %Y").to_string())
            .size(12)
            .color(t.muted),
    ]
    .spacing(8)
    .into()
}

/// The aggregate CPU line chart: overall CPU% over its retained history on a
/// fixed 0..100 gauge, with the hover readout. Shared by the `Cpu` drill-in
/// (via the generic branch) and the `CpuAll` combined body.
fn aggregate_cpu_chart(state: &Tty, chart_h: f32) -> Element<'_, Message> {
    let agg = state.metrics.latest.map(|s| s.cpu_percent).unwrap_or(0.0);
    let series = vec![Series {
        points: state
            .metrics
            .cpu_history
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as f64, v as f64))
            .collect(),
        color: load_color(agg),
    }];
    line_chart(
        LineChart {
            title: "CPU".into(),
            series,
            y_max: Some(100.0),
            y_max_label: Some("100%".to_string()),
            hover_format: Some(hover_percent),
        },
        chart_h,
    )
}

/// The per-core sparkline grid, grouped into Performance / Efficiency clusters,
/// each cell colored by its cluster (see [`core_color`]). Column count fits the
/// card width. Shared by the `CpuCores` and `CpuAll` drill-ins.
fn cpu_core_grid(state: &Tty, expanded: bool, card_w: f32) -> iced::widget::Column<'_, Message> {
    let t = theme::tokens();
    let m = &state.metrics;
    let (cell_w, cell_h) = if expanded { (96.0, 34.0) } else { (58.0, 20.0) };
    // Fit as many columns as the card width holds (accounting for padding + gap).
    let cols = (((card_w - 28.0) / (cell_w + 8.0)).floor() as usize).max(2);
    let n = m.core_history.len();
    let mut col = column![].spacing(8);
    for (name, kind, cores) in core_groups(m.perf_levels.as_deref(), n) {
        col = col.push(text(name).size(11).color(t.muted));
        let mut grid = column![].spacing(6);
        for chunk in cores.chunks(cols) {
            let mut r = row![].spacing(8).align_y(iced::Alignment::Center);
            for &ci in chunk {
                r = r.push(core_cell(m, ci, kind, cell_w, cell_h));
            }
            grid = grid.push(r);
        }
        col = col.push(grid);
    }
    col
}

/// The `CpuAll` drill-in body: the aggregate line chart, the current readout,
/// then the per-core grid — the two CPU views stacked.
fn combined_cpu_body(
    state: &Tty,
    expanded: bool,
    card_w: f32,
    chart_h: f32,
) -> Element<'_, Message> {
    let t = theme::tokens();
    let agg = state.metrics.latest.map(|s| s.cpu_percent).unwrap_or(0.0);
    column![
        aggregate_cpu_chart(state, chart_h),
        text(format!("CPU {}%", agg.round() as u32))
            .size(13)
            .color(t.ink),
    ]
    .spacing(8)
    .push(cpu_core_grid(state, expanded, card_w))
    .into()
}

/// The `CpuCores` drill-in body: the per-core grid alone, under a compact
/// aggregate readout (no line chart — that is the separate `Cpu` drill-in).
fn core_grid_body(state: &Tty, expanded: bool, card_w: f32) -> Element<'_, Message> {
    let t = theme::tokens();
    let agg = state.metrics.latest.map(|s| s.cpu_percent).unwrap_or(0.0);
    column![text(format!("CPU cores — {}%", agg.round() as u32))
        .size(13)
        .color(t.ink),]
    .spacing(8)
    .push(cpu_core_grid(state, expanded, card_w))
    .into()
}

/// One core's cell: a small sparkline of its recent load (colored by cluster via
/// [`core_color`]) over its current percentage.
fn core_cell<'a>(
    m: &crate::metrics::Metrics,
    ci: usize,
    kind: prexp_core::system::CpuKind,
    w: f32,
    h: f32,
) -> Element<'a, Message> {
    let t = theme::tokens();
    let hist = &m.core_history[ci];
    let cur = hist.back().copied().unwrap_or(0.0);
    let spark = sparkline(
        Sparkline::single(
            hist.iter().map(|&v| v as f64).collect(),
            100.0,
            core_color(kind, cur),
        ),
        w,
        h,
    );
    column![
        spark,
        text(format!("{}%", cur.round() as u32))
            .size(10)
            .color(t.muted),
    ]
    .spacing(2)
    .align_x(iced::Alignment::Center)
    .into()
}

/// A row of colored-dot + label legend entries for a multi-series drill-in.
fn legend_row<'a>(items: &[(&str, iced::Color)]) -> Element<'a, Message> {
    let t = theme::tokens();
    let mut r = row![].spacing(14).align_y(iced::Alignment::Center);
    for &(label, color) in items {
        let dot = container(
            iced::widget::Space::new()
                .width(Length::Fixed(9.0))
                .height(Length::Fixed(9.0)),
        )
        .style(move |_| container::Style {
            border: Border {
                radius: 5.0.into(),
                ..Default::default()
            },
            ..container::background(color)
        });
        r = r.push(
            row![dot, text(label.to_string()).size(11).color(t.muted)]
                .spacing(6)
                .align_y(iced::Alignment::Center),
        );
    }
    r.into()
}

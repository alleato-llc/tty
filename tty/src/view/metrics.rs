//! The metrics UI: the bottom status bar (shell name + machine-stat cells,
//! width-shedding, scroll paging) and the metric drill-ins (the popover cards,
//! the graduated panes, and every body — CPU/memory/rate charts, the per-core
//! grids, uptime/clock/load/battery, and the Processes table + per-process
//! detail). Split out of `view.rs`.

use iced::widget::{column, container, mouse_area, pane_grid, row, scrollable, text};
use iced::{Border, Element, Length};

use rime::theme;
use rime::widgets::{
    button, line_chart, sparkline, table, CellAlign, LineChart, Series, Sparkline, TableColumn,
    TableMetrics,
};

use crate::message::Message;
use crate::state::Tty;

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
pub(super) fn metric_render(cfg: crate::settings::ResolvedMetric, state: &Tty) -> Option<MetricRender> {
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
fn metric_body<'a>(
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

/// One graduated metric **pane** (a metric drill-in promoted into the pane grid):
/// a compact header (the metric's name + maximize/restore + close) over its
/// [`metric_body`]. Focus/right-click-to-split come from the surrounding
/// `pane_grid` like any pane.
#[allow(clippy::too_many_arguments)]
pub(super) fn metric_pane_content<'a>(
    state: &'a Tty,
    kind: crate::settings::MetricKind,
    win: iced::window::Id,
    pane: pane_grid::Pane,
    is_focused: bool,
    maximized: bool,
    multi: bool,
    highlight: bool,
    accent: iced::Color,
    hairline: iced::Color,
    bg: iced::Color,
) -> pane_grid::Content<'a, Message> {
    let t = theme::tokens();
    // Size the body to the pane's rough share of the window (the closure isn't
    // handed pixel bounds). Bias width toward a split's half so long names in a
    // table truncate (an ellipsis) rather than wrap; the chart fills the height.
    let card_w = (state.window_width * 0.45).max(240.0);
    let chart_h = (state.window_height * 0.30).clamp(120.0, 360.0);

    let header = row![
        container(
            text(kind.to_string())
                .size(13)
                .color(if is_focused { accent } else { t.ink }),
        )
        .width(Length::Fill),
        button::ghost_compact(
            if maximized { "▪" } else { "▫" },
            Message::ToggleMaximizePane(win),
        ),
        button::ghost_compact("×", Message::CloseMetricPane(win, pane)),
    ]
    .align_y(iced::Alignment::Center)
    .spacing(0);

    let inner = column![header, metric_body(state, kind, false, card_w, chart_h)]
        .spacing(6)
        .padding(6);
    let border_color = if is_focused && highlight {
        accent
    } else {
        hairline
    };
    let bordered = container(inner).style(move |_| {
        let border = if multi {
            Border {
                color: border_color,
                width: 1.0,
                radius: 0.0.into(),
            }
        } else {
            Border::default()
        };
        container::Style {
            border,
            ..container::background(bg)
        }
    });
    pane_grid::Content::new(mouse_area(bordered).on_right_press(Message::PaneRightClick(pane)))
}

pub(super) fn metric_popover_card<'a>(
    state: &'a Tty,
    pop: &crate::state::MetricPopover,
    index: usize,
) -> Element<'a, Message> {
    let kind = pop.kind;
    let expanded = pop.expanded;
    let pinned = state.settings.status_bar_metrics_pinned();
    let t = theme::tokens();

    // The current size: the user's dragged override, else the compact or
    // expanded default (see `MetricPopover::effective_size`).
    let (card_w, chart_h) = pop.effective_size(state.window_width, state.window_height);
    let body = metric_body(state, kind, expanded, card_w, chart_h);

    let card = container(body)
        .width(Length::Fixed(card_w))
        .padding(14)
        .style(move |_| container::Style {
            border: Border {
                color: t.hairline,
                width: 1.0,
                radius: 10.0.into(),
            },
            ..container::background(t.surface)
        });
    // Border-resize strips and the top-right controls (expand, plus a close "×"
    // when popovers are pinned) overlay the card.
    iced::widget::stack![
        with_resize_edges(index, card.into()),
        popover_controls(
            index,
            kind,
            expanded,
            pinned,
            state.settings.graduate_metrics(),
        ),
    ]
    .into()
}

/// The top-right control cluster overlaid on a popover card: a "move to pane" ⊞
/// (graduate the drill-in into a real split pane, when enabled), the expand "+" /
/// collapse "−" toggle, plus a close "×" when popovers are pinned (in the
/// one-at-a-time mode a click away closes it, so no per-card button is needed).
fn popover_controls<'a>(
    index: usize,
    kind: crate::settings::MetricKind,
    expanded: bool,
    pinned: bool,
    can_graduate: bool,
) -> Element<'a, Message> {
    let mut controls = row![].spacing(0).align_y(iced::Alignment::Center);
    if can_graduate {
        controls = controls.push(button::ghost_compact(
            "⊞",
            Message::PromotePopoverMenu(kind),
        ));
    }
    controls = controls.push(button::ghost_compact(
        if expanded { "−" } else { "+" },
        Message::ToggleMetricDetailExpanded(index),
    ));
    if pinned {
        controls = controls.push(button::ghost_compact(
            "×",
            Message::CloseMetricPopover(index),
        ));
    }
    container(controls)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .align_y(iced::alignment::Vertical::Top)
        .padding([4, 6])
        .into()
}

/// Place a popover `card` in the window: compact cards anchor just above the
/// status bar (centered), expanded ones center; a user drag offsets from there,
/// laid out absolutely via top-left padding (horizontal exact, vertical measured
/// up from the bottom). Pinned popovers past the first cascade up-and-right so a
/// stack stays legible. All clamped to stay on-screen; the headless path (window
/// size 0) falls back to the alignment anchor.
pub(super) fn place_metric_popover<'a>(
    state: &Tty,
    pop: &crate::state::MetricPopover,
    index: usize,
    card: Element<'a, Message>,
) -> Element<'a, Message> {
    use iced::alignment::{Horizontal, Vertical};

    let known = state.window_width > 1.0 && state.window_height > 1.0;
    let cascade = 28.0 * index as f32;
    let dx = pop.move_offset.0 + cascade;
    let dy = pop.move_offset.1 - cascade;
    let (card_w, _) = pop.effective_size(state.window_width, state.window_height);

    if (dx != 0.0 || dy != 0.0) && known {
        let base_gap = if pop.expanded { 60.0 } else { 44.0 };
        let left = ((state.window_width - card_w) / 2.0 + dx)
            .clamp(8.0, (state.window_width - card_w - 8.0).max(8.0));
        let bottom = (base_gap - dy).clamp(8.0, (state.window_height - 120.0).max(8.0));
        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Left)
            .align_y(Vertical::Bottom)
            .padding(iced::Padding::ZERO.left(left).bottom(bottom))
            .into()
    } else {
        let a = container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center);
        if pop.expanded {
            a.align_y(Vertical::Center).into()
        } else {
            a.align_y(Vertical::Bottom)
                .padding(iced::Padding::ZERO.bottom(44.0))
                .into()
        }
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

/// Clip a process name to `max` chars for the compact cell (an `…` when cut).
/// The Processes drill-in. Normally a clickable header row (re-sort by clicking a
/// column) over a virtualized, scrollable `rime` table of every process, ordered
/// by the active sort; the bar cell shows only the busiest process, this is the
/// list. Double- or right-clicking a row drills into that one process's detail
/// (fds + a live chart), rendered by [`proc_detail_body`] instead.
fn procs_body(state: &Tty, card_w: f32, chart_h: f32) -> Element<'_, Message> {
    use crate::state::ProcSortColumn as Col;
    let t = theme::tokens();

    // A per-process detail takes over the whole card when one is open.
    if state.proc_detail_pid.is_some() {
        return proc_detail_body(state, chart_h);
    }

    let procs = &state.metrics.processes;
    if procs.is_empty() {
        return column![
            text("Processes").size(14).color(t.ink),
            text("Collecting…").size(12).color(t.muted),
        ]
        .spacing(8)
        .into();
    }

    let (sort_col, desc) = state.proc_sort;
    // Row order: indices into `procs` sorted by the active column/direction.
    let mut order: Vec<usize> = (0..procs.len()).collect();
    order.sort_by(|&a, &b| {
        let (pa, pb) = (&procs[a], &procs[b]);
        let cmp = match sort_col {
            Col::Name => pa.name.to_lowercase().cmp(&pb.name.to_lowercase()),
            Col::Cpu => pa
                .cpu_percent
                .partial_cmp(&pb.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal),
            Col::Mem => pa.memory_bytes.cmp(&pb.memory_bytes),
        };
        if desc {
            cmp.reverse()
        } else {
            cmp
        }
    });

    // Column widths (must match the header row and the table below); `CELL_PAD`
    // in the table is 8px, so the header cells pad to match.
    const NUM_W: f32 = 64.0;
    // The name column is the fill remainder; truncate names to what fits so a long
    // one (e.g. `com.apple.WebKit.WebContent`) clips with an ellipsis rather than
    // spilling into the numbers. ~7px per glyph at the table's 13px, minus padding.
    let name_px = (card_w - 28.0 - 2.0 * NUM_W - 16.0).max(40.0);
    let name_budget = (name_px / 7.0) as usize;

    let arrow = move |c: Col| -> &'static str {
        if sort_col == c {
            if desc {
                " ▾"
            } else {
                " ▴"
            }
        } else {
            ""
        }
    };
    let header_cell = |label: &str, col: Col, width: Length, right: bool| -> Element<'_, Message> {
        let color = if sort_col == col { t.ink } else { t.muted };
        let txt = text(format!("{label}{}", arrow(col))).size(11).color(color);
        let mut c = container(txt).width(width).padding([0, 8]);
        if right {
            c = c.align_x(iced::alignment::Horizontal::Right);
        }
        mouse_area(c)
            .on_press(Message::SetProcSort(col))
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    };
    let header = row![
        header_cell("PROCESS", Col::Name, Length::Fill, false),
        header_cell("CPU", Col::Cpu, Length::Fixed(NUM_W), true),
        header_cell("MEM", Col::Mem, Length::Fixed(NUM_W), true),
    ]
    .align_y(iced::Alignment::Center);

    // Sorted-order metadata for the callbacks: (pid, name) so a right-clicked row
    // opens its context menu, and the CPU% per row so a hog can be graded a color.
    let row_meta: Vec<(i32, String)> = order
        .iter()
        .map(|&i| (procs[i].pid, procs[i].name.clone()))
        .collect();
    let cpu_by_row: Vec<f32> = order.iter().map(|&i| procs[i].cpu_percent).collect();
    // Grade the CPU% cell by the same cutoffs as the CPU status-bar cell, so a
    // busy process reads amber (>=60%) / red (>=85%) at a glance.
    let (warn, alarm, _) = crate::settings::MetricKind::Cpu
        .default_thresholds()
        .unwrap_or((60.0, 85.0, false));
    let (warn, alarm) = (warn as f32, alarm as f32);

    // The virtualized body. The cell closure owns the sorted order and borrows the
    // process list.
    let rows = order.len();
    let cell = move |row: usize, col: usize| -> String {
        let p = &procs[order[row]];
        match col {
            0 => truncate_name(&p.name, name_budget),
            1 => format!("{}%", p.cpu_percent.round() as i32),
            _ => crate::metrics::format_bytes(p.memory_bytes),
        }
    };
    let cell_color = move |row: usize, col: usize| -> Option<iced::Color> {
        if col != 1 {
            return None;
        }
        match grade(cpu_by_row[row], warn, alarm, false) {
            Grade::Calm => None,
            g => Some(grade_color(g)),
        }
    };
    let columns = vec![
        TableColumn::fill("").align(CellAlign::Left),
        TableColumn::fixed("", NUM_W).align(CellAlign::Right),
        TableColumn::fixed("", NUM_W).align(CellAlign::Right),
    ];
    let body = table(rows, columns, cell)
        .cell_color(cell_color)
        .metrics(TableMetrics {
            row_height: 22.0,
            header_height: 0.0,
        })
        .offset(state.proc_table_scroll)
        .on_scroll(Message::ProcTableScroll)
        // Right-click opens the row's context menu (View fds / Copy path / PID /
        // name); it is the way into the per-process detail.
        .on_right_click(move |row| {
            let (pid, name) = &row_meta[row];
            Message::ProcRowRightClick(*pid, name.clone())
        });

    column![
        text(format!("Processes — {}", rows)).size(14).color(t.ink),
        header,
        container(body)
            .height(Length::Fixed(chart_h))
            .width(Length::Fill),
    ]
    .spacing(6)
    .into()
}

/// One process's detail, shown in place of the process list when a row is
/// double- or right-clicked: a "‹ Back" control, a live CPU% chart (its history
/// is fresh for this process — we do not retain a series for every process), the
/// current memory / thread count, and the scrollable list of open file
/// descriptors. Refreshed each sample by [`crate::metrics::Metrics::sample_proc_detail`].
fn proc_detail_body(state: &Tty, chart_h: f32) -> Element<'_, Message> {
    use prexp_core::models::ResourceKind;
    let t = theme::tokens();
    let Some(d) = state.metrics.proc_detail.as_ref() else {
        // The process exited between opening and rendering; offer the way back.
        return column![
            back_row("‹ Back"),
            text("That process is no longer running.")
                .size(12)
                .color(t.muted),
        ]
        .spacing(8)
        .into();
    };

    // A "‹ Back" control, then the process identity.
    let head = column![
        back_row(&format!("‹ {}", truncate_name(&d.name, 28))),
        text(format!("pid {} · {} threads", d.pid, d.thread_count))
            .size(11)
            .color(t.muted),
    ]
    .spacing(2);

    // The live CPU chart. It needs a couple of points to draw a segment; until
    // then (just opened) it shows a note.
    let cpu_now = d.cpu_history.back().copied().unwrap_or(0.0);
    let chart: Element<'_, Message> = if d.cpu_history.len() >= 2 {
        let series = vec![Series {
            points: d
                .cpu_history
                .iter()
                .enumerate()
                .map(|(i, &v)| (i as f64, v as f64))
                .collect(),
            color: t.accent,
        }];
        let peak = d.cpu_history.iter().copied().fold(1.0_f32, f32::max);
        line_chart(
            LineChart {
                title: "CPU".to_string(),
                series,
                y_max: None,
                y_max_label: Some(format!("{}%", peak.round() as i32)),
                hover_format: Some(hover_percent),
            },
            (chart_h * 0.42).max(48.0),
        )
    } else {
        container(text("Collecting CPU…").size(12).color(t.muted))
            .height(Length::Fixed((chart_h * 0.42).max(48.0)))
            .into()
    };

    let stats = text(format!(
        "CPU {}%   ·   Mem {}",
        cpu_now.round() as i32,
        crate::metrics::format_bytes(d.memory_bytes),
    ))
    .size(13)
    .color(t.ink);

    // The file descriptors: a summary count line, then a scrollable list. When the
    // OS denied fd access we say so instead of showing an empty list.
    let files = d
        .resources
        .iter()
        .filter(|r| r.kind == ResourceKind::File)
        .count();
    let sockets = d
        .resources
        .iter()
        .filter(|r| r.kind == ResourceKind::Socket)
        .count();
    let fd_head = text(format!(
        "Open files — {} ({} files, {} sockets)",
        d.resources.len(),
        files,
        sockets,
    ))
    .size(12)
    .color(t.muted);

    let fd_list: Element<'_, Message> = if !d.accessible {
        text("fd access denied by the OS for this process.")
            .size(12)
            .color(t.muted)
            .into()
    } else if d.resources.is_empty() {
        text("No open file descriptors.")
            .size(12)
            .color(t.muted)
            .into()
    } else {
        let mut list = column![].spacing(1);
        for r in &d.resources {
            let kind = resource_kind_label(&r.kind);
            let path = r.path.as_deref().unwrap_or("—");
            let line = container(
                text(format!("{:>3}  {:<6} {}", r.descriptor, kind, path))
                    .size(11)
                    .color(t.muted)
                    .font(iced::Font::MONOSPACE),
            )
            .width(Length::Fill);
            // Right-click a descriptor with a path to copy it — the whole row is
            // the hit target (not just the glyphs).
            list = list.push(match &r.path {
                Some(p) => mouse_area(line)
                    .on_right_press(Message::FdRowRightClick(p.clone()))
                    .interaction(iced::mouse::Interaction::Pointer)
                    .into(),
                None => Element::from(line),
            });
        }
        scrollable(list).height(Length::Fill).into()
    };

    column![
        head,
        chart,
        stats,
        fd_head,
        container(fd_list).height(Length::Fill).width(Length::Fill),
    ]
    .spacing(6)
    .into()
}

/// A clickable "‹ Back" row that returns the Processes drill-in to the list.
fn back_row(label: &str) -> Element<'static, Message> {
    mouse_area(text(label.to_string()).size(14).color(theme::tokens().ink))
        .on_press(Message::CloseProcDetail)
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

/// A short label for a file-descriptor kind, for the process detail's fd list.
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

/// One invisible resize strip for [`with_resize_edges`]: a transparent hit area
/// of the given size whose press starts a drag-resize from `edge` (tracked
/// through `PointerMoved`, ended by `PointerReleased`), showing the matching
/// resize cursor on hover. It carries no paint of its own; the card's own border
/// is the visible edge the user grabs.
fn resize_strip<'a>(
    index: usize,
    edge: crate::state::ResizeEdge,
    width: Length,
    height: Length,
    cursor: iced::mouse::Interaction,
) -> Element<'a, Message> {
    mouse_area(iced::widget::Space::new().width(width).height(height))
        .on_press(Message::MetricDetailResizeStart(index, edge))
        .interaction(cursor)
        .into()
}

/// Overlay thin invisible resize strips along the card's right and bottom edges
/// and its bottom-right corner, so the popover resizes by dragging its borders
/// (each with the matching resize cursor) rather than a separate grip. The strips
/// stack over the card; `iced`'s `stack` sizes to its first child, so they span
/// exactly the card, not the window. `index` is the popover the drag addresses.
fn with_resize_edges(index: usize, card: Element<'_, Message>) -> Element<'_, Message> {
    use crate::state::ResizeEdge;
    use iced::alignment::{Horizontal::Right, Vertical::Bottom};
    use iced::mouse::Interaction;

    const EDGE: f32 = 8.0;
    const CORNER: f32 = 16.0;

    let right = container(resize_strip(
        index,
        ResizeEdge::Right,
        Length::Fixed(EDGE),
        Length::Fill,
        Interaction::ResizingHorizontally,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Right);
    let bottom = container(resize_strip(
        index,
        ResizeEdge::Bottom,
        Length::Fill,
        Length::Fixed(EDGE),
        Interaction::ResizingVertically,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(Bottom);
    let corner = container(resize_strip(
        index,
        ResizeEdge::Corner,
        Length::Fixed(CORNER),
        Length::Fixed(CORNER),
        Interaction::ResizingDiagonallyDown,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Right)
    .align_y(Bottom);

    iced::widget::stack![card, right, bottom, corner].into()
}

/// Hover-readout formatters for [`metric_popover_card`]'s chart, as plain `fn`
/// pointers `rime`'s `LineChart` can hold: a percentage for CPU/memory, a
/// throughput rate for the network/disk series.
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


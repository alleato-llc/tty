# Idea: machine stats in the status bar

Surface live machine metrics (CPU, memory, network, disk) in tty's bottom
status bar, configurable, rendered as compact sparklines so a glance shows both
*now* and the recent *trend* in the one line the bar has to spare. Status: a
design sketch to refine, not built. The decisions locked so far are marked
**[chosen]**; the open ones are in the last section.

This leans on three things that already exist, so it is mostly wiring rather
than new machinery:

- **`fdtop`'s `prexp-core`** (sibling repo) already samples the system
  cross-platform: `cpu_ticks() -> Vec<CpuTicks>` (per-core user/sys/idle/nice)
  and `memory_info() -> MemoryInfo` (total/used/free/wired/compressed), macOS
  via native libproc FFI (`prexp-ffi`), Linux via `procfs`. CPU% is a delta of
  tick counts between two samples, top-style. **[chosen]** tty consumes it as a
  path dependency, matching the `dorado-engine` / `rime` reuse pattern, rather
  than pulling `sysinfo`.
- **`rime`'s chart kernel** (`widgets/chart.rs`: a Canvas `line_chart` /
  `Series` / `LineChart`) already draws stroked polylines against the live iced
  theme. A small `sparkline()` helper beside it gives the compact filled form.
- **tty already ticks** (`iced::time::every(120ms)` for output coalescing in
  `subscription.rs`), so a slower metrics-sample subscription is idiomatic.

## Representation: canvas sparklines **[chosen]**

Four rungs of ambition were on the table; we picked the richest:

| Style | Cost | Notes |
|---|---|---|
| Numeric text | none | renders in today's string status bar |
| Unicode block gauge (`███░░`) | none | fdtop's look, pure text |
| Braille sparkline (`▁▂▃▅▇`) | none | trend in ~8 glyphs, pure text |
| **Canvas sparkline [chosen]** | rebuild the bar | real filled mini line-charts, color-by-load |

Canvas sparklines mean the status bar can no longer be the string-based
`rime::status_bar(left, right)`; it becomes a proper Element row that mixes text
segments with small `Canvas` widgets (each metric a ~40px filled line tinted by
its current load). This is the one real structural cost of the choice, and it is
also what unlocks the ambient-peek idea below.

Each metric renders as: a short label, a fixed-width sparkline of its history,
and the current value. Color encodes load (calm accent → warning → danger as it
climbs), read from the live theme at paint time (the chart kernel already takes
the theme this way, not `tokens()`, to stay valid after the palette scope drops).

## Metrics: CPU, memory, network, disk **[chosen]**

| Metric | Source | Status |
|---|---|---|
| CPU % (aggregate) | `prexp-core` `cpu_ticks`, summed then delta'd | ready in prexp-core |
| Memory (used/total, %) | `prexp-core` `memory_info` | ready in prexp-core |
| Network rate (rx/tx) | `prexp-core` `network_counters`, delta'd per interval | added (macOS; Linux stub) |
| Disk I/O rate (r/w) | `prexp-core` `disk_counters`, delta'd per interval | added (macOS; Linux stub) |

Network and disk throughput were added to `prexp-core` as cumulative
system-wide byte counters (`network_counters` / `disk_counters`), which tty
diffs per interval into a rate. On macOS the samplers read `sysctl`
(`NET_RT_IFLIST2`, summed over non-loopback interfaces) and IOKit
(`IOBlockStorageDriver` statistics) via `prexp-ffi`. The Linux backend
(`/proc/net/dev` + `/proc/diskstats`) is a tracked follow-up: it returns an
"unsupported" error today, which tty drops gracefully (the metric shows no
rate rather than a fabricated zero). CPU and memory work with prexp-core as-is.
Per-core CPU is deliberately *not* in the bar (too wide) — it belongs to the
hover/overlay view below.

## Configurability

An ordered list in `tty.settings.json` — order is display order, empty = off:

```json
"status_bar_metrics": [
  { "metric": "cpu",    "style": "sparkline" },
  { "metric": "mem",    "style": "sparkline" },
  { "metric": "net_rx", "style": "rate" },
  { "metric": "disk_w", "style": "rate" }
],
"status_bar_metrics_interval_ms": 2000
```

`metric` ∈ `cpu | mem | swap | load | net_rx | net_tx | disk_r | disk_w`
(battery/temp later). `style` picks the per-metric render (`sparkline`, `gauge`,
`number`, `rate`). When the bar runs out of width it sheds metrics from the
right before anything wraps, so a narrow window degrades gracefully.

Implemented today (phases 2-3): the ordered list,
`status_bar_metrics_interval_ms`, and width-shedding are live. `metric` accepts
`cpu`, `mem`, `net_rx`, `net_tx`, `disk_r`, `disk_w`, `net_io`, and `disk_io`
(net/disk on macOS only for now). `net_io` / `disk_io` overlay the two directions
(rx+tx, read+write) on a single sparkline — two series on a shared scale, the
first in the accent and the second in `warn`, via rime's multi-series
`Sparkline` — with both rates in the label. `style` accepts `sparkline` and `number` and applies to any metric; the
metric's *kind* decides its value domain, so a network/disk cell's value is a
byte-rate (its label a formatted throughput, its sparkline auto-scaled to its
own recent peak and tinted with the neutral accent) while CPU/memory grade by
load against 100%. The sketch's separate `rate`/`gauge` style names collapse
into this: they parse leniently to the sparkline form. Unknown `metric` values
are parsed leniently and dropped, so a config written by a newer build never
breaks an older one. Remaining: `swap`/`load` metrics and the Linux net/disk
samplers.

## Data flow

```mermaid
flowchart LR
    TICK["iced::time::every(interval)<br/>(new subscription)"]
    SsampleMsg["Message::SampleMetrics"]
    SAMPLE["metrics::sample()<br/>prexp-core cpu_ticks + memory_info<br/>+ net/disk counters"]
    DIFF["diff vs last sample<br/>→ CPU%, mem%, rx/s, tx/s, r/s, w/s"]
    RING["per-metric ring buffers<br/>in Tty state (cap ~60 samples)"]
    VIEW["status bar Element<br/>label + sparkline Canvas + value"]

    TICK --> SsampleMsg --> SAMPLE --> DIFF --> RING --> VIEW
```

Sampling holds the previous raw counters to compute deltas; the ring buffers are
bounded (~60 points ≈ 2 minutes at 2s) so memory is fixed. Sampling only runs
when at least one metric is enabled, and can slow (or pause) when the window is
unfocused to stay battery-friendly.

## Fitting the auto-hiding bar: the ambient peek

The status bar now auto-hides by default (it floats in on near-hover). Metrics
hidden until you hover would lose the at-a-glance value, so the synthesis:

- **Ambient peek line.** While the bar is auto-hidden, draw a 1–2px hairline
  along the very bottom edge, segmented and tinted by load (e.g. left third =
  CPU, middle = MEM, right = net/disk). It is an always-on ambient gauge that
  costs almost no space; nearing the bottom still expands to the full stats bar.
  Auto-hide and metrics reinforce each other instead of competing.

Fallbacks if the peek line is more than we want: enabling any metric could pin
the bar visible, or a dedicated always-visible micro-strip could carry only the
metrics separate from the auto-hiding text. The peek line is the preferred path.

## The endgame (out of scope for v1)

Clicking a metric cluster opens a **System overlay** — essentially a mini-fdtop
in tty's own chrome: per-core CPU bars, a memory breakdown, and top processes by
CPU/mem, reusing much more of `prexp-core`. "htop lives in your terminal chrome,
one click away." A north star, not a v1 commitment. The single-metric drill-in
popover (phase 5) is the first, lightweight cut of this: clicking one cell for
that metric's full-size chart, rather than a whole system panel.

## Dependency + cross-platform notes

- New path dependency on `prexp-core` (pulls `prexp-ffi` on macOS, `procfs` on
  Linux) — **approved**. It keeps the reuse-the-siblings pattern and avoids
  `sysinfo`.
- `prexp-core` covers macOS + Linux, matching tty's targets. The net/disk
  samplers are written for macOS; the Linux backend is a tracked follow-up
  (it returns an "unsupported" error until then).
- All sampling is read-only and cheap at a 2s cadence; no privileges beyond what
  fdtop already uses.

## Phasing

1. **[done] Data path + numeric CPU/MEM**, behind an off-by-default setting,
   `prexp-core` sampling at 2s. `tty/src/metrics.rs` (sampler + pure helpers +
   an ignored live check), a `Metrics` field on `Tty`, the `SampleMetrics` tick,
   and the `status_bar_metrics_enabled` toggle.
2. **[done] Status bar rebuilt as an Element, with the config list and
   width-shedding.** New `rime` `sparkline()`/`Sparkline` (a filled area canvas)
   and `status_bar_content` (the styled strip around arbitrary content);
   `Metrics` grew bounded CPU%/MEM% history; the bar renders each configured
   metric as a color-graded canvas sparkline (calm/caution/alarm from the
   theme's success/warn/danger) or a plain number. The single on/off toggle
   became the ordered `status_bar_metrics` list — `{ metric, style }` entries
   edited in Settings (add, remove, reorder, per-metric style), with a
   `status_bar_metrics_interval_ms` sample cadence. When the tracked window
   width can't hold every cell, the bar sheds metrics from the right before
   anything wraps (`view::visible_metric_count`). The no-stats bar stays
   pixel-identical to the old text footer.
3. **[done, macOS] Network + disk samplers.** `prexp-core` grew
   `network_counters()` / `disk_counters()` (cumulative system-wide byte
   counters), read on macOS via new `prexp-ffi` bindings — `sysctl`
   (`NET_RT_IFLIST2`, non-loopback interfaces) and IOKit (`IOBlockStorageDriver`
   statistics). tty's `Metrics` diffs them per interval into per-second rates
   (`net_rx`/`net_tx`/`disk_r`/`disk_w`), each a bounded rate history rendered as
   an auto-scaled, accent-tinted sparkline with a formatted throughput label.
   The Linux backend returns an "unsupported" error (a tracked follow-up); tty
   drops the failed read so the metric simply shows no rate.
4. **Ambient peek line** for the auto-hidden state.
5. **[done] Single-metric drill-in popover.** Clicking a status-bar sparkline
   opens a small card, bottom-centered over the bar, with that metric's full-size
   `rime` `line_chart` over its retained history (the same series and shared scale
   the cell used, with a peak y-axis label in the metric's units), its current
   readout, a two-line legend for the combined I/O metrics, and a sample-count
   caption. A metric whose history isn't chartable yet (a rate metric with no
   sampler on this platform, or the first ticks after opening) still shows the
   card, with a "collecting" note in place of the chart, so the drill-in always
   gives visible feedback. A transparent click-away layer or Escape dismisses it
   (`opaque` on the card so clicking it doesn't close). `Tty::metric_detail` holds
   the drilled-in `MetricKind`; `view::metric_detail_popover` builds the card and
   `main_view` stacks it (over the chrome, under the settings/scrollback layers).
   `rime`'s `LineChart` grew an optional `y_max_label`. A lighter first cut of the
   mini-fdtop overlay below.
   - **Expand.** A `ghost` "Expand" / "Collapse" button floats over the chart's
     top-right (`Tty::metric_detail_expanded` + `ToggleMetricDetailExpanded`).
     Expanded is a large centered card whose chart is sized off the window
     geometry; compact is the small bottom-anchored card. Reset to compact on
     each open / close / Escape.
   - **Hover.** `rime`'s `LineChart` reads the cursor it is already handed in
     `draw` (no state, no message): over the plot it snaps to the nearest sample,
     draws a vertical guide, and marks + labels each series' value there in the
     metric's units (a new `hover_format: Option<fn(f64) -> String>`, a plain `fn`
     so the widget stays generic). Works in the compact and expanded charts alike.
   - **Resize.** A corner grip on the compact card drag-resizes it
     (`metric_detail_size` + `metric_detail_resize`), mirroring the tab-drag
     mechanism: the grip's press starts the drag, `PointerMoved` tracks the delta,
     `PointerReleased` ends it. Clamped to stay usable and on-screen; Expand still
     overrides to the window-sized card.
   - **Peak-scaled axis.** The chart auto-scales its y axis to the data's own
     peak, so the axis label reads that peak in the metric's units rather than the
     metric's fixed gauge max — a CPU/memory chart no longer shows a misleading
     "100%" at the top while its line sits at the real (lower) peak.
   - **Per-core CPU grid.** CPU drills into a grid of small per-core sparklines
     (each color-graded by its current load) instead of the single aggregate
     chart, grouped into **Performance** / **Efficiency** sections. Per-core %
     history lives in `Metrics::core_history`; the P/E split comes from
     `prexp-core`'s `cpu_perf_levels()` (each core's IORegistry `cluster-type`,
     read once and cached), falling back to one ungrouped section where the
     platform reports no perf levels. The aggregate sparkline stays in the inline
     status-bar cell.
6. **System overlay** (mini-fdtop) on click.

## Open questions

- Sparkline window: fixed 2 minutes, or a setting? Fixed width in px, or grow
  with available space?
- Color-by-load thresholds: shared with the theme's accent/warning/danger, or
  their own scale?
- Unfocused behavior: pause sampling, slow it, or keep it (for the peek line)?
- Does the peek line belong to this feature or to the status-bar auto-hide
  feature it visually extends?
- Do we want per-metric `style` at all in v1, or is "sparkline for gauges, rate
  for throughput" a fine fixed mapping until someone asks?

## Pointers

- Reuse: `../../fdtop/crates/prexp-core` (system sampling),
  `../../rime/rime/src/widgets/chart.rs` (the chart kernel),
  `tty/src/subscription.rs` (the existing tick), `tty/src/view.rs`
  `status_text` / the `status_bar` call site.
- Related: the status-bar auto-hide (ADR/README) this extends.

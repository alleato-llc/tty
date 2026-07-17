# Architecture

tty is a tabbed terminal in three layers, split across three crates. Data flows
**shell → engine → widget → screen**, and repaints are driven by output rather than
a clock.

```
            ┌─────────────────────── tty (app) ───────────────────────┐
            │  state · update · view · subscription · theme · settings │
            └───────────────┬───────────────────────┬─────────────────┘
         renders with       │                        │  chrome (tabs, status bar)
                            ▼                        ▼
                     phosphor (widget)            rime (../rime/rime)
                            │  renders / measures
                            ▼
                     cathode (engine):  parser → screen ← pty
                            ▲                              │
                            └──────── shell (PTY) ─────────┘
```

## cathode — the engine (no iced)

- **`pty`** — `PtySession::spawn(shell, cols, rows)` opens a real PTY via
  `portable-pty`, returning the session (write/resize) and a tokio channel of output
  bytes. The app runs a background read loop on this channel.
- **`parser`** — a `vte`-based VT/ANSI parser; `process(bytes, &mut screen)` applies
  SGR colors/attributes, cursor motion, erase, scroll regions, etc.
- **`screen`** — `TerminalScreen`: the grid of `Cell`s (char + fg/bg + bold/italic/
  underline/dim/inverse), the cursor, a configurable **`scrollback`** (evicts the
  oldest line past `max_scrollback`; `set_max_scrollback` truncates live so a lowered
  setting applies to an already-open tab, not just new ones), and a **`command_log`** —
  a bounded `VecDeque<CommandEntry>` (command text + its captured output + a
  timestamp) built from a queue of `pending_boundaries`: the host calls
  `mark_command_boundary(cap)` right before forwarding a real Enter to the shell
  (queued as "derive the text from whatever row completes next" — nothing else can
  race between a real Enter and its own echo), or `mark_command_boundary_with(text,
  cap)` for an already-known line (an unbracketed multi-line paste, one call per
  complete pasted line, queued *before* any of it is sent — since several can be
  in flight before their echoes arrive, each is matched by its completing row
  actually *ending with* that known text, not just "whichever row completes next").
  Either way, `advance_row()`'s `record_output_line` resolves the front of the queue
  into a real `CommandEntry` once seen, then feeds subsequent lines into it until it
  hits the command's own `max_output_lines` cap — so a streaming command (`tail -f`)
  just stops growing instead of recording forever. The live grid (and command
  boundaries) are skipped entirely while on the
  **alt screen**, so full-screen apps (htop, vim) never pollute scrollback or
  history. `resize(cols, rows)` reflows.
- **`commands`** — `glob_match(pattern, text)` (a `*`/`?` backtracking matcher) and
  `resolve_output_cap(command, overrides, default)` — the shared implementation both
  tty and fed-ide use to resolve a command's output-line cap from the first matching
  glob override (e.g. `"tail *" → 200`), falling back to the configured default. See
  [ADR 0004](adr/0004-scrollback-history.md) for why command/output separation is
  built this way instead of shell-integration (OSC 133) or a position-tagging scheme.
- **`history`** — the pure data side of **encrypted history** (ADR 0006):
  `PersistedCommandEntry` (id + command text + wall-clock timestamp + pane tag —
  never output) and `HistoryEvent::{Upsert, Tombstone}`. `TerminalScreen` queues an
  event whenever `command_log` mutates by explicit user action (new command, Clear,
  Delete) into `pending_history_events`, drained by the host; window eviction and a
  RIS reset queue **nothing** (eviction is not deletion). `seed_command_log` loads
  persisted entries back into the live window at startup. cathode knows nothing of
  crypto or files — that all lives in the app.
- **`wake`** — a process-global signal channel. The read loop calls
  `wake::signal()` after each parse (and on shell exit); the UI's subscription awaits
  it. This is what makes repaint **output-driven**: no idle polling, zero cost when
  the shell is quiet. `take_receiver()` hands the single receiver to the app once.

cathode has no UI dependency, so it can be embedded by any front-end.

## phosphor — the widget

A stateful custom `iced` `Widget` over an `Arc<Mutex<TerminalScreen>>` (shared with
the read loop):

- Renders cells with color + text attributes, paints a block cursor, and draws
  selection tint. Foreground runs are shaped via `renderer.fill_text` with finite
  bounds; underlines are thin quads.
- Holds its own view state: how far it's **scrolled back**, and an in-progress
  **mouse selection** (reported to the host as text for `⌘C`).
- Measures how many cols/rows fit at the current font and reports it, so the host can
  resize the PTY (a real SIGWINCH).
- Self-styles from a plain `TerminalStyle` (16 ANSI colors + fg/bg/cursor/selection),
  so it carries **no theme-crate dependency**. `ANSI_DEFAULT` /
  `TerminalStyle::default_dark()` give a starting palette.
- **`input`** — `to_bytes(key, mods)` translates an iced key press into the bytes a
  PTY expects (control codes, arrow escapes), so a focused terminal behaves like one.
  Modified arrows/backspace map to the readline/zsh line-editing bindings: `⌥`/`Ctrl`
  move or delete by word (Meta-b/f, `ESC ⌫`), `⌘` jumps to / deletes to line start/end
  (Ctrl-A/E/U).
- **`find_matches(screen, cols, query)`** — a pure substring scan over the *whole*
  transcript (scrollback + live grid), not just whatever's currently scrolled into
  view, so the host can drive a live "N of M" count and next/previous navigation.
  `.scroll_to(Option<usize>)` is an edge-triggered builder prop: it brings a target
  line into view exactly once per distinct target (tracked against a cached
  `last_scroll_to` in widget state), so it doesn't fight a subsequent manual scroll.
  `draw()`'s match highlight stays viewport-scoped (only visible rows are ever
  drawn) but is backed by this same list, not a separate scan.
- **`link`** — `link_at(row, col)` detects a URL under a cell, trimming a trailing
  closing punctuation/quote so it isn't captured as part of the link. `⌘`-hover
  underlines the URL under the pointer as an affordance; `⌘`-click opens it directly
  (`on_open_link`, no menu); a plain right-click over a link opens an **Open Link** /
  **Copy Link** menu instead (`on_link`).

## tty — the app

Thin glue, mirroring `fed`'s module shape:

- **`state`** — `Tty { tabs: Vec<Tab>, active, theme, font, font_size, … }`. A `Tab` is
  a `pane_grid::State<Pane>` split tree plus its focused `Pane`. `Pane` is the
  per-pane content enum (the extension point): `Pane::Term(Term)` for a shell, or
  `Pane::Metric(MetricKind)` for a metric drill-in "graduated" from a floating
  popover into a real pane (via `promote_metric_to_pane`) — no PTY, reads the shared
  `Metrics`, never reaps. A `Term` is a `screen` + an `Option<PtySession>` (`None`
  only in tests) + an `alive` flag the read loop clears on shell exit. Terminal
  operations filter to `Pane::as_term` (and `Tab::terms()/terms_mut()`), so a metric
  pane is transparent to keystrokes/resize/reaping while sharing the generic
  split/focus/resize/maximize machinery. Methods target the active tab's focused
  pane: `split_focused`/`split_with`/`focus_dir`/`close_focused_pane`/`close_pane`,
  `toggle_maximize_pane`, `write_focused`/`write_pane`, `resize_pane`,
  `new_tab`/`close_tab`, `zoom`/`reset_zoom`. `reap_dead` drops dead terminal panes,
  then any tab with no live pane, then exits when none remain.
- **`update`** — app **chords use ⌘** (`Modifiers::command()`) so `Ctrl` stays a real
  terminal control code: `⌘T`/`⌘N`/`⌘W`, `⌘1`–`⌘9`, `⌘±`/`⌘0`, `⌘C` copy, `⌥⌘`+arrows
  split / `⌃⌘`+arrows move focus. `⌘F` opens the scrollback find bar (`Enter`/`⇧Enter`
  step to the next/previous match, driving `phosphor`'s `.scroll_to`), `⌘K` clears the
  focused pane's scrollback (`command_log` included), and `⌘⇧H` toggles the
  **Scrollback History** panel — all three main-window-only, mirroring `⌘,`'s scope.
  Everything else becomes PTY input via `phosphor::input`.
- **`view`** — window-aware: `root_view(state, window)` routes a **detached** window to a
  lean `detached_view` (the tab's `pane_grid` + a Reattach button + a status bar) and every
  other window to `main_view` — the `rime` `tabs` strip (shown only with >1 tab, matching
  fed / fed-ide), a `pane_grid` over the active tab's panes (the closure matches the
  `Pane`: a `phosphor` terminal, or `metric_pane_content` — a header with maximize/close
  over `metric_body`, the same body a drill-in popover renders; the focused one's border
  turns accent only when the tab has >1 pane), and a `rime` `status_bar`. A right-click (or `⌃`-click, macOS's secondary-click) opens a `rime`
  `context_menu` at the tracked pointer — a pane menu (split + close pane), a tab menu
  (new tab + rename + **detach** + split + close tab), or — if the click landed on a
  detected URL — a **link** menu (Open Link / Copy Link), per `state.menu`. "Rename tab"
  shows a focused field under the strip; `Tab::label()` resolves a tab's display name
  (custom → program title → shell). Pane messages carry the originating `window::Id` so a
  click / resize / selection routes to the right tab. The **Scrollback History** panel
  (`⌘⇧H`) renders a rime `table`: each `command_log` entry is a row, expandable (a
  flattened per-render row list, not variable-row-height support in `table` itself) to an
  accordion of its captured output lines; a text filter narrows the list, double-clicking
  a row copies its command, and a single `stat("Commands", …)` reports the shown/filtered
  count. **Machine stats** live on the status bar: each `settings.status_bar_metrics`
  entry (`{ metric, style, warn?, alarm? }`, resolved leniently via
  `status_bar_metrics_indexed` so an unknown metric is skipped, not fatal) renders as a
  `rime` `sparkline` or a plain number. The graded cells (CPU, memory, battery) color
  by a `grade` against per-cell **warn/alarm thresholds** (configurable, per-kind
  defaults) and, when past a threshold, recolor the whole cell including its label
  (`MetricRender::alert`); network/disk rates auto-scale to their recent peak (Net/Disk
  I/O overlay two series on one scale); `Uptime` / `Session` / `Clock` are text cells;
  `Load` sparklines the 1-minute load; `Battery` is a fixed 0..100% gauge.
  `visible_metric_count` sheds cells from the right when the tracked window width can't
  hold them all; a **wheel scroll** over the bar slides a window (`Tty::status_bar_scroll`,
  clamped by `status_bar_scroll_max`, `‹`/`›` chevrons) through the shed cells. A cell
  **tap** opens its drill-in (`open_metric_detail` on release); a **press-hold** past
  `status_bar_edit_hold_secs` enters drag-to-reorder **edit mode** (`status_bar_edit`):
  cells outline, the dragged one lifts, an insertion bar marks the drop
  (`status_metric_drag` / `status_metric_drop`), and the reorder commits on release.
  Opening a drill-in pushes a `MetricPopover` (metric + per-popover expand / size /
  position) onto `Tty::metric_details`; `metric_popover_card` builds each and `main_view` places them
  (bottom-centered over the bar, cascaded when several are open). The card holds the
  metric's full-size `rime` `line_chart` over its retained history — or a "collecting"
  note when the history isn't chartable yet — with a "+" / "−" expand affordance
  (compact is bottom-anchored, expanded is a large centered card sized off the window)
  and border-resize strips (`with_resize_edges`, one `ResizeEdge` per side/corner);
  hovering a point reads its value off the chart (`LineChart::hover_format`). CPU has
  three drill-in variants keyed by `MetricKind` (`Cpu` aggregate / `CpuCores` grid /
  `CpuAll` both) that share one status-bar cell. By default one popover is open at a
  time and an `opaque` click-away backdrop fires `CloseMetricDetail`; with
  `status_bar_metrics_pinned` on, several stay open (each with a `CloseMetricPopover`
  "×", no backdrop). `Esc` closes all (checked before the other overlays).
- **`metrics`** — the status-bar sampler: `Metrics::sample()` reads CPU ticks + memory
  (required) and network / disk byte counters (optional) from `fdtop`'s `prexp-core`,
  folds the aggregate CPU% from tick deltas and the throughput rates from byte-counter
  deltas over the `Instant`-measured interval, and keeps a bounded per-metric history
  for the sparklines. It also keeps **per-core** CPU% history (`core_history`) for the
  CPU drill-in's per-core grid, plus each core's cached P/E `perf_levels` (static, read
  once from `prexp-core`'s `cpu_perf_levels()`). Memory now also carries **swap**
  (`prexp-core`'s `MemoryInfo.swap_*` — `sysctl(vm.swapusage)` on macOS, `/proc/meminfo`
  on Linux), shown as a line in the Memory drill-in. It tracks two uptimes
  (`system_uptime_secs` / `session_uptime_secs`): the system boot time is read once
  via `prexp-core`'s `system_boot_time_secs()` (`sysctl(KERN_BOOTTIME)` on macOS,
  `/proc/uptime` on Linux) and the session start is stamped on the first sample; the
  `Uptime` / `Session` kinds render as text cells (abbreviated `up 3d 4h`, drilling
  into a full breakdown). It also samples the **load average**
  (`system_load_average()` — `getloadavg(3)` / `/proc/loadavg`) and the **battery**
  (`system_battery()` — IOKit power sources / `/sys/class/power_supply`, hidden with no
  battery). The `Clock` cell is the live wall time (its own 1s tick, no sampler).
  For the **Processes** cell, `sample_processes()` folds a per-pid CPU% from
  `cpu_time_ns` deltas and reads each process's physical footprint (bytes) over
  `prexp-core`'s light `process_summaries()` (every pid, *no* fd enumeration); it
  runs only while a Processes cell is shown, since it walks the whole table. The
  cell shows the busiest process; `procs_body` renders the drill-in — a clickable
  header (re-sort by CPU, absolute memory, or name) over a virtualized, scrollable
  `rime` `table` (`proc_sort` / `proc_table_scroll`), names truncated to the fill
  column and the CPU% cell graded a color via the table's `cell_color` hook (the
  shared `grade`/`grade_color` at the CPU cell's 60/85 cutoffs). Right-clicking a
  row opens a `MenuKind::ProcRow` context menu (the app's one global `context_menu`
  overlay, anchored at the pointer): **View Process** →
  `OpenProcDetail`, plus copy actions. **Copy path** resolves lazily via
  `metrics::process_path` (prexp-core's light `process_path(pid)`) so the list
  never pays for paths; PID/name reuse `CopyText`. `proc_detail_body` (opened only
  from that menu) shows the one process: `sample_proc_detail()` reads it fully via
  `snapshot_pid` (which *does* enumerate fds) each tick, folding a CPU% history
  kept only while that process is open (`Metrics::proc_detail`, reset on each open
  — we never retain a series per process); the fd rows are each right-click-to-copy
  (`MenuKind::FdRow`). `proc_detail_pid` gates the sampling and the view, and
  `Esc` / "‹ Back" clears it.
  Network / disk have macOS samplers only for now (via `prexp-ffi` — `sysctl
  NET_RT_IFLIST2` + IOKit `IOBlockStorageDriver`); on other platforms those reads error
  and are dropped, so the metric simply shows no rate. A failed CPU/memory read is
  warned and skipped — a stats hiccup never disturbs the terminal.
- **`subscription`** — key events + per-window geometry (`Focused`/`Resized`/`Moved` via
  `listen_with`'s window id) + `window::close_events` + **one always-on output stream** fed
  by `cathode::wake` (drains an output burst into a single redraw; also reaps dead tabs).
  While a detached window is settling after a drag, a short-lived timer polls the drag-dock
  debounce.
- **`history`** — the app half of **encrypted history** (ADRs 0006/0007/0008; the
  full key-derivation pipeline and its open refinement options are surveyed in
  `docs/history-keys.md`), opt-in
  and off by default: `crypto` (AEAD wrap/unwrap, a self-describing `cipher_id` byte
  selecting ChaCha20-Poly1305 or dorado's Threefish-256 construction — the latter a
  sibling path dependency, `../dorado/rust/crates/dorado-engine`), `keychain` (a random
  256-bit key from the OS keychain via `keyring` — the platform backend features are
  load-bearing; without them keyring silently compiles a non-persisting mock),
  `passphrase` (the alternative key source: a user-chosen KDF — Argon2id default,
  now at 64 MiB/t=3 for a local unlock, scrypt, or PBKDF2-SHA256 — over a user
  passphrase; the algorithm, salt, and params live in a plaintext, self-describing
  KDF sidecar that is authoritative for an existing archive; the launch boots
  *locked* until unlocked). Either key source then fans the master into
  per-purpose subkeys (`HistoryKeys`, `dorado_engine::kdf::derive_from_key_with`)
  under a family-matched PRF (`settings::HistoryFanout`: BLAKE3 for ChaCha,
  Skein-512 for Threefish, `Auto` by default, user-overridable, fixed at enable),
  so the master never encrypts anything directly. `segment`
  (one encrypted file per local calendar day, opaque random filename, atomic
  temp-file+rename writes), `manifest` (the encrypted date→segment index),
  `writer` (a single background thread, the sole writer — panes funnel
  `HistoryEvent`s to it over an `mpsc` from `drain_effects`, so concurrent panes need
  no file locking), and `reauth` (macOS LocalAuthentication gating: opening the
  Scrollback History panel requires Touch ID / the device password once per session,
  plus an optional idle interval; fail-closed; a no-op off macOS). Every start is
  **async** (thread + oneshot into an `iced::Task` — the keychain read can block on an
  OS dialog and must never run on the UI thread; an in-app explainer precedes the first
  keychain access), landing in `apply_history_started`, which raises a command-id floor
  on every screen (`reserve_command_ids` — deferred starts must not mint ids that
  collide with today's archived entries) and seeds the newest entries into the first
  tab only if its log is still empty; the panel's "Load older day" pages back through
  the archive, and archived rows carry stable `(date, id)` targets so Clear/Delete on
  them go straight to the writer. **Untracked** tabs (⌘⇧T; ○-marked, badged in the
  panel, chip in the status bar) and untracked *sessions* (the `history_session_start`
  setting's record/ask/untracked, or `tty --untracked`) suppress at the source: the
  screen queues no events at all, and an untracked session does zero crypto and stays
  untracked until relaunch. The settings History section drills into a
  full-height archive browser behind the same gate (own paging cursor;
  right-click a row to Copy or Delete… it — Delete confirms via `rime::dialog`
  and tombstones through the writer; double-click copies; entries dropped when
  the browser or settings close). Failure policy throughout: refuse to load, warn,
  never crash — and a toggle-on failure reverts the setting rather than persisting
  "on but broken". Deleting the archive is a separate, dialog-confirmed **Reset**
  action; the off-toggle never deletes anything.

## Windows (detachable tabs)

tty runs on iced's **`daemon`** model (not `application`) so it can open extra OS windows
for **detached tabs** (ADR 0003): a daemon's `view`/`title`/`theme` take a `window::Id`,
letting each window render different content. `boot` opens the main window and records
`Tty::main_window`. A whole `Tab` (its owned `pane_grid::State<Term>`) is the detachable
unit — detaching moves it out of `tabs` into `detached: HashMap<window::Id, Tab>` and back
on reattach; `reap_dead`/`drain_effects` walk both. A daemon keeps running after its last
window closes, so closing the **main** window calls `iced::exit()` (tearing down every
detached window + shell). Detached terminals are **ephemeral** — no session is persisted.
- **`theme` / `settings`** — a `Theme { palette, terminal }` pairing a rime chrome
  `Palette` with a `phosphor::TerminalStyle`. Named themes come from rime's shared
  `builtin_themes()` catalog (8, the same list fed-ide shows), each with a base16 ANSI
  palette; a base16 import / panel edit becomes a "Custom" palette (chrome derived from
  the terminal colors). The **Appearance** section is itself split into horizontal
  sub-tabs (`APPEARANCE_TABS`: Theme / Tabs / Status bar / Terminal / Window,
  tracked by `Tty::appearance_tab`) so it shows one pane at a time instead of one
  long scroll; `settings_subtabs` renders the chip strip. Those panes carry the
  **Highlight active tab** and **Highlight the focused pane** toggles (the latter
  gates the accent border in the pane-grid closures; the rime `tabs` strip takes a
  `TabBarStyle { highlight_active, text_size }`, so accent-ink vs. subtler emphasis
  is host-tunable); the status-bar chrome — **Disable status bar** (drops it
  entirely; wins over auto-hide) and **auto-hide status bar** (on by default; when
  on, `main_view` drops the bar from the column and floats it back over the bottom
  edge via a `stack` only while `status_bar_revealed()` — the pointer within
  `STATUS_BAR_REVEAL_ZONE` of the bottom — so toggling it never reflows the pane
  grid); and the **Window** controls — **Keep window on top** (drives the iced `window::Level`,
  broadcast to every live window via `window::set_level`) and the two transparency
  amounts. A separate top-level **Metrics** section (above History,
  `metrics_section`) owns everything about the machine-stat cells: the cell editor
  (add / reorder / style / remove, per-cell warn/alarm thresholds, clock-format
  toggles), the pin-popovers toggle, the reorder-hold stepper, and the
  **graduate-into-a-pane** toggle (`graduate_metrics`, which gates the popover ⊞
  control). A read-only **Keys** section documents the shortcuts.
  `tty.settings.json` persists the theme name, font family/size, any custom palette,
  the active-tab highlight flag, `highlight_focused_pane`, the status-bar flags
  (`status_bar_autohide`, `status_bar_disabled`, `status_bar_metrics_pinned`,
  `graduate_metrics`, `status_bar_edit_hold_secs`),
  the clock format (`clock_24h` / `clock_seconds` / `clock_date`) and the ordered
  `status_bar_metrics` (each `{ metric, style, warn?, alarm? }`), the window flags
  (`window_always_on_top`), the `unfocused_opacity` / `focused_opacity` amounts, and
  the encrypted-history fields
  (`encrypted_history_enabled`, `history_key_source`, `history_kdf`,
  `history_fanout`, `history_cipher`, `history_reauth_interval_minutes`,
  `history_session_start`). `window_opacity()` drives a uniform per-surface fade —
  the `focused_opacity` while focused (floored at `MIN_FOCUSED_OPACITY`, so active
  transparency tops out at 50% and stays readable) and the `unfocused_opacity`
  otherwise (floored at `MIN_OPACITY`, 95%) — since iced 0.14 has no runtime
  window-opacity API, and never fades to an invisible, unrecoverable window.
- **`app_icon`** — the neon "tty." icon glue (shared shape with fed/rift): decodes the
  embedded `assets/icon-512.png` into an `iced::window::Icon` (Linux/Windows) and sets the
  macOS **Dock** icon at runtime via AppKit (`objc2`) from the first `root_view` render
  (post-launch, main thread, `Once`-guarded — a bare `cargo run` binary isn't a bundle).
  The packaged `.app` gets `AppIcon.icns` from salpa.

## Shared with fed-ide

`cathode` + `phosphor` are path-depended by **fed-ide** for its terminal panel. The
edge is one-directional (fed → tty); see `docs/adr/0001-crate-split.md`.

## Testing

nextest is the runner (`.config/nextest.toml`). `cathode` carries engine unit tests;
the `tty` app carries `behavior::*` (drive state/update with pty-less tabs — no shell,
no GPU) and `snapshot::*` (render the chrome to a PNG; backend-specific baseline,
excluded from the default run). Run snapshots with
`cargo nextest run --ignore-default-filter -E 'test(snapshot)'`.

**CI runs all three tiers headlessly**, including snapshots. `iced_test::Simulator`
tries `wgpu` first, which needs a display — the unit/behavior tier gets one from
`xvfb-run`. Snapshot **pixel comparison** is a different problem: `wgpu`'s output is
GPU/driver-specific, so a baseline recorded on one machine won't byte-match another's —
which is why `snapshots/*-wgpu.png` (recorded on macOS/Metal, for local dev iteration)
were historically excluded from CI. `iced_test` also compiles in a `tiny-skia` software
rasterizer (an iced default feature, no extra Cargo work needed) with zero GPU/display
dependency, selectable via `ICED_TEST_BACKEND=tiny-skia` — forcing it makes CI's
`snapshots/*-tiny-skia.png` baselines fully portable: verified byte-for-byte identical
across independent fresh Linux containers (matching the `ubuntu-latest` CI runner), no
`xvfb` needed for that step. `.github/workflows/ci.yml`'s `rust` job runs it as a
separate step from the `xvfb`-wrapped unit/behavior run.

**Coverage** (`cargo-llvm-cov`, workspace-scoped: `cathode`/`phosphor`/`tty`, not rime
or upstream deps) runs in CI as its own `coverage` job — same `tiny-skia` forcing, so
the instrumented run also exercises the snapshot tier — gated at 60% lines (`--fail-
under-lines`) with an HTML report uploaded as a build artifact for drilling into misses.
Known low-coverage areas, inherent to what they are rather than undertested:
`cathode::pty`/`wake` (real PTY/OS signal plumbing), `tty::main`/`subscription`/
`app_icon` (entry point, iced subscriptions, platform Dock-icon AppKit calls) — all
thin, hard-to-unit-test glue not worth chasing. `phosphor::terminal`'s pure helpers
(`hit`, `cell_pos`, `resolve`, `order`, `selected_text`, …) and `Terminal`'s own
methods (`cell_colors`, `dims`, `selection_text`, …) are unit-tested directly; what's
left uncovered there is almost entirely the `Widget` trait's `draw`/`layout`/`update`
methods, which need a full render/event harness to exercise meaningfully beyond what
the snapshot and behavior tests already do incidentally. See
[ADR 0005](adr/0005-headless-ci-snapshots-and-coverage.md).

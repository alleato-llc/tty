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
  underline/dim/inverse), the cursor, and `scrollback`. `resize(cols, rows)` reflows.
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

## tty — the app

Thin glue, mirroring `fed`'s module shape:

- **`state`** — `Tty { tabs: Vec<Tab>, active, theme, font, font_size, … }`. A `Tab` is
  a `pane_grid::State<Term>` split tree plus its focused `Pane` (a single pane until the
  user splits). A `Term` is a `screen` + an `Option<PtySession>` (`None` only in tests)
  + an `alive` flag the read loop clears on shell exit. Methods target the active tab's
  focused pane: `split_focused`/`focus_dir`/`close_focused_pane`, `write_focused`/
  `write_pane`, `resize_pane`, `new_tab`/`close_tab`, `zoom`/`reset_zoom`. `reap_dead`
  drops dead panes, then any tab with no live pane, then exits when none remain.
- **`update`** — app **chords use ⌘** (`Modifiers::command()`) so `Ctrl` stays a real
  terminal control code: `⌘T`/`⌘N`/`⌘W`, `⌘1`–`⌘9`, `⌘±`/`⌘0`, `⌘C` copy, `⌥⌘`+arrows
  split / `⌃⌘`+arrows move focus. Everything else becomes PTY input via `phosphor::input`.
- **`view`** — window-aware: `root_view(state, window)` routes a **detached** window to a
  lean `detached_view` (the tab's `pane_grid` + a Reattach button + a status bar) and every
  other window to `main_view` — the `rime` `tabs` strip (shown only with >1 tab, matching
  fed / fed-ide), a `pane_grid` over the active tab's panes (each pane a `phosphor`
  terminal; the focused one's border turns accent only when the tab has >1 pane), and a
  `rime` `status_bar`. A right-click (or `⌃`-click, macOS's secondary-click) opens a `rime`
  `context_menu` at the tracked pointer — a pane menu (split + close pane) or a tab menu
  (new tab + rename + **detach** + split + close tab), per `state.menu`. "Rename tab" shows
  a focused field under the strip; `Tab::label()` resolves a tab's display name (custom →
  program title → shell). Pane messages carry the originating `window::Id` so a click /
  resize / selection routes to the right tab.
- **`subscription`** — key events + per-window geometry (`Focused`/`Resized`/`Moved` via
  `listen_with`'s window id) + `window::close_events` + **one always-on output stream** fed
  by `cathode::wake` (drains an output burst into a single redraw; also reaps dead tabs).
  While a detached window is settling after a drag, a short-lived timer polls the drag-dock
  debounce.

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
  the terminal colors). The settings panel also carries a **Highlight active tab**
  toggle (the rime `tabs` strip takes a `TabBarStyle { highlight_active, text_size }`,
  so accent-ink vs. subtler emphasis is host-tunable) and a read-only **Keys** section
  documenting the shortcuts. `tty.settings.json` persists the theme name, font
  family/size, any custom palette, the active-tab highlight flag, and the
  "Transparency On Blur" amount. `window_opacity()` drives
  a uniform per-surface fade when the window loses focus (no runtime window-opacity API
  in iced 0.14), clamped to `settings::MIN_OPACITY` so it tops out at 95% and never
  fades to an invisible, unrecoverable window.
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

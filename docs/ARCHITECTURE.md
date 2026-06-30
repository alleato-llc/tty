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

## tty — the app

Thin glue, mirroring `fed`'s module shape:

- **`state`** — `Tty { tabs: Vec<Term>, active, theme, font, font_size, … }`. A
  `Term` is a `screen` + an `Option<PtySession>` (`None` only in tests) + an `alive`
  flag the read loop clears on shell exit. Methods: `new_tab`/`close_tab` (last close
  → quit), `write_active`, `resize_active`, `zoom`/`reset_zoom`, `reap_dead`.
- **`update`** — app **chords use ⌘** (`Modifiers::command()`) so `Ctrl` stays a real
  terminal control code: `⌘T`/`⌘N`/`⌘W`, `⌘1`–`⌘9`, `⌘±`/`⌘0`, `⌘C` copy. Everything
  else becomes PTY input via `phosphor::input`.
- **`view`** — the `rime` `tabs` strip (only when >1 tab), the `phosphor` terminal,
  and a `rime` `status_bar`.
- **`subscription`** — key events + **one always-on output stream** fed by
  `cathode::wake` (drains an output burst into a single redraw; also reaps dead tabs).
- **`theme` / `settings`** — a `Theme { palette, terminal }` pairing a rime chrome
  `Palette` with a `phosphor::TerminalStyle`. Named themes come from rime's shared
  `builtin_themes()` catalog (8, the same list fed-ide shows), each with a base16 ANSI
  palette; a base16 import / panel edit becomes a "Custom" palette (chrome derived from
  the terminal colors). `tty.settings.json` persists the theme name, font family/size,
  any custom palette, and the "Transparency On Blur" amount. `window_opacity()` drives
  a uniform per-surface fade when the window loses focus (no runtime window-opacity API
  in iced 0.14), clamped to `settings::MIN_OPACITY` so it tops out at 95% and never
  fades to an invisible, unrecoverable window.

## Shared with fed-ide

`cathode` + `phosphor` are path-depended by **fed-ide** for its terminal panel. The
edge is one-directional (fed → tty); see `docs/adr/0001-crate-split.md`.

## Testing

nextest is the runner (`.config/nextest.toml`). `cathode` carries engine unit tests;
the `tty` app carries `behavior::*` (drive state/update with pty-less tabs — no shell,
no GPU) and `snapshot::*` (render the chrome to a PNG; backend-specific baseline,
excluded from the default run). Run snapshots with
`cargo nextest run --ignore-default-filter -E 'test(snapshot)'`.

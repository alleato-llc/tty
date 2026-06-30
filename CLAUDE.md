# CLAUDE.md

Guidance for working in this repo. Read this before adding UI code.

## What this is

**tty** is a GPU-accelerated terminal built on Rust + [iced](https://iced.rs) 0.14.
It's the terminal counterpart of [`fed`](https://github.com/nycjv321/fed) and shares
its foundation (iced + the `rime` component kit). Three crates:

| Crate      | Role                                                                 |
|------------|----------------------------------------------------------------------|
| `cathode`  | Terminal **engine** — VT/ANSI `parser`, the `screen` grid model, a `pty` session, and a `wake` signal for output-driven repaint. Pure Rust, **no iced**. |
| `phosphor` | Terminal **widget** — a stateful iced widget rendering a `cathode` screen (color + attributes, scrollback, mouse select/copy), plus `input` (iced key → PTY bytes). Self-styles from a plain `TerminalStyle` (no theme-crate dependency). |
| `tty`      | The **app** — a minimalist tabbed terminal: thin glue over `cathode` + `phosphor` + `rime`. |

Names follow the retro CRT path: a **cathode** ray lights up the **phosphor** screen.

## The terminal is shared with fed-ide

`cathode` + `phosphor` are consumed by **fed-ide**'s integrated terminal panel via a
path dependency (`../tty/cathode`, `../tty/phosphor`), the same way `fed` consumes
`rime`. The dependency is **one-directional, fed → tty**: nothing here depends on
`fed` or its crates. So:

- **Keep `phosphor` theme-crate-free.** The host supplies a `TerminalStyle` (plain
  colors); fed-ide maps its `patina` theme into one, tty builds one from `rime` +
  `phosphor::ANSI_DEFAULT`. Don't reach for `patina` here — it lives in the fed repo.
- **Terminal behavior is shared.** A refinement to the engine or widget belongs in
  `cathode`/`phosphor`, so both the standalone app and the IDE panel get it.

## rime — the component kit (READ THIS)

`rime` is **our** reusable iced component kit, living *outside* this repo at
`../rime/rime` (a path dependency), shared with `fed`. It is the single source of
truth for the visual vocabulary: buttons, inputs, the `tabs` strip, `status_bar`,
etc.

> **Extend rime. Never fork, duplicate, or inline a UI component.**

rime components are **stateless** and self-style from a thread-local palette scope.
The host enters it once at the top of `view()`:

```rust
let _scope = theme::enter(self.theme.palette); // RAII; drops at end of view
```

Components call `theme::tokens()` at view-time. tty's `theme.rs` builds on rime's
shared catalog: `rime::theme::builtin_themes()` gives the 8 named chrome palettes
(Dracula, Nord, Gruvbox Dark, Solarized Dark/Light, GitHub Light, Neon Nights,
Phosphor) — the same list fed-ide shows — and tty pairs each with a `TerminalStyle`
(its base16 ANSI palette). A base16 import / panel edit becomes a "Custom" palette that
re-themes both the grid and the chrome. **Keep the catalog in rime**, not forked here,
so the two apps never drift. If you need a new reusable primitive, **add it to rime**,
then depend on it here — don't hand-roll a styled widget.

The **whole window fades when unfocused** (an optional transparency setting): `view()`
runs the palette and `TerminalStyle` through `fade_palette`/`fade_style` by
`state.window_opacity()`, and `state::theme()` fades the iced runtime theme to match —
iced 0.14 has no runtime window-opacity action, so the fade is uniform per-surface.

## iced

iced is pinned at **0.14**. Before writing iced code (the `phosphor` custom widget,
app wiring, subscriptions), read **`../rime/ICED.md`** — the patterns & gotchas plus
the 0.13→0.14 diff. The terminal repaints **on output**: the `cathode` read thread
calls `cathode::wake::signal()`, and each app runs one always-on subscription that
awaits it (no idle polling). Don't reintroduce a periodic tick.

## Build & test

[cargo-nextest](https://nexte.st) is the test runner; the snapshot tests are
categorized in `.config/nextest.toml` (a `serial-ui` group + a snapshot
`default-filter`, mirroring fed).

```sh
cargo build --bins
cargo nextest run                          # unit + behavior (the everyday command)
cargo nextest run --ignore-default-filter  # whole suite, incl. the snapshot
cargo clippy --all-targets -- -D warnings
```

`behavior::*` tests drive `tty::state`/`update` with pty-less tabs (no shell);
`snapshot::*` renders the chrome to a PNG (backend-specific baseline, excluded from
the default run). Full story in `docs/ARCHITECTURE.md`.

# tty

A fast, focused, GPU-accelerated **terminal** written in Rust — tabs, scrollback,
mouse select & copy, full ANSI color, and output-driven repaint (it redraws when
the shell writes and sleeps when it doesn't). The terminal counterpart of
[`fed`](https://github.com/nycjv321/fed), built on the same `iced` + `rime`
foundation.

> The name nods to the old `tty` — but it ships as **`Tty.app`** so the bare `tty`
> binary never shadows the POSIX `tty(1)` command.

## Crates

This is a Cargo workspace of three crates — engine, widget, app — named for the
retro CRT path *cathode ray → phosphor glow*:

| Crate      | Role                                                                 |
|------------|----------------------------------------------------------------------|
| `cathode`  | The terminal **engine**: a VT/ANSI parser, the screen-grid model, a PTY session, and a wake signal. Pure Rust, no iced — embeddable by any front-end. |
| `phosphor` | The terminal **widget**: a stateful `iced` widget that renders a `cathode` screen (color + attributes, scrollback, mouse select/copy), plus iced-key → PTY-byte translation. |
| `tty`      | The **app**: a minimalist tabbed terminal — thin glue over `cathode` + `phosphor` + `rime` chrome. |

`cathode` + `phosphor` are also consumed by **fed-ide**'s integrated terminal
panel (by path dependency, the way `fed` consumes `rime`), so refinements to the
terminal land in both the standalone app and the IDE at once. See
`docs/ARCHITECTURE.md` and `docs/adr/0001-crate-split.md`.

## rime

The chrome (tab strip, status bar) comes from **rime**, our shared `iced`
component kit — a sibling path dependency at `../rime/rime`
([`alleato-llc/rime`](https://github.com/alleato-llc/rime)). Lay it down next to
this repo so the path dependency resolves. **Extend rime; never fork a UI
component.**

## Build & run

[cargo-nextest](https://nexte.st) is the test runner (the snapshot tests are
categorized in `.config/nextest.toml`):

```sh
cargo run -p tty                           # launch the terminal
cargo build --bins
cargo nextest run                          # unit + behavior
cargo nextest run --ignore-default-filter  # whole suite, incl. the snapshot
cargo clippy --all-targets -- -D warnings
```

## Keys

- `⌘T` / `⌘N` new tab · `⌘W` close tab (or the last → quit) · `⌘1`–`⌘9` jump to tab
- `⌘+` / `⌘−` font zoom · `⌘0` reset
- `⌘C` copy the selection (drag to select); `Ctrl+C` stays a real SIGINT to the shell
- wheel to scroll back through history

## Landing page

`web/` is a static [Astro](https://astro.build/) site deployed to
`tty.alleato.dev` via `salpa` (see `salpa.yaml` and
`.github/workflows/deploy-site.yml`).

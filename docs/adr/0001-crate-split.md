# 0001 — Three crates: cathode (engine), phosphor (widget), tty (app)

Status: accepted

## Context

tty began life inside the `fed` monorepo as three pieces: a `geyser` terminal engine,
a `fjord::terminal` iced widget (+ `fjord::pty` key translation), and a minimal `tty`
binary. We pulled it out into its own product repo (`alleato-llc/tty`) with its own
landing page and release pipeline, while keeping **fed-ide's integrated terminal panel
working** — fed-ide should consume the terminal as an external dependency, the same way
`fed` already consumes the shared `rime` component kit (sibling `../rime/rime`, path
dependency).

Two consumers therefore need the terminal: the standalone `tty` app *and* `fed-ide`.
Both need the engine **and** the widget.

## Decision

Split into **three crates** along a clean seam, named for the retro CRT path
*cathode ray → phosphor glow*:

- **`cathode`** — the engine (ex-`geyser`): VT/ANSI parser, screen grid, PTY session,
  wake signal. **No iced** — pure terminal emulation, embeddable anywhere.
- **`phosphor`** — the iced widget (ex-`fjord::terminal` + `fjord::pty`→`input`).
  Depends on `cathode` + iced. Self-styles from a plain `TerminalStyle`, so it carries
  **no theme-crate dependency**.
- **`tty`** — the app: glue over `cathode` + `phosphor` + `rime`.

fed-ide depends on `cathode` + `phosphor` by path (`../tty/...`).

### Why not two crates?

A two-crate split (one bundled engine+widget lib + the app) was considered. Three keeps
the engine **iced-free and reusable** — a real value for an emulation core — at the cost
of one extra crate. fed-ide pulls both libs; that's fine.

## Consequences

- **No dependency cycle.** `phosphor` takes plain colors and never imports `patina`, so
  only `fed-ide` reaches into this repo. The edge is one-directional: **fed → tty**, and
  both → `rime`. A clean DAG. fed's `patina` keeps its terminal palette and maps it into
  a `phosphor::TerminalStyle` at view-time, exactly as it did with the old `fjord` widget.
- **tty owns its theming.** Rather than depend on fed's `patina`, the app uses rime's
  built-in `ThemeChoice` (Dracula/GitHub) + `phosphor`'s default ANSI palette, with its
  own `tty.settings.json`. tty is fully standalone.
- **Refinements land in both surfaces.** Engine/widget changes ship to the standalone
  app and the IDE panel at once.
- **CI checks out siblings.** Building `fed-ide` now requires `../tty` checked out next
  to fed (like `../rime/rime`); fed's CI/release add a `tty` checkout step. tty's own CI
  needs only the `rime` sibling.

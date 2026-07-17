# 0002 — Split panes via iced's `pane_grid`

Status: accepted

## Context

tty shipped as one terminal per tab (`Tty { tabs: Vec<Term>, active }`). Users wanted to
**split a tab into multiple panes** laid out left/right/up/down, each its own shell — the
"Option B: splits only" path the roadmap flagged as the biggest fork in tty's identity
(it pulls against **§ simplicity**). The constraint was explicit: splits **without** a
config language and **without** a session server. Persistent/detachable sessions
("Option C") were ruled out as a different product.

The open question was *how* to model the split tree, divider dragging, focus, and
cardinal navigation without hand-rolling a layout engine.

## Decision

Build splits on **iced 0.14's built-in `iced::widget::pane_grid`**, and restructure each
tab into a pane tree:

```rust
pub struct Tab { panes: pane_grid::State<Term>, focus: pane_grid::Pane }
pub struct Tty { tabs: Vec<Tab>, active: usize, /* … */ }
```

`pane_grid::State` already owns everything we'd otherwise write: the split tree
(`split(axis, …)`), drag-to-resize dividers (`resize`), focus, and cardinal
`adjacent(pane, Direction)` navigation. A tab is a single pane until the user splits, so
the common case is unchanged.

- **Input targets the active tab's focused pane.** `write_focused`/`resize_pane`/`paste`
  and the ⌘C selection read `tabs[active].panes.get(focus)`; `drain_effects`/`reap_dead`
  walk every pane of every tab. `reap_dead` closes dead panes, then drops any tab with no
  live pane, then exits when none remain.
- **Directional keybindings.** `⌥⌘`+arrow splits the focused pane toward that direction
  (`Left|Right → Axis::Vertical`, `Up|Down → Axis::Horizontal`; for `Left|Up` we
  `swap` so the new shell lands on the requested side). `⌃⌘`+arrow moves focus via
  `adjacent`. `⌘W` closes the focused pane (last pane → tab → quit). These are keyboard
  chords handled directly in `update::handle_key`, like the other ⌘ shortcuts; only the
  widget-driven `FocusPane` (click) and `ResizeSplit` (drag) flow through `Message`.
- **Spawn seam for tests.** `split_focused` spawns the shell; its spawn-free core
  `split_with(dir, term)` lets the headless behavior tests inject a pty-less pane.
- **Focus affordance.** When a tab has more than one pane, the focused pane's container
  border turns the accent color (others use the hairline) and fades with the window when
  unfocused. A lone pane shows no border (no stray accent rectangle).
- **Right-click menu.** A `mouse_area` per pane (and the tab strip's right-press hook)
  opens a rime `context_menu` anchored at the tracked pointer — the same actions as the
  chords, for discoverability. `state.menu` records which kind is open: a **pane** menu
  (Split ×4 + Close pane) or a **tab** menu (New tab + Split ×4 + Close tab). The **tab
  strip is always shown** (even with a single tab) so a tab is always right-clickable.
  Phosphor only captures the left button (selection) when mouse-reporting is off, so the
  right-click reaches the `mouse_area`; when an app enables mouse-reporting it goes there.
  macOS delivers a **Ctrl+click** as `Left+Control` rather than `Button::Right`, so
  `update` also opens the menu when `ActivateTab` / `FocusPane` arrive with Control held
  — the menu is reachable without a two-button mouse. The **tab menu** also offers
  **Rename tab** (a `Tab.title` override edited in a focused field under the strip).

## Consequences

- No new layout code, no session server, no config DSL — the simplicity constraint
  holds. Nothing was added to `rime`: `pane_grid` is an iced built-in, not reusable
  chrome, so it stays in the app.
- `Message::Resize/Select/PtyBytes` now carry the originating `pane_grid::Pane`, since a
  pane's widget reports its own size/selection/mouse bytes.
- Snapshot coverage gained a `split_pane_view` baseline alongside the single-pane one.
- **fed-ide is unaffected** — splits live entirely in the `tty` app; `cathode` and
  `phosphor` are untouched, so the one-directional fed → tty dependency (ADR 0001) still
  holds.
- Persistence (Option C) remains out of scope. The complementary bet — be excellent
  *under* tmux — still stands.

## Update — panes carry a content enum

The pane content was later generalized from `Term` to a `Pane` enum
(`pane_grid::State<Pane>`, `Pane = Term(Term) | Metric(MetricKind)`) so a status-bar
metric drill-in can be "graduated" into a real pane. This preserved every decision
above: the split/focus/resize/close/maximize machinery is generic over the pane
content, and terminal operations filter through `Pane::as_term` (`Tab::terms()`), so
a `Metric` pane is transparent to them (no PTY, never reaps). New non-terminal pane
kinds now slot into the enum without touching the layout code. See
`docs/ideas/status-bar-metrics.md` and `ARCHITECTURE.md` for the metric-pane
specifics (promote/replace flows, maximize, the focus-highlight toggle).

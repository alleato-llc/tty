# 0003 — Detachable tabs (tear a tab into its own window)

Status: accepted

## Context

tty shows a stack of tabs, each a `pane_grid::State<Term>` split tree (ADR 0002). Users
wanted to **tear a tab off into its own OS window** and dock it back — the terminal
counterpart of fed-ide's detachable *editor* tabs (fed ADR 0016). The win is the same:
put one terminal on a second monitor, or beside another app, while the rest stay in the
main strip.

ADR 0002 ruled out "persistent/detachable sessions (Option C)" as a different product.
This ADR refines that line: what 0002 excluded is a **session server** — surviving
process restart, a config language, reconnecting to a daemon. **Ephemeral** detach (a
live tab moved between windows of the *same running process*) is a UI affordance, not a
session model, and is in scope. A detached shell still dies when tty quits.

The hard constraint: iced's `application` model gives every window the *same* `view`, so
a second window can't render different content. Multi-window needs the `daemon` model.

## Decision

Convert tty from `iced::application` to **`iced::daemon`**, whose `view`/`title`/`theme`
take a `window::Id`, and route per window:

```rust
pub fn root_view(state: &Tty, window: window::Id) -> Element<_> {
    match state.detached.get(&window) {
        Some(tab) => detached_view(state, window, tab), // lean: panes + Reattach + status
        None      => main_view(state),                  // the full tabbed chrome
    }
}
```

A daemon opens no window itself, so `boot` opens the main window, records
`Tty::main_window`, and returns the open task.

- **The detachable unit is a whole `Tab`** (its owned pane tree). Detaching **moves** the
  `Tab` out of `tabs` into `detached: HashMap<window::Id, Tab>`; reattaching moves it back
  at its `detach_origin` index. Because a `Term` is self-contained (its PTY read loop runs
  on its own thread and repaints via `cathode::wake::signal()`), moving a tab between
  windows needs no engine coordination — simpler than fed's shared-`DocumentId` case. If
  detaching would empty the main strip, a fresh shell tab is spawned so the main window is
  never empty.
- **Pane messages are window-tagged.** `pane_grid::Pane` ids are only unique within one
  `pane_grid::State`, so `Resize`/`Select`/`PtyBytes`/`FocusPane`/`ResizeSplit` carry a
  `window::Id`; `update` resolves the target tab via `tab_for(_mut)(window)` (main window
  → `tabs[active]`, else `detached[window]`). The keyboard routes to the **focused
  window** (`focused_window`, tracked via per-window `Focused` events).
- **Three reattach paths** (full parity with fed): a **Reattach** button in the detached
  window, **closing** the window (an OS-close docks the tab back), and **drag-to-dock**
  (drag the window onto the main window's tab-bar band — `detach_drag.rs`, a position-
  overlap heuristic since iced has no cross-window drop event). Two detach paths: a
  **Detach Tab** context-menu item and a **tear-off drag** (drag a tab down out of the
  strip past `TAB_TEAR_THRESHOLD`).
- **⌘W in a detached window** closes the focused pane; the last pane removes the tab from
  `detached` *before* closing the window, so the ensuing `WindowClosed` finds nothing to
  reattach (⌘W-through-the-last-pane kills the window; an OS-close reattaches).
- **No session persistence.** Unlike a fed document (restored from its disk path), a
  detached shell is ephemeral. Closing the **main** window calls `iced::exit()`, tearing
  down every window and its shell (a daemon keeps running after its last window closes, so
  exit is explicit). There is no `detached.json` analog.

## Consequences

- Every pane message now carries a `window::Id`; `drain_effects`/`reap_dead` walk both the
  main strip and the detached windows, and `reap_dead` returns the windows to close when a
  detached tab's shell dies.
- Settings, find, rename, and the context menu render only in the main window, so detached
  windows stay lean (terminal + Reattach + status bar); splitting still works there via the
  ⌥⌘-arrow chords. Per-window theme/opacity fade is global in v1 (one `state.focused`).
- `detach_drag.rs` (and the `window_bounds`/`last_detached_move` fields it drives) is the
  only experimental piece and is deletable as a unit — the button + reattach-on-close paths
  stand on their own.
- The `transparent`/window-size that were `application` builder calls move into
  `window::Settings`.

## Follow-up — in-strip reorder (2026-06-30)

Dragging a tab *sideways* reorders it, reusing this ADR's drag machinery rather than
adding new state. The arm is the same `tab_drag = Some((idx, pointer))` set when a tab is
pressed; as the pointer crosses another tab the strip's `on_hover(Some(target))` fires and
`reorder_dragged_tab(target)` moves the dragged tab to that slot live (browser-style),
re-anchoring the drag to the new index. The vertical `TAB_TEAR_THRESHOLD` still decides
tear-off at release, so a sideways drag reorders and a downward drag detaches.

This depends on the tab activating on **press** (mouse-down), which is a `rime::tabs`
change: the tab body is a plain container and the wrapping `mouse_area.on_press` reports
the press — an iced `button` only reports on release, by which point the gesture is over.
The same press-arm powers tear-off; both silently no-op'd while activation was release-based.

## Update — pane-tabs detach into their own window too

A single terminal tab inside a pane (see ADR 0002's update) can also detach, reusing this
machinery: `detach_pane_tab` lifts the terminal into a one-pane `Tab` via the shared
`open_detached_window`, recording a `PaneTabOrigin`. On reattach, `restore_pane_tab` drops
it back into its origin tab group when that group still exists; otherwise it falls back to
the top-level dock described here (a new main-strip tab). Detaching a pane-tab is a
deliberate menu action only — never a drag-off — so a mis-aimed cross-pane move can't tear
the tab out of the window.

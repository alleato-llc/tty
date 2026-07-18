---
name: ui
description: Use when building or changing a UI surface in tty or rime — a popover/drill-in, a status-bar cell, a settings toggle or section, editing an existing widget, or authoring a new reusable rime primitive. Covers the Message→update→view→state wiring loop, the tty/rime boundary, popover + modal chrome, the settings TOML round-trip, snapshot tests (and their re-baseline + clock gotchas), and the verification commands. Apply whenever you touch iced view/update/state code in tty (`tty/src/`) or a widget in rime (`../rime/rime`).
---

# Building UI in tty + rime

tty is an iced 0.14 app; `rime` (`../rime/rime`, a path dep) is the shared, stateless
component kit. **Reusable chrome lives in rime; app-specific surfaces live in tty.** This
skill is the procedure for both. Read `tty/CLAUDE.md` (invariants, the crate split) and
`../rime/ICED.md` (iced 0.14 patterns) alongside it; this covers the *workflow*, not the
constraints.

## Where things live (tty/src/)

The app is one Elm loop. A feature threads through the same files every time:

| File | Role |
|------|------|
| `message.rs` | The `Message` enum — every event/action variant. |
| `update.rs` | `update()` — the big `match Message` that mutates state. |
| `view.rs` + `view/*.rs` | Render `&Tty` → `Element`. One file per surface (`view/env.rs`, `view/popover.rs`, `view/metrics.rs`, `view/settings.rs`, `view/scrollback.rs`). |
| `state.rs` + `state/types.rs` | `Tty` (the whole app state struct) lives in `state/types.rs`; its methods + `Tty::new` in `state.rs` (and `state/{metrics,panes,scrollback,encrypted_history}.rs`). |
| `settings.rs` | `Settings` (persisted TOML) + resolvers + migrations. |
| `snapshot.rs` / `behavior.rs` | Tests. `snapshot::*` renders chrome to a PNG; `behavior::*` drives `update()` with pty-less tabs. |

## The wiring loop (memorize this)

Every surface — popover, cell, toggle — is the same five steps:

1. **State** — add fields to `struct Tty` in `state/types.rs` (doc-comment them).
2. **Message** — add the event/action variant(s) to `Message` in `message.rs`.
3. **Update** — handle them in the `update()` match in `update.rs` (mutate `state`, usually calling a method you add in `state.rs`).
4. **View** — render from `&Tty` in the relevant `view/*.rs`; wire buttons/fields to your `Message`s.
5. **Init** — a new `Tty` field must be initialized in **all three** full struct-literal sites, or it won't compile:
   - `state.rs` → `Tty::new`
   - `behavior.rs` (the test constructor)
   - `snapshot.rs` → the `populated()` helper

> ⚠️ **The 3-init trap.** Forgetting a site gives `error[E0063]: missing field ...`. Add the field to all three in the same pass. `grep -rn "<a_neighboring_field>:" tty/src/{state.rs,behavior.rs,snapshot.rs}` finds the sites fast.

## The tty ↔ rime boundary

- **Reusable visual primitive** (a button style, a card, a floating-panel chrome) → **add it to rime**, then depend on it. Never hand-roll or inline a styled widget in tty. See "Authoring a rime primitive" below.
- **App-specific surface** (the Env view, the metric drill-in, a settings section) → tty, *composing* rime primitives.
- rime primitives are **stateless** and read colors from `theme::tokens()`. tty owns the state; it passes open/selected/expanded flags in.

Shared chrome already in rime you'll reach for: `button::{primary,secondary,ghost,ghost_compact,danger,icon}`, `text_field`, `toggle`, `section`, `caption`, `stat`, `select`, `stepper`, `modal`/`modal_sized`, `dialog`, `context_menu`, `table`, and **`popover`/`resize_edges`/`ResizeEdge`** (the draggable-resizable floating card).

---

## Recipe: a popover / drill-in

A floating, draggable, resizable card over the terminal (the Env view, metric drill-ins).
Use rime's `popover` primitive — do **not** re-hand-roll drag/resize.

**State** (`state/types.rs`): position `Option<(f32,f32)>`, size `(f32,f32)`, a move-drag
and a resize-drag field, plus whatever content state (`env_expanded`, filters, …). Mirror
the `env_*` or `metric_detail_*` fields.

**View** (`view/<surface>.rs`):
```rust
pub(super) fn place_<x>_popover<'a>(state: &'a Tty, base: Element<'a, Message>) -> Element<'a, Message> {
    let (x, y) = state.<x>_effective_pos();      // env_pos or centered
    let (w, h) = state.<x>_view_size();           // see sizing below
    let floating = rime::widgets::popover(        // whole card = drag handle, opaque, resize edges
        card(state, w, h),
        Message::<X>MoveStart,
        Message::<X>ResizeStart,                  // fn(ResizeEdge) -> Message
    );
    let placed = container(floating)
        .width(Length::Fill).height(Length::Fill)
        .align_x(Left).align_y(Top)
        .padding(Padding::ZERO.left(x).top(y));
    stack![base, placed].into()
}
```
Then in `view.rs`, stack it in when open: `if state.show_<x> { base = <surface>::place_<x>_popover(state, base); }`.

**Update** (`update.rs`): `MoveStart` records `(pointer, effective_pos)`; `ResizeStart`
records `(pointer, size, edge)`; the drag itself is tracked in the `PointerMoved` handler
(clamp to the window) and cleared on `PointerReleased`. Copy the `env_*`/`metric_detail_*`
arms.

**Sizing** (`ResizeEdge::axes()` says which dims a grab changes): give it a `<x>_view_size()`
method. For a compact-vs-expanded card, shrink the *height* to content and cap it so a long
list scrolls instead of ballooning (see `Tty::env_view_size` + `ENV_COMPACT_MAX_HEIGHT`).
Wrap long content in `scrollable(list).height(Fill)` so the cap scrolls.

**Compact ↔ expanded**: add an `<x>_expanded: bool` + a `Toggle<X>Expanded` message that
flips it and snaps the size, and use rime's `+`/`−` glyph controls
(`button::ghost_compact(if expanded {"−"} else {"+"}, ...)`) — same cluster as the metric
drill-ins, for consistency.

**A sub-form** (add/edit): use a centered `rime::widgets::modal_sized(base, content, on_dismiss, width)`
opened by a bool in state, rather than an always-inline footer. See `place_env_add_modal`.

## Recipe: a status-bar cell

Cells are `MetricKind` variants in `settings.rs`. A **sampled** metric (CPU/mem) has a
sampler + sparkline; a **launcher** cell (like `MetricKind::Env`) just renders a static
label and opens a surface on click. To add one:

1. `settings.rs`: add the `MetricKind` variant; update `ALL` (bump the array length),
   `as_setting_str`, `from_setting_str`, and the `Display` impl (round-trips via `ALL`).
2. `view/metrics.rs`: in `metric_render`, either produce a series (sampled) or short-circuit
   with a static label like `Clock`/`Env` do (add the `K::<X> => unreachable!(...)` arm in
   the stats match if you short-circuit before it).
3. Click: in `update.rs`'s `PointerReleased`, route the cell key — `open_metric_detail(&key)`
   for a drill-in, or a custom action (`Env` calls `toggle_env_view()`).

## Recipe: a settings toggle / section

`Settings` (in `settings.rs`) is serde + `toml_edit` (comment-preserving round-trip).

- **Field**: add it to the relevant struct. Optional fields carry
  `#[serde(default, skip_serializing_if = "Option::is_none")]` so an unset value stays out
  of the file; a nested group uses `skip_serializing_if = "<Group>::is_empty"`.
- **Grouping + gating**: group related flags in a sub-struct (see `ShellIntegration`) with a
  **resolver** method (`settings.shell_integration() -> Resolved…`) that applies a master
  gate and defaults. Callers read the *resolved* value, never the raw `Option`.
- **Migration**: renaming/moving a key? Keep the old key as
  `#[serde(default, skip_serializing)]` (reads old files, never writes it) and fold it in a
  `migrate_*` step. A malformed value is an error, never a silent re-mint.
- **UI**: `view/settings.rs` dispatches on `state.settings_section`; add controls to the
  right section fn (e.g. the Shell section) using `toggle`/`select`/`stepper`/`text_field`,
  wired to `Set*` messages that call a setter which does `self.settings.<x> = …;
  self.settings.save();`.
- **Live preview**: settings changes should reflect immediately — resolve off
  `state.settings` at view time, don't cache.

> `Settings::save` is a **no-op under `cfg(test)`**, so behavior tests can drive real
> `update()` paths without rewriting the developer's settings file. Don't defeat that.

## Recipe: editing an existing widget

Find its `view/*.rs`, change the render, and **re-baseline every snapshot it appears in**
(see below). If you change layout math (sizes, columns), prefer branching in the view over
new state. Keep the compact/expanded and rime-primitive conventions above.

## Authoring a rime primitive (../rime/rime)

When tty needs reusable chrome, add it to rime — do not inline it. The contract is in
`../rime/rime/src/widgets/COMPONENTS.md` (read it). In short:

1. New file `src/widgets/<x>.rs`. One primitive, **stateless**, generic over the message
   type `M`, returns `Element<'a, M>`. Read colors from `theme::tokens()` — **never a
   hardcoded color**. Capture `tokens()` into draw-time `move` closures (see `card.rs`).
2. Register in `src/widgets/mod.rs`: `mod <x>;` **and** `pub use <x>::{...};`.
3. `mouse_area(..).on_press(msg)` requires `M: Clone` — bound your fn accordingly.
4. Add it to the demo (`demo/`) so the visual/theme-toggle check covers it, and to the
   README widget catalog + `CHANGELOG.md` (Unreleased).
5. Content taller than the window needs a `scrollable` — a bare `Shrink` column silently
   stops rendering past a height (see rime's ICED.md).

The `popover`/`resize_edges`/`ResizeEdge` primitive is the reference example: it holds no
position state (the caller owns it) and supplies drag messages — that's how a stateless
primitive backs a stateful surface.

---

## Snapshot tests

`snapshot::*` renders a `Tty` to a PNG and pixel-compares. Author one by copying an
existing test:

```rust
#[test]
fn my_surface_view() {
    let mut tty = populated();
    tty.show_x = true;
    // …set the exact state the snapshot should capture…
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim.snapshot(&crate::state::theme(&tty)).expect("render snapshot");
    let matches = snap.matches_image("snapshots/tty-my-surface.png").expect("write/compare");
    assert!(matches, "snapshot `tty-my-surface` changed — delete its PNG to re-baseline");
}
```

**Re-baselining** (after an intentional visual change):
- The real file has a **backend suffix**: `snapshots/tty-my-surface-**wgpu**.png` (not the
  bare name in `matches_image`). `ls tty/snapshots/ | grep <name>` to find it.
- Delete that PNG and re-run — a missing baseline is **written and passes**. Re-run once
  more to confirm it now compares clean.

**Gotchas**
- ⚠️ **Fixed clock.** A fixture seeded from `chrono::Utc::now()` (or `SystemTime::now()`)
  bakes wall-clock time into a pixel-exact image → it flakes, and **flips outright across
  midnight**. Seed from a *fixed* anchor and pin the view's clock (`Tty::clock_override`,
  threaded into `now_ms()` / `age_from_epoch_ms`). This already bit the history snapshots.
- New `Tty` field → update the `snapshot.rs` literal too (the 3-init trap).
- Snapshots are excluded from the default nextest run (`--ignore-default-filter` includes
  them).

## Verify (from inside `tty/`)

```sh
cargo fmt -p tty
cargo clippy -p tty --all-targets -- -D warnings     # must be clean
cargo nextest run -p tty --ignore-default-filter --no-fail-fast   # whole suite incl. snapshots
```
Target one test while iterating: `cargo nextest run -p tty --ignore-default-filter -E 'test(my_surface_view)'`.
For rime work: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
in `../rime`, then eyeball `cargo run -p rime-demo`.

**No inline test modules**: a module `foo.rs` keeps unit tests in a sibling `foo_tests.rs`,
wired `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;`.

## Checklist

- [ ] State fields added + doc-commented (`state/types.rs`)
- [ ] Initialized in **all three** literals (`state.rs`, `behavior.rs`, `snapshot.rs`)
- [ ] `Message` variant(s) + `update.rs` handler(s)
- [ ] Rendered in `view/*.rs`; reusable chrome came from rime, not inlined
- [ ] Settings (if any): field + `skip_serializing_if`, resolver/gate, migration, UI, `save()`
- [ ] Snapshot(s) added/updated; re-baselined the `-wgpu` PNGs; no `now()` in fixtures
- [ ] `fmt` clean, `clippy -D warnings` clean, full suite green
- [ ] Changelog/docs updated where the repo expects it (rime CHANGELOG; tty README/ARCHITECTURE for user-facing or architectural changes)

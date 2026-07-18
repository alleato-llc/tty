---
name: ui
description: Start here for any UI-surface work in tty or rime — building or changing a popover/drill-in, a status-bar cell, a settings toggle or section, an existing widget, or a reusable rime primitive. This file carries the shared wiring loop, the tty/rime boundary, snapshot-test mechanics, and verification; it routes to a focused per-component reference for the specifics. Apply whenever you touch iced view/update/state code in `tty/src/` or a widget in `../rime/rime`.
---

# Building UI in tty + rime

tty is one iced 0.14 Elm loop; `rime` (`../rime/rime`, a path dep) is the shared, stateless
component kit. **Reusable chrome lives in rime; app-specific surfaces live in tty.** The UI
is a set of *separate components* (a popover, a status-bar cell, a settings section, a pane)
that all thread through the **same wiring loop** below. Read the loop, then jump to the one
reference for the component you're building.

Read `tty/CLAUDE.md` (invariants) and `../rime/ICED.md` (iced 0.14 patterns) alongside this.

## Where things live (tty/src/)

| File | Role |
|------|------|
| `message.rs` | The `Message` enum — every event/action variant. |
| `update.rs` | `update()` — the big `match Message` that mutates state. |
| `view.rs` + `view/*.rs` | Render `&Tty` → `Element`. One file per surface (`view/env.rs`, `view/popover.rs`, `view/metrics.rs`, `view/settings.rs`, `view/scrollback.rs`). |
| `state.rs` + `state/types.rs` | `struct Tty` (all app state) in `state/types.rs`; its methods + `Tty::new` in `state.rs` (+ `state/{metrics,panes,scrollback,encrypted_history}.rs`). |
| `settings.rs` | `Settings` (persisted TOML) + resolvers + migrations. |
| `snapshot.rs` / `behavior.rs` | Tests: `snapshot::*` renders chrome to a PNG; `behavior::*` drives `update()` with pty-less tabs. |

## The wiring loop (shared by every component)

1. **State** — add fields to `struct Tty` in `state/types.rs` (doc-comment them).
2. **Message** — add the variant(s) to `Message` in `message.rs`.
3. **Update** — handle them in the `update()` match in `update.rs` (usually calling a method you add in `state.rs`).
4. **View** — render from `&Tty` in the relevant `view/*.rs`; wire controls to your `Message`s.
5. **Init** — a new `Tty` field must be initialized in **all three** full struct-literal sites:
   - `state.rs` → `Tty::new`
   - `behavior.rs` (the test constructor)
   - `snapshot.rs` → the `populated()` helper

> ⚠️ **The 3-init trap.** Miss a site and you get `error[E0063]: missing field ...`. Add the
> field to all three in one pass. `grep -rn "<neighbor_field>:" tty/src/{state.rs,behavior.rs,snapshot.rs}` finds them.

## The tty ↔ rime boundary

- **Reusable primitive** (a button style, a card, floating-panel chrome) → **extend rime**,
  then depend on it. Never inline or hand-roll a styled widget in tty.
- **App-specific surface** → tty, *composing* rime primitives.
- rime primitives are **stateless**, read colors from `theme::tokens()`; tty owns the state
  and passes flags in.

Chrome already in rime: `button::{primary,secondary,ghost,ghost_compact,danger,icon}`,
`text_field`, `toggle`, `section`, `caption`, `stat`, `select`, `stepper`,
`modal`/`modal_sized`, `dialog`, `context_menu`, `table`, and
**`popover`/`resize_edges`/`ResizeEdge`** (the draggable-resizable floating card).

## Pick the component → read its reference

| Building… | Read |
|-----------|------|
| a popover / floating panel (drag + resize, compact↔expanded, a sub-form modal) | [`reference/popover.md`](reference/popover.md) |
| a status-bar cell (sampled metric, or a launcher like `Env`) | [`reference/status-bar-cell.md`](reference/status-bar-cell.md) |
| a settings toggle / section (the TOML round-trip, gating, migration) | [`reference/settings.md`](reference/settings.md) |
| a reusable rime primitive (the contract, demo, tokens) | [`reference/rime-primitive.md`](reference/rime-primitive.md) |
| editing an existing surface | find its `view/*.rs`, change the render, re-baseline its snapshots (below). Branch in the view over adding state when you can. |

## Snapshot tests (shared)

`snapshot::*` renders a `Tty` to a PNG and pixel-compares. Author one by copying an existing test:

```rust
#[test]
fn my_surface_view() {
    let mut tty = populated();
    tty.show_x = true;                      // …set the exact state to capture…
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));
    let snap = sim.snapshot(&crate::state::theme(&tty)).expect("render snapshot");
    let matches = snap.matches_image("snapshots/tty-my-surface.png").expect("write/compare");
    assert!(matches, "snapshot `tty-my-surface` changed — delete its PNG to re-baseline");
}
```

**Re-baseline** (after an intentional visual change):
- The real file has a **backend suffix**: `snapshots/tty-my-surface-**wgpu**.png` (not the bare
  name in `matches_image`). `ls tty/snapshots/ | grep <name>` to find it.
- Delete it and re-run — a missing baseline is **written and passes**; re-run once more to
  confirm it compares clean.

**Gotchas**
- ⚠️ **Fixed clock.** A fixture seeded from `Utc::now()` / `SystemTime::now()` bakes wall time
  into a pixel-exact image → it flakes and **flips across midnight**. Seed a fixed anchor and
  pin the view's clock (`Tty::clock_override`, threaded into `now_ms()` / `age_from_epoch_ms`).
- New `Tty` field → update the `snapshot.rs` literal too (the 3-init trap).
- Snapshots are excluded from the default run; `--ignore-default-filter` includes them.

## Verify (from inside `tty/`)

```sh
cargo fmt -p tty
cargo clippy -p tty --all-targets -- -D warnings                    # must be clean
cargo nextest run -p tty --ignore-default-filter --no-fail-fast     # whole suite incl. snapshots
```
Iterate on one test: `cargo nextest run -p tty --ignore-default-filter -E 'test(my_surface_view)'`.
rime work: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test` in
`../rime`, then eyeball `cargo run -p rime-demo`.

**No inline test modules**: `foo.rs` keeps its tests in a sibling `foo_tests.rs`, wired
`#[cfg(test)] #[path = "foo_tests.rs"] mod tests;`.

## Checklist

- [ ] State fields added + doc-commented; initialized in **all three** literals
- [ ] `Message` variant(s) + `update.rs` handler(s)
- [ ] Rendered in `view/*.rs`; reusable chrome came from rime, not inlined
- [ ] Component reference followed (popover / cell / settings / rime primitive)
- [ ] Snapshot(s) added/updated; `-wgpu` PNGs re-baselined; no `now()` in fixtures
- [ ] `fmt` clean, `clippy -D warnings` clean, full suite green
- [ ] Changelog/docs updated where the repo expects it (rime CHANGELOG; tty README/ARCHITECTURE for user-facing or architectural changes)

---
name: ui
description: Orientation + shared conventions for UI-surface work in tty and rime. Carries the wiring loop every UI change follows (Message→update→view→state, the three struct-literal init sites), the tty/rime boundary, snapshot-test authoring + re-baselining, and the verify commands. Start here when unsure which UI workflow applies or for the shared conventions; the focused workflow skills — ui-popover, ui-settings, ui-status-cell, rime-widget — trigger on their own and build on this.
---

# UI in tty + rime — shared core

tty is one iced 0.14 Elm loop; `rime` (`../rime/rime`, a path dep) is the shared, stateless
component kit. **Reusable chrome lives in rime; app-specific surfaces live in tty.** The UI
is a set of *separate components* that all thread through the **same wiring loop** below.

This skill is the shared foundation. Each specific workflow is its own skill (see the table)
and recaps just enough of the loop to stand alone — but this is where the full loop, snapshot
mechanics, and verify commands live. Also read `tty/CLAUDE.md` (invariants) and
`../rime/ICED.md` (iced 0.14 patterns).

## Which workflow skill

| Building… | Skill |
|-----------|-------|
| a popover / floating panel (drag + resize, compact↔expanded, a sub-form modal) | **`ui-popover`** |
| a status-bar cell (sampled metric, or a launcher like `Env`) | **`ui-status-cell`** |
| a settings toggle / section (TOML round-trip, gating, migration) | **`ui-settings`** |
| a reusable rime primitive (the contract, demo, tokens) | **`rime-widget`** |
| editing an existing surface | find its `view/*.rs`, change the render, re-baseline its snapshots (below). Branch in the view over adding state when you can. |

Invoke the matching skill; each carries the component-specific detail. This file has the rest.

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
   `Tty::new` (`state.rs`), `behavior.rs`, and `populated()` (`snapshot.rs`).

> ⚠️ **The 3-init trap.** Miss a site and you get `error[E0063]: missing field ...`. Add the
> field to all three in one pass. `grep -rn "<neighbor_field>:" tty/src/{state.rs,behavior.rs,snapshot.rs}` finds them.

## The tty ↔ rime boundary

- **Reusable primitive** (a button style, a card, floating-panel chrome) → **extend rime**
  (the `rime-widget` skill), then depend on it. Never inline or hand-roll a styled widget in tty.
- **App-specific surface** → tty, *composing* rime primitives.
- rime primitives are **stateless**, read colors from `theme::tokens()`; tty owns the state.

Chrome already in rime: `button::{primary,secondary,ghost,ghost_compact,danger,icon}`,
`text_field`, `toggle`, `section`, `caption`, `stat`, `select`, `stepper`,
`modal`/`modal_sized`, `dialog`, `context_menu`, `table`, and
**`popover`/`resize_edges`/`ResizeEdge`** (the draggable-resizable floating card).

## Snapshot tests

Any surface gets a `snapshot::*` test (render chrome → PNG, pixel-compare). The full workflow —
authoring, the `-wgpu` re-baseline flow, the nextest setup, and the fixed-clock flake — is the
**`snapshot-testing`** skill; invoke it when you add or re-baseline one. Two things it hammers that
bite here: re-baseline by deleting the **`-wgpu`**-suffixed PNG (not the bare name) and re-running,
and **never seed a fixture from `now()`** (it flips across midnight — use `Tty::clock_override`).

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

## Checklist (any UI change)

- [ ] State fields added + doc-commented; initialized in **all three** literals
- [ ] `Message` variant(s) + `update.rs` handler(s)
- [ ] Rendered in `view/*.rs`; reusable chrome came from rime, not inlined
- [ ] The component workflow skill followed (popover / settings / status-cell / rime-widget)
- [ ] Snapshot(s) added/updated; `-wgpu` PNGs re-baselined; no `now()` in fixtures
- [ ] `fmt` clean, `clippy -D warnings` clean, full suite green
- [ ] Changelog/docs updated where the repo expects it (rime CHANGELOG; tty README/ARCHITECTURE for user-facing or architectural changes)

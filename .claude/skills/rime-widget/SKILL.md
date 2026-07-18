---
name: rime-widget
description: Use when authoring or changing a reusable widget/primitive in the rime component kit (`../rime/rime`) — a stateless, theme-token-driven iced 0.14 component shared by tty and fed (a button, card, floating-panel chrome, etc.). Covers rime's COMPONENTS.md contract, registration, the demo/theme-toggle check, and the key gotchas. Apply when tty needs reusable chrome rather than a one-off app surface.
---

# Authoring a rime primitive

When tty (or fed) needs **reusable chrome**, it goes in rime (`../rime/rime`), never inlined in
the app. rime already documents the full contract — **read `../rime/rime/src/widgets/COMPONENTS.md`**
before adding one. This is the short version + the traps.

(Deciding whether it belongs in rime vs tty? A reusable, domain-free visual primitive → rime.
An app-specific surface composed from primitives → tty, and one of the `ui-*` skills.)

## The rules that matter

1. **One primitive per file**, `src/widgets/<x>.rs`. Stateless, generic over the message type
   `M`, returns `Element<'a, M>` (or a concrete builder when callers chain).
2. **No hardcoded colors.** Read from `theme::tokens()`, and capture the tokens into any
   draw-time `move` closure so styling doesn't depend on *when* iced calls back (see `card.rs`).
3. **No app types, no I/O, no state.** The nine `Palette` tokens are the portability contract;
   don't add a token for one app's need — that color is the app's domain concern.
4. **Register it** in `src/widgets/mod.rs`: `mod <x>;` **and** `pub use <x>::{...};`.
5. **`mouse_area(..).on_press(msg)` requires `M: Clone`** — bound your fn `where M: Clone`
   (this is why `popover`/`resize_edges` carry the bound).
6. **Content taller than the window needs a `scrollable`** — a bare `Length::Shrink` column
   silently stops rendering past a height (see rime's `ICED.md`).

## Ship it

- Add it to the **demo** (`demo/`) so the one-screen visual + theme-toggle check covers it — it
  must re-color correctly when the theme flips (proof no color leaked). `cargo run -p rime-demo`.
- Add it to the **README** widget catalog and to `CHANGELOG.md` (Unreleased).
- Verify: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test` in `../rime`.

## The stateless-primitive-for-a-stateful-surface pattern

A primitive holds no state, but a tty surface is stateful — the primitive takes the state as
inputs and hands back messages. `popover` is the reference: the **caller** owns the position and
size and supplies the drag messages; the primitive only builds the draggable/resizable/opaque
view. That's how one stateless primitive backs the Env view *and* the metric drill-ins.

Then depend on it from tty and compose — the app side is the `ui-popover` (or other `ui-*`) skill.

---
name: ui-popover
description: Use when building or changing a floating popover / drill-in panel in tty — a draggable, border-resizable card over the terminal (like the Env view or a metric drill-in), including compact↔expanded modes and an add/edit sub-form modal. Covers the rime `popover` primitive, the move/resize wiring, content-driven sizing, and the modal. Apply whenever you add or edit a `place_*_popover` in a `view/*.rs`.
---

# A popover / floating panel

A floating, draggable, resizable card over the terminal. Use rime's `popover` primitive — do
**not** re-hand-roll drag/resize.

**Shared loop** (full detail in the `ui` skill): `Message`→`update`→`view`→`state`, and a new
`Tty` field must be initialized in all three struct-literal sites (`state.rs` `Tty::new`,
`behavior.rs`, `snapshot.rs` `populated()`). This page is the popover-specific detail.

## State (`state/types.rs`)

Mirror the `env_*` / `metric_detail_*` fields:
- `<x>_pos: Option<(f32, f32)>` — top-left, `None` = centered until dragged.
- `<x>_size: (f32, f32)` — current size (drag-resize writes it).
- `<x>_move_drag: Option<(Point, (f32,f32))>` — `(pointer, pos)` at grab.
- `<x>_resize: Option<(Point, (f32,f32), ResizeEdge)>` — `(pointer, size, edge)` at grab.
- content state (`<x>_expanded: bool`, filter strings, an `<x>_add_open: bool` for a modal).

## View (`view/<surface>.rs`)

```rust
pub(super) fn place_<x>_popover<'a>(state: &'a Tty, base: Element<'a, Message>) -> Element<'a, Message> {
    let (x, y) = state.<x>_effective_pos();     // <x>_pos, else centered from the size
    let (w, h) = state.<x>_view_size();          // see "Sizing"
    let floating = rime::widgets::popover(       // whole card = drag handle; opaque; resize edges
        card(state, w, h),
        Message::<X>MoveStart,
        Message::<X>ResizeStart,                 // a fn(ResizeEdge) -> Message
    );
    let placed = container(floating)
        .width(Length::Fill).height(Length::Fill)
        .align_x(Left).align_y(Top)
        .padding(Padding::ZERO.left(x).top(y));
    stack![base, placed].into()
}
```

Stack it in `view.rs` when open:
```rust
if state.show_<x> {
    base = <surface>::place_<x>_popover(state, base);
    if state.<x>_add_open { base = <surface>::place_<x>_add_modal(state, base); }
}
```

`rime::widgets::popover(card, on_move, on_resize)` already gives you: the whole card as a drag
handle, `opaque` (so a press doesn't leak to the terminal behind), and the border resize strips.
Inner controls/rows/fields still win the hit test because a child captures the press first.
Don't wrap your own `mouse_area`/`opaque` — that's what made the Env view only drag by its
title before it adopted the primitive.

## Update (`update.rs`)

Copy the `env_*` / `metric_detail_*` arms:
- `<X>MoveStart` → `state.<x>_move_drag = Some((state.pointer, state.<x>_effective_pos()))`.
- `<X>ResizeStart(edge)` → `state.<x>_resize = Some((state.pointer, state.<x>_size, edge))`.
- In `PointerMoved`: if a drag is active, update `<x>_pos` / `<x>_size` from the delta, clamped
  to the window. `ResizeEdge::axes()` says which dims a corner/edge changes.
- In `PointerReleased`: clear both drag fields.

## Sizing (compact ↔ expanded)

Give it a `Tty::<x>_view_size()` method. To open compact and expand on demand (like the metric
drill-ins):
- `<x>_expanded: bool` + a `Toggle<X>Expanded` message that flips it and snaps `<x>_size`
  between a compact and an expanded default.
- Controls: reuse the `+`/`−` glyph cluster — `button::ghost_compact(if expanded {"−"} else {"+"}, Message::Toggle<X>Expanded)` — same as the metric popovers, plus a `×` close.
- **Shrink compact to content, then cap + scroll.** Compute the compact height from the row
  count so a short list leaves no whitespace, capped at a max (see `Tty::env_view_size` +
  `ENV_COMPACT_MAX_HEIGHT`) so a long list scrolls instead of ballooning. Wrap the list in
  `scrollable(list).height(Fill)` so the cap actually scrolls.
- Layout: a masked/short value column reads better right-aligned (`text(name).width(Fill)` then
  the value) so it doesn't strand a middle band; real long values want a left-aligned
  `FillPortion` second column. Branch on `expanded`.

## A sub-form (add / edit an item)

Don't inline an always-on editor footer. Add a bool (`<x>_add_open`) + `Open<X>Add` /
`Close<X>Add` messages, and render a centered modal:
```rust
rime::widgets::modal_sized(base, content, Message::Close<X>Add, 380.0)
```
with `text_field`s + a `button::primary`/`ghost` action row; a successful action closes it. See
`place_env_add_modal`. (A full settings *section* is different — that's the `ui-settings` skill.)

## Snapshots + verify

Add compact, expanded, and (if any) modal snapshots — each a `#[test]` setting
`show_<x>`/`<x>_expanded`/`<x>_add_open`. Snapshot authoring, the `-wgpu` re-baseline flow, and
the verify commands are in the `ui` skill. Reference example end-to-end: `view/env.rs` + the
`env_view_*` tests.

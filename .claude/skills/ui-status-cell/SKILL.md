---
name: ui-status-cell
description: Use when adding or changing a tty status-bar cell — a sampled metric (sparkline/number with a drill-in) or a launcher cell like Env/Clock (a static label that runs an action on click). Covers the `MetricKind` variant, the `ALL` + string round-trip, `metric_render`, and click routing. Apply whenever you touch `MetricKind` in `settings.rs` or `view/metrics.rs`.
---

# A status-bar cell

Cells are `MetricKind` variants (in `settings.rs`), rendered by `view/metrics.rs`. Two kinds:
- **Sampled** (CPU, memory, net) — a sampler feeds a history; the cell draws a sparkline/number
  and a click opens a drill-in popover.
- **Launcher** (`MetricKind::Env`, `Clock`) — no sampler; a static label, and a click runs an
  action instead of a drill-in.

**Shared loop** (full detail in the `ui` skill): `Message`→`update`→`view`→`state`, init a new
`Tty` field in all three struct-literal sites. This page is the cell-specific detail.

## 1. Add the variant (`settings.rs`)

- Add the `MetricKind` variant.
- Update **`ALL`** (bump the array length, e.g. `[MetricKind; 17]` → `18`) — round-tripping and
  the settings "add cell" list iterate `ALL`.
- Update `as_setting_str`, `from_setting_str`, and the `Display` impl. The round-trip test covers
  the string mapping via `ALL`; keep the three in sync.

## 2. Render (`view/metrics.rs`)

In `metric_render`:
- **Sampled**: produce `MetricRender { label, series, max, alert }` from the sampled stats.
- **Launcher**: short-circuit with a static label (like `Clock`/`Env`) *before* the stats read,
  and add a `K::<X> => unreachable!("… handled before the stats read")` arm in the following
  match so it stays exhaustive.

## 3. Click behavior (`update.rs`)

In the `PointerReleased` handler where a cell key is resolved:
- **Sampled** → `state.open_metric_detail(&key)` (opens the drill-in — that popover is the
  `ui-popover` skill's territory).
- **Launcher** → your action, special-cased by key, e.g. `if key == "env" { state.toggle_env_view() } else { … }`.

## Notes + snapshots

- A launcher cell is added/reordered/removed in **Settings → Metrics** like any cell (free, since
  it's a `MetricKind`), and persists as `metric = "<x>"`.
- Thresholds/alerts (`warn`/`alarm` grading) apply only to bounded sampled kinds.
- Snapshot: add a cell-in-the-bar test, and re-baseline the metrics-editor snapshots (the "add
  cell" list gains your entry). Snapshot mechanics + verify are in the `ui` skill.

Reference example: `MetricKind::Env` — the launcher cell that opens the Env popover.

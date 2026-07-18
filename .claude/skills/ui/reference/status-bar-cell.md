# Reference: a status-bar cell

Cells are `MetricKind` variants (in `settings.rs`), rendered by `view/metrics.rs`. Two kinds:

- **Sampled** (CPU, memory, net) — a sampler feeds a history; the cell draws a sparkline/number
  and a click opens a drill-in popover.
- **Launcher** (like `MetricKind::Env`, `Clock`) — no sampler; renders a static label and a
  click runs an action instead of a drill-in.

Follow the wiring loop in `SKILL.md`. Component specifics:

## 1. Add the variant (`settings.rs`)

- Add the `MetricKind` variant.
- Update **`ALL`** (bump the array length, e.g. `[MetricKind; 17]` → `18`) — round-tripping
  and the settings "add cell" list iterate `ALL`.
- Update `as_setting_str`, `from_setting_str`, and the `Display` impl. The round-trip test
  covers the string mapping via `ALL`, so keep the three in sync.

## 2. Render (`view/metrics.rs`)

In `metric_render`:
- **Sampled**: produce the `MetricRender { label, series, max, alert }` from the sampled stats.
- **Launcher**: short-circuit with a static label (like `Clock`/`Env` do) *before* the stats
  read, and add a `K::<X> => unreachable!("… handled before the stats read")` arm in the match
  that follows, so the compiler stays exhaustive.

## 3. Click behavior (`update.rs`)

In the `PointerReleased` handler where a cell key is resolved:
- **Sampled** → `state.open_metric_detail(&key)` (opens the drill-in).
- **Launcher** → your action, special-cased by key, e.g. `if key == "env" { state.toggle_env_view() } else { … }`.

## Notes

- A launcher cell is added/reordered/removed in **Settings → Metrics** like any cell (free,
  since it's a `MetricKind`), and persists as `metric = "<x>"`.
- Thresholds/alerts (`warn`/`alarm` grading) apply only to bounded sampled kinds; launchers
  and rate kinds don't grade.
- Snapshot: add a cell-in-the-bar test, and re-baseline the metrics-editor snapshots (the
  "add cell" list gains your new entry). See `SKILL.md` snapshot section.

Reference example: `MetricKind::Env` — the launcher cell that opens the Env popover.
